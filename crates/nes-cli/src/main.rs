//! Headless NES runner.
//!
//! ```text
//! nes-cli info game.nes
//! nes-cli run game.nes --frames 600 --screenshot out.png --trace trace.log
//! nes-cli test test-data/nes-test-roms/instr_test-v5/rom_singles/*.nes
//! nes-cli nestest nestest.nes nestest.log
//! nes-cli bench game.nes --frames 3000
//! ```

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use nes_core::testing::{self, Outcome};
use nes_core::{Buttons, Config, Nes, Palette, Region, StopReason};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(name = "nes-cli", version, about = "Headless NES emulator tools")]
struct Cli {
    /// Force a console region instead of using the ROM header.
    #[arg(long, global = true, value_parser = parse_region)]
    region: Option<Region>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print header information.
    Info { rom: PathBuf },
    /// Run a ROM for a number of frames.
    Run {
        rom: PathBuf,
        #[arg(long, default_value_t = 600)]
        frames: u32,
        /// Write the final frame as a PNG.
        #[arg(long)]
        screenshot: Option<PathBuf>,
        /// Write a nestest-style instruction trace.
        #[arg(long)]
        trace: Option<PathBuf>,
        /// Hold buttons on given frames, e.g. `120-130:start` or `200:a+right`.
        #[arg(long = "press")]
        presses: Vec<String>,
    },
    /// Run test ROMs that report through $6000 and print pass/fail.
    Test {
        roms: Vec<PathBuf>,
        #[arg(long, default_value_t = 3600)]
        max_frames: u32,
        /// Save the final frame of each test into this directory.
        #[arg(long)]
        screenshots: Option<PathBuf>,
        /// Write the final frames of all tests as one contact sheet PNG
        /// (useful for older tests that only report on screen).
        #[arg(long)]
        sheet: Option<PathBuf>,
        /// For ROMs without the $6000 protocol, keep running this many more
        /// frames before reading the screen.
        #[arg(long, default_value_t = 0)]
        settle: u32,
    },
    /// Compare execution of nestest.nes (automation mode) with a golden log.
    Nestest {
        rom: PathBuf,
        log: PathBuf,
        /// Also compare the PPU dot/scanline columns.
        #[arg(long)]
        ppu: bool,
    },
    /// Run the whole accuracy suite and write a Markdown report.
    CompatReport {
        /// Directory containing the nes-test-roms collection.
        #[arg(long, default_value = "test-data/nes-test-roms")]
        roms: PathBuf,
        #[arg(long, default_value = "docs/compat.md")]
        out: PathBuf,
    },
    /// Measure emulation speed.
    Bench {
        rom: PathBuf,
        #[arg(long, default_value_t = 3000)]
        frames: u32,
    },
}

fn parse_region(s: &str) -> Result<Region, String> {
    match s.to_ascii_lowercase().as_str() {
        "ntsc" => Ok(Region::Ntsc),
        "pal" => Ok(Region::Pal),
        "dendy" => Ok(Region::Dendy),
        _ => Err("expected ntsc, pal or dendy".into()),
    }
}

fn load(path: &Path, region: Option<Region>) -> Result<Nes> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Nes::new(
        &data,
        Config {
            region,
            ..Config::default()
        },
    )
    .with_context(|| format!("loading {}", path.display()))
}

fn write_png(nes: &Nes, path: &Path) -> Result<()> {
    let palette = if nes.region() == Region::Pal {
        Palette::pal()
    } else {
        Palette::ntsc()
    };
    let mut rgba = Vec::new();
    palette.convert(nes.framebuffer(), &mut rgba);
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), 256, 240);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&rgba)?;
    Ok(())
}

/// Tiles 256x240 RGBA frames four to a row.
fn write_sheet(frames: &[Vec<u8>], path: &Path) -> Result<()> {
    const COLS: usize = 4;
    let rows = frames.len().div_ceil(COLS).max(1);
    let (w, h) = (256 * COLS, 240 * rows);
    let mut out = vec![0u8; w * h * 4];
    for (i, f) in frames.iter().enumerate() {
        let (cx, cy) = ((i % COLS) * 256, (i / COLS) * 240);
        for y in 0..240 {
            let src = &f[y * 256 * 4..(y + 1) * 256 * 4];
            let dst = ((cy + y) * w + cx) * 4;
            out[dst..dst + 256 * 4].copy_from_slice(src);
        }
    }
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&out)?;
    Ok(())
}

