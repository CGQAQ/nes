//! The accuracy suite: which third-party test ROMs the emulator is expected
//! to pass, and how each reports its result. Shared by `cargo test` and
//! `nes-cli compat-report`.

use super::{Outcome, run_blargg, screen_text};
use crate::{Config, Nes, Region, StopReason};
use std::path::Path;

#[derive(Clone, Copy, Debug)]
pub enum Check {
    /// Reports through `$6000`.
    Protocol,
    /// Prints its result; any of the `|`-separated strings means pass
    /// (some outputs legitimately depend on CPU/PPU power-up alignment).
    Screen { frames: u32, expect: &'static str },
}

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub group: &'static str,
    /// Relative to the nes-test-roms directory.
    pub path: &'static str,
    pub check: Check,
    pub region: Option<Region>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryResult {
    Passed,
    Failed(String),
    /// The ROM isn't in the test-data directory.
    Missing,
}

const fn p(group: &'static str, path: &'static str) -> Entry {
    Entry {
        group,
        path,
        check: Check::Protocol,
        region: None,
    }
}

const fn s(group: &'static str, path: &'static str, frames: u32, expect: &'static str) -> Entry {
    Entry {
        group,
        path,
        check: Check::Screen { frames, expect },
        region: None,
    }
}

const fn pal(path: &'static str) -> Entry {
    Entry {
        group: "APU (PAL)",
        path,
        check: Check::Screen {
            frames: 700,
            expect: "PASSED",
        },
        region: Some(Region::Pal),
    }
}

pub static SUITE: &[Entry] = &[
    // CPU
    p("CPU", "instr_test-v5/rom_singles/01-basics.nes"),
    p("CPU", "instr_test-v5/rom_singles/02-implied.nes"),
    p("CPU", "instr_test-v5/rom_singles/03-immediate.nes"),
    p("CPU", "instr_test-v5/rom_singles/04-zero_page.nes"),
    p("CPU", "instr_test-v5/rom_singles/05-zp_xy.nes"),
    p("CPU", "instr_test-v5/rom_singles/06-absolute.nes"),
    p("CPU", "instr_test-v5/rom_singles/07-abs_xy.nes"),
    p("CPU", "instr_test-v5/rom_singles/08-ind_x.nes"),
    p("CPU", "instr_test-v5/rom_singles/09-ind_y.nes"),
    p("CPU", "instr_test-v5/rom_singles/10-branches.nes"),
    p("CPU", "instr_test-v5/rom_singles/11-stack.nes"),
    p("CPU", "instr_test-v5/rom_singles/12-jmp_jsr.nes"),
    p("CPU", "instr_test-v5/rom_singles/13-rts.nes"),
    p("CPU", "instr_test-v5/rom_singles/14-rti.nes"),
    p("CPU", "instr_test-v5/rom_singles/15-brk.nes"),
    p("CPU", "instr_test-v5/rom_singles/16-special.nes"),
    p("CPU", "instr_misc/rom_singles/01-abs_x_wrap.nes"),
    p("CPU", "instr_misc/rom_singles/02-branch_wrap.nes"),
    p("CPU", "instr_misc/rom_singles/03-dummy_reads.nes"),
    p("CPU", "instr_misc/rom_singles/04-dummy_reads_apu.nes"),
    p("CPU", "instr_timing/rom_singles/1-instr_timing.nes"),
    p("CPU", "instr_timing/rom_singles/2-branch_timing.nes"),
    p("CPU", "cpu_interrupts_v2/rom_singles/1-cli_latency.nes"),
    p("CPU", "cpu_interrupts_v2/rom_singles/2-nmi_and_brk.nes"),
    p("CPU", "cpu_interrupts_v2/rom_singles/3-nmi_and_irq.nes"),
    p("CPU", "cpu_interrupts_v2/rom_singles/4-irq_and_dma.nes"),
    p(
        "CPU",
        "cpu_interrupts_v2/rom_singles/5-branch_delays_irq.nes",
    ),
    p("CPU", "cpu_dummy_writes/cpu_dummy_writes_oam.nes"),
    p("CPU", "cpu_dummy_writes/cpu_dummy_writes_ppumem.nes"),
    p("CPU", "cpu_exec_space/test_cpu_exec_space_apu.nes"),
    p("CPU", "cpu_exec_space/test_cpu_exec_space_ppuio.nes"),
    p("CPU", "cpu_reset/ram_after_reset.nes"),
    p("CPU", "cpu_reset/registers.nes"),
    s("CPU", "cpu_dummy_reads/cpu_dummy_reads.nes", 120, "Passed"),
    s(
        "CPU",
        "branch_timing_tests/1.Branch_Basics.nes",
        120,
        "PASSED",
    ),
    s(
        "CPU",
        "branch_timing_tests/2.Backward_Branch.nes",
        120,
        "PASSED",
    ),
    s(
        "CPU",
        "branch_timing_tests/3.Forward_Branch.nes",
        120,
        "PASSED",
    ),
    s(
        "CPU",
        "cpu_timing_test6/cpu_timing_test.nes",
        1300,
        "PASSED",
    ),
    s(
        "CPU",
        "blargg_nes_cpu_test5/cpu.nes",
        1500,
        "All tests complete",
    ),
    s(
        "CPU",
        "blargg_nes_cpu_test5/official.nes",
        1500,
        "All tests complete",
    ),
    // PPU
    p("PPU", "ppu_vbl_nmi/rom_singles/01-vbl_basics.nes"),
    p("PPU", "ppu_vbl_nmi/rom_singles/02-vbl_set_time.nes"),
    p("PPU", "ppu_vbl_nmi/rom_singles/03-vbl_clear_time.nes"),
    p("PPU", "ppu_vbl_nmi/rom_singles/04-nmi_control.nes"),
    p("PPU", "ppu_vbl_nmi/rom_singles/05-nmi_timing.nes"),
    p("PPU", "ppu_vbl_nmi/rom_singles/06-suppression.nes"),
    p("PPU", "ppu_vbl_nmi/rom_singles/07-nmi_on_timing.nes"),
    p("PPU", "ppu_vbl_nmi/rom_singles/08-nmi_off_timing.nes"),
    p("PPU", "ppu_vbl_nmi/rom_singles/09-even_odd_frames.nes"),
    p("PPU", "ppu_vbl_nmi/rom_singles/10-even_odd_timing.nes"),
    p("PPU", "ppu_open_bus/ppu_open_bus.nes"),
    p("PPU", "ppu_read_buffer/test_ppu_read_buffer.nes"),
    p("PPU", "oam_read/oam_read.nes"),
    p("PPU", "oam_stress/oam_stress.nes"),
    s(
        "PPU",
        "sprite_hit_tests_2005.10.05/01.basics.nes",
        120,
        "PASSED",
    ),
    s(
        "PPU",
        "sprite_hit_tests_2005.10.05/02.alignment.nes",
        120,
        "PASSED",
    ),
    s(
        "PPU",
        "sprite_hit_tests_2005.10.05/03.corners.nes",
        120,
        "PASSED",
    ),
    s(
        "PPU",
        "sprite_hit_tests_2005.10.05/04.flip.nes",
        120,
        "PASSED",
    ),
    s(
        "PPU",
        "sprite_hit_tests_2005.10.05/05.left_clip.nes",
        120,
        "PASSED",
    ),
    s(
        "PPU",
        "sprite_hit_tests_2005.10.05/06.right_edge.nes",
        120,
        "PASSED",
    ),
    s(
        "PPU",
        "sprite_hit_tests_2005.10.05/07.screen_bottom.nes",
        120,
        "PASSED",
    ),
    s(
        "PPU",
        "sprite_hit_tests_2005.10.05/08.double_height.nes",
        120,
        "PASSED",
    ),
    s(
        "PPU",
        "sprite_hit_tests_2005.10.05/09.timing_basics.nes",
        120,
        "PASSED",
    ),
    s(
        "PPU",
        "sprite_hit_tests_2005.10.05/10.timing_order.nes",
        120,
        "PASSED",
    ),
    s(
        "PPU",
        "sprite_hit_tests_2005.10.05/11.edge_timing.nes",
        120,
        "PASSED",
    ),
    s("PPU", "sprite_overflow_tests/1.Basics.nes", 120, "PASSED"),
    s("PPU", "sprite_overflow_tests/2.Details.nes", 120, "PASSED"),
    s("PPU", "sprite_overflow_tests/3.Timing.nes", 1500, "PASSED"),
    s("PPU", "sprite_overflow_tests/4.Obscure.nes", 120, "PASSED"),
    s("PPU", "sprite_overflow_tests/5.Emulator.nes", 120, "PASSED"),
    s("PPU", "vbl_nmi_timing/1.frame_basics.nes", 1500, "PASSED"),
    s("PPU", "vbl_nmi_timing/2.vbl_timing.nes", 1500, "PASSED"),
    s("PPU", "vbl_nmi_timing/3.even_odd_frames.nes", 300, "PASSED"),
    s(
        "PPU",
        "vbl_nmi_timing/4.vbl_clear_timing.nes",
        300,
        "PASSED",
    ),
    s(
        "PPU",
        "vbl_nmi_timing/5.nmi_suppression.nes",
        1500,
        "PASSED",
    ),
    s("PPU", "vbl_nmi_timing/6.nmi_disable.nes", 300, "PASSED"),
    s("PPU", "vbl_nmi_timing/7.nmi_timing.nes", 300, "PASSED"),
    s(
        "PPU",
        "blargg_ppu_tests_2005.09.15b/palette_ram.nes",
        120,
        "$01",
    ),
    s(
        "PPU",
        "blargg_ppu_tests_2005.09.15b/power_up_palette.nes",
        120,
        "$01",
    ),
    s(
        "PPU",
        "blargg_ppu_tests_2005.09.15b/sprite_ram.nes",
        120,
        "$01",
    ),
    s(
        "PPU",
        "blargg_ppu_tests_2005.09.15b/vbl_clear_time.nes",
        120,
        "$01",
    ),
    s(
        "PPU",
        "blargg_ppu_tests_2005.09.15b/vram_access.nes",
        120,
        "$01",
    ),
    // APU
    p("APU", "apu_test/rom_singles/1-len_ctr.nes"),
    p("APU", "apu_test/rom_singles/2-len_table.nes"),
    p("APU", "apu_test/rom_singles/3-irq_flag.nes"),
    p("APU", "apu_test/rom_singles/4-jitter.nes"),
    p("APU", "apu_test/rom_singles/5-len_timing.nes"),
    p("APU", "apu_test/rom_singles/6-irq_flag_timing.nes"),
    p("APU", "apu_test/rom_singles/7-dmc_basics.nes"),
    p("APU", "apu_test/rom_singles/8-dmc_rates.nes"),
    p("APU", "apu_reset/4015_cleared.nes"),
    p("APU", "apu_reset/4017_timing.nes"),
    p("APU", "apu_reset/4017_written.nes"),
    p("APU", "apu_reset/irq_flag_cleared.nes"),
    p("APU", "apu_reset/len_ctrs_enabled.nes"),
    p("APU", "apu_reset/works_immediately.nes"),
    s("APU", "blargg_apu_2005.07.30/01.len_ctr.nes", 400, "$01"),
    s("APU", "blargg_apu_2005.07.30/02.len_table.nes", 400, "$01"),
    s("APU", "blargg_apu_2005.07.30/03.irq_flag.nes", 400, "$01"),
    s(
        "APU",
        "blargg_apu_2005.07.30/04.clock_jitter.nes",
        400,
        "$01",
    ),
    s(
        "APU",
        "blargg_apu_2005.07.30/05.len_timing_mode0.nes",
        400,
        "$01",
    ),
    s(
        "APU",
        "blargg_apu_2005.07.30/06.len_timing_mode1.nes",
        400,
        "$01",
    ),
    s(
        "APU",
        "blargg_apu_2005.07.30/07.irq_flag_timing.nes",
        400,
        "$01",
    ),
    s("APU", "blargg_apu_2005.07.30/08.irq_timing.nes", 400, "$01"),
    s(
        "APU",
        "blargg_apu_2005.07.30/09.reset_timing.nes",
        400,
        "$01",
    ),
    s(
        "APU",
        "blargg_apu_2005.07.30/10.len_halt_timing.nes",
        400,
        "$01",
    ),
    s(
        "APU",
        "blargg_apu_2005.07.30/11.len_reload_timing.nes",
        400,
        "$01",
    ),
    pal("pal_apu_tests/01.len_ctr.nes"),
    pal("pal_apu_tests/02.len_table.nes"),
    pal("pal_apu_tests/03.irq_flag.nes"),
    pal("pal_apu_tests/04.clock_jitter.nes"),
    pal("pal_apu_tests/05.len_timing_mode0.nes"),
    pal("pal_apu_tests/06.len_timing_mode1.nes"),
    pal("pal_apu_tests/07.irq_flag_timing.nes"),
    pal("pal_apu_tests/08.irq_timing.nes"),
    pal("pal_apu_tests/10.len_halt_timing.nes"),
    pal("pal_apu_tests/11.len_reload_timing.nes"),
    // DMA and bus conflicts
    p("DMA", "sprdma_and_dmc_dma/sprdma_and_dmc_dma.nes"),
    p("DMA", "sprdma_and_dmc_dma/sprdma_and_dmc_dma_512.nes"),
    s(
        "DMA",
        "dmc_dma_during_read4/dma_2007_read.nes",
        120,
        "159A7A8F|5E3DF9C4",
    ),
    s(
        "DMA",
        "dmc_dma_during_read4/dma_2007_write.nes",
        120,
        "Passed",
    ),
    s(
        "DMA",
        "dmc_dma_during_read4/dma_4016_read.nes",
        120,
        "Passed",
    ),
    s(
        "DMA",
        "dmc_dma_during_read4/double_2007_read.nes",
        120,
        "85CFD627|F018C287|440EF923|E52F41A5",
    ),
    s(
        "DMA",
        "dmc_dma_during_read4/read_write_2007.nes",
        120,
        "Passed",
    ),
    s("DMA", "read_joy3/thorough_test.nes", 600, "Passed"),
    // Mappers
    p("Mappers", "mmc3_test_2/rom_singles/1-clocking.nes"),
    p("Mappers", "mmc3_test_2/rom_singles/2-details.nes"),
    p("Mappers", "mmc3_test_2/rom_singles/3-A12_clocking.nes"),
    p("Mappers", "mmc3_test_2/rom_singles/4-scanline_timing.nes"),
    p("Mappers", "mmc3_test_2/rom_singles/5-MMC3.nes"),
    s("Mappers", "mmc3_irq_tests/1.Clocking.nes", 120, "PASSED"),
    s("Mappers", "mmc3_irq_tests/2.Details.nes", 120, "PASSED"),
    s(
        "Mappers",
        "mmc3_irq_tests/3.A12_clocking.nes",
        120,
        "PASSED",
    ),
    s(
        "Mappers",
        "mmc3_irq_tests/4.Scanline_timing.nes",
        120,
        "PASSED",
    ),
    s("Mappers", "mmc3_irq_tests/6.MMC3_rev_B.nes", 120, "PASSED"),
];

/// Runs one suite entry against ROMs under `rom_dir`.
pub fn run_entry(rom_dir: &Path, e: &Entry) -> EntryResult {
    let Ok(data) = std::fs::read(rom_dir.join(e.path)) else {
        return EntryResult::Missing;
    };
    let mut nes = match Nes::new(
        &data,
        Config {
            region: e.region,
            ..Config::default()
        },
    ) {
        Ok(n) => n,
        Err(err) => return EntryResult::Failed(err.to_string()),
    };
    match e.check {
        Check::Protocol => match run_blargg(&mut nes, 3600) {
            Outcome::Passed(_) => EntryResult::Passed,
            Outcome::Failed { code, message } => {
                EntryResult::Failed(format!("code {code}: {}", message.replace('\n', " ")))
            }
            Outcome::Timeout(m) => EntryResult::Failed(format!("timed out: {m}")),
            Outcome::NoProtocol => EntryResult::Failed("no $6000 result".into()),
        },
        Check::Screen { frames, expect } => {
            for _ in 0..frames {
                while nes.run_frame() != StopReason::FrameComplete {}
            }
            let text = screen_text(&nes);
            if expect.split('|').any(|x| text.contains(x)) {
                EntryResult::Passed
            } else {
                EntryResult::Failed(format!("screen: {}", text.replace('\n', " | ")))
            }
        }
    }
}