/// `120-130:start`, `200:a+right`.
fn parse_press(s: &str) -> Result<(u32, u32, Buttons)> {
    let (frames, buttons) = s.split_once(':').context("expected FRAMES:BUTTONS")?;
    let (start, end) = match frames.split_once('-') {
        Some((a, b)) => (a.parse()?, b.parse()?),
        None => {
            let f = frames.parse()?;
            (f, f + 5)
        }
    };
    let mut b = Buttons::default();
    for name in buttons.split('+') {
        let bit = Buttons::NAMES
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, bit)| *bit)
            .with_context(|| format!("unknown button {name}"))?;
        b.set(bit, true);
    }
    Ok((start, end, b))
}

fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    match cli.command {
        Command::Info { rom } => {
            let nes = load(&rom, cli.region)?;
            let i = nes.rom_info();
            println!("format:     {:?}", i.format);
            println!(
                "mapper:     {} (submapper {}) - {}",
                i.mapper,
                i.submapper,
                mapper_name(&nes)
            );
            println!("PRG ROM:    {} KiB", i.prg_rom_size / 1024);
            println!("CHR ROM:    {} KiB", i.chr_rom_size / 1024);
            println!(
                "PRG RAM:    {} KiB (+{} KiB battery)",
                i.prg_ram_size / 1024,
                i.prg_nvram_size / 1024
            );
            println!(
                "CHR RAM:    {} KiB",
                (i.chr_ram_size + i.chr_nvram_size) / 1024
            );
            println!("mirroring:  {}", i.mirroring.name());
            println!("battery:    {}", i.battery);
            println!("region:     {}", nes.region().name());
            println!("CRC32:      {:08X} (PRG {:08X})", i.crc32, i.prg_crc32);
        }
        Command::Run {
            rom,
            frames,
            screenshot,
            trace,
            presses,
        } => {
            let mut nes = load(&rom, cli.region)?;
            let presses = presses
                .iter()
                .map(|p| parse_press(p))
                .collect::<Result<Vec<_>>>()?;
            let mut trace_out = match &trace {
                Some(p) => {
                    nes.bus.dbg.trace_enabled = true;
                    Some(std::io::BufWriter::new(std::fs::File::create(p)?))
                }
                None => None,
            };
            let mut seq = 0;
            // FNV-1a over the audio output, to check changes are bit-exact.
            let mut audio_hash: u64 = 0xcbf2_9ce4_8422_2325;
            let mut samples = Vec::new();
            for f in 0..frames {
                let mut b = Buttons::default();
                for (start, end, pb) in &presses {
                    if (*start..=*end).contains(&f) {
                        b.0 |= pb.0;
                    }
                }
                nes.set_buttons(0, b);
                loop {
                    match nes.run_frame() {
                        StopReason::FrameComplete => break,
                        StopReason::Break(r) => eprintln!("break: {r}"),
                        StopReason::StepDone => {}
                    }
                }
                nes.drain_audio(&mut samples);
                for s in samples.drain(..) {
                    for byte in s.to_bits().to_le_bytes() {
                        audio_hash = (audio_hash ^ byte as u64).wrapping_mul(0x0100_0000_01b3);
                    }
                }
                if let Some(out) = trace_out.as_mut() {
                    for e in nes.bus.dbg.trace.since(seq) {
                        writeln!(out, "{}", e.nestest_line())?;
                    }
                    seq = nes.bus.dbg.trace.total();
                }
            }
            println!(
                "frame hash {:016x}, audio hash {audio_hash:016x}",
                nes_core::testing::frame_hash(&nes)
            );
            if let Some(p) = screenshot {
                write_png(&nes, &p)?;
                println!("wrote {}", p.display());
            }
        }
        Command::Test {
            roms,
            max_frames,
            screenshots,
            sheet,
            settle,
        } => {
            let mut failed = 0;
            let mut frames: Vec<Vec<u8>> = Vec::new();
            for rom in &roms {
                let mut nes = match load(rom, cli.region) {
                    Ok(n) => n,
                    Err(e) => {
                        println!("ERROR {}: {e:#}", rom.display());
                        failed += 1;
                        continue;
                    }
                };
                let outcome = testing::run_blargg(&mut nes, max_frames);
                if outcome == Outcome::NoProtocol {
                    for _ in 0..settle {
                        while nes.run_frame() != StopReason::FrameComplete {}
                    }
                }
                let name = rom.display();
                match &outcome {
                    Outcome::Passed(_) => println!("PASS  {name}"),
                    Outcome::Failed { code, message } => {
                        println!(
                            "FAIL  {name} (code {code}): {}",
                            message.replace('\n', " | ")
                        )
                    }
                    Outcome::Timeout(m) => println!("TIME  {name}: {}", m.replace('\n', " | ")),
                    Outcome::NoProtocol => {
                        let text = testing::screen_text(&nes).replace('\n', " | ");
                        println!("????  {name}: no $6000 result; screen: {text}")
                    }
                }
                if !outcome.passed() {
                    failed += 1;
                }
                if let Some(dir) = &screenshots {
                    std::fs::create_dir_all(dir)?;
                    let stem = rom.file_stem().unwrap_or_default().to_string_lossy();
                    write_png(&nes, &dir.join(format!("{stem}.png")))?;
                }
                if sheet.is_some() {
                    let mut rgba = Vec::new();
                    Palette::ntsc().convert(nes.framebuffer(), &mut rgba);
                    frames.push(rgba);
                }
            }
            println!("{} passed, {failed} not passed", roms.len() - failed);
            if let Some(path) = sheet {
                write_sheet(&frames, &path)?;
                println!("wrote {}", path.display());
            }
        }
        Command::Nestest { rom, log, ppu } => nestest(&rom, &log, ppu, cli.region)?,
        Command::CompatReport { roms, out } => compat_report(&roms, &out)?,
        Command::Bench { rom, frames } => {
            let mut nes = load(&rom, cli.region)?;
            let start = Instant::now();
            let mut chunk = Instant::now();
            let mut sink = Vec::new();
            for f in 1..=frames {
                while nes.run_frame() != StopReason::FrameComplete {}
                nes.drain_audio(&mut sink);
                sink.clear();
                if f % 1000 == 0 {
                    log::info!(
                        "frames {}-{f}: {:.0} fps",
                        f - 999,
                        1000.0 / chunk.elapsed().as_secs_f64()
                    );
                    chunk = Instant::now();
                }
            }
            let secs = start.elapsed().as_secs_f64();
            let fps = frames as f64 / secs;
            println!(
                "{frames} frames in {secs:.2}s: {fps:.0} fps ({:.1}x real time)",
                fps / nes.region().frame_rate()
            );
        }
    }
    Ok(())
}

fn compat_report(rom_dir: &Path, out: &Path) -> Result<()> {
    use nes_core::testing::suite::{EntryResult, SUITE, run_entry};
    use std::fmt::Write as _;

    let mut groups: Vec<(&str, Vec<(&str, EntryResult)>)> = Vec::new();
    for e in SUITE {
        let result = run_entry(rom_dir, e);
        let mark = match &result {
            EntryResult::Passed => "pass",
            EntryResult::Failed(_) => "FAIL",
            EntryResult::Missing => "missing",
        };
        println!("{mark:7} {}", e.path);
        match groups.iter_mut().find(|(g, _)| *g == e.group) {
            Some((_, v)) => v.push((e.path, result)),
            None => groups.push((e.group, vec![(e.path, result)])),
        }
    }
    if groups
        .iter()
        .flat_map(|(_, v)| v)
        .all(|(_, r)| *r == EntryResult::Missing)
    {
        bail!(
            "no test ROMs found under {}; run scripts/fetch-test-data first",
            rom_dir.display()
        );
    }

    let mut md = String::new();
    writeln!(md, "# Compatibility report\n")?;
    writeln!(
        md,
        "Generated by `nes-cli compat-report` from the [nes-test-roms](https://github.com/christopherpow/nes-test-roms) collection.\n"
    )?;
    let (mut total, mut passed) = (0, 0);
    writeln!(md, "| Area | Passed |\n|---|---|")?;
    for (group, results) in &groups {
        let present: Vec<_> = results
            .iter()
            .filter(|(_, r)| *r != EntryResult::Missing)
            .collect();
        let ok = present
            .iter()
            .filter(|(_, r)| *r == EntryResult::Passed)
            .count();
        total += present.len();
        passed += ok;
        writeln!(md, "| {group} | {ok} / {} |", present.len())?;
    }
    writeln!(md, "| **Total** | **{passed} / {total}** |\n")?;
    writeln!(
        md,
        "Plus nestest (all 8,991 trace lines) and a randomized save-state / register test for every mapper, run by `cargo test`.\n"
    )?;

    for (group, results) in &groups {
        writeln!(md, "## {group}\n\n| Test ROM | Result |\n|---|---|")?;
        for (path, r) in results {
            let cell = match r {
                EntryResult::Passed => "pass".to_string(),
                EntryResult::Failed(why) => format!("**fail**: {}", why.replace('|', "/")),
                EntryResult::Missing => "not downloaded".to_string(),
            };
            writeln!(md, "| `{path}` | {cell} |")?;
        }
        writeln!(md)?;
    }

    writeln!(md, "## Supported mappers\n")?;
    let list: Vec<String> = nes_core::cart::SUPPORTED_MAPPERS
        .iter()
        .map(u16::to_string)
        .collect();
    writeln!(
        md,
        "{} iNES mapper numbers: {}.\n",
        list.len(),
        list.join(", ")
    )?;
    writeln!(md, "## Known gaps\n")?;
    for gap in [
        "MMC3 revision A IRQ behavior is only used when the NES 2.0 header asks for it (submapper 4); `mmc3_test_2/6-MMC3_alt` and `mmc3_irq_tests/5.MMC3_rev_A` target that revision.",
        "MMC5 vertical split-screen mode is not emulated (used by one game).",
        "Famicom Disk System, VS. System and PlayChoice-10 hardware are not emulated.",
        "Jaleco SS88006's uPD7756 speech chip is not emulated.",
        "`other/test28.nes` (an interactive test for a 2012 revision of the Action 53 board) disagrees with the current board specification; Action 53 compilations themselves run.",
    ] {
        writeln!(md, "- {gap}")?;
    }

    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out, md)?;
    println!("{passed}/{total} passed; wrote {}", out.display());
    Ok(())
}

fn mapper_name(nes: &Nes) -> &'static str {
    use nes_core::cart::Mapper;
    nes.bus.cart.name()
}

fn nestest(rom: &Path, log: &Path, compare_ppu: bool, region: Option<Region>) -> Result<()> {
    let mut nes = load(rom, region)?;
    let golden = std::fs::read_to_string(log)?;
    // Automation mode starts at $C000 instead of the reset vector.
    nes.cpu.regs.pc = 0xC000;
    nes.bus.dbg.trace_enabled = true;
    for (i, expected) in golden.lines().enumerate() {
        nes.step_instruction();
        let entry = nes
            .bus
            .dbg
            .trace
            .get(nes.bus.dbg.trace.len() - 1)
            .expect("traced");
        let actual = entry.nestest_line();
        let cut = |s: &str| -> String {
            if compare_ppu {
                s.to_string()
            } else {
                // Drop the PPU column but keep CYC.
                match (s.find("PPU:"), s.find("CYC:")) {
                    (Some(p), Some(c)) => format!("{}{}", &s[..p], &s[c..]),
                    _ => s.to_string(),
                }
            }
        };
        // Values shown for APU/IO registers depend on the logging emulator's
        // open-bus model, not on CPU behavior; don't compare them.
        let io_operand = expected.get(16..48).is_some_and(|op| op.contains(" $40"));
        let same = if io_operand {
            let strip =
                |s: &str| format!("{}{}", s.get(..16).unwrap_or(""), s.get(48..).unwrap_or(""));
            cut(&strip(expected.trim_end())) == cut(&strip(&actual))
        } else {
            cut(expected.trim_end()) == cut(&actual)
        };
        if !same {
            println!(
                "mismatch at line {}:\n  expected: {expected}\n  actual:   {actual}",
                i + 1
            );
            bail!("nestest diverged");
        }
    }
    println!(
        "nestest: all {} lines match; result bytes $02={:02X} $03={:02X}",
        golden.lines().count(),
        nes.peek(2),
        nes.peek(3)
    );
    Ok(())
}
