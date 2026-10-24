//! Accuracy test ROMs. These need `test-data/` (run
//! `scripts/fetch-test-data.sh`); tests are skipped when ROMs are absent.
//! The list of ROMs lives in `nes_core::testing::suite`, shared with
//! `nes-cli compat-report`.

use nes_core::testing::suite::{EntryResult, SUITE, run_entry};
use nes_core::testing::{self};
use nes_core::{Config, Nes, StopReason};
use std::path::PathBuf;

fn rom_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-data/nes-test-roms")
}

fn check_group(group: &str) {
    let dir = rom_dir();
    let mut failures = Vec::new();
    let mut ran = 0;
    for e in SUITE.iter().filter(|e| e.group == group) {
        match run_entry(&dir, e) {
            EntryResult::Passed => ran += 1,
            EntryResult::Missing => {}
            EntryResult::Failed(why) => {
                ran += 1;
                failures.push(format!("{}: {why}", e.path));
            }
        }
    }
    if ran == 0 {
        eprintln!("skipping {group}: test data not downloaded");
    }
    assert!(
        failures.is_empty(),
        "{group} failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn cpu() {
    check_group("CPU");
}

#[test]
fn ppu() {
    check_group("PPU");
}

#[test]
fn apu() {
    check_group("APU");
}

#[test]
fn apu_pal() {
    check_group("APU (PAL)");
}

#[test]
fn dma() {
    check_group("DMA");
}

#[test]
fn mappers() {
    check_group("Mappers");
}

fn load(rel: &str) -> Option<Nes> {
    let data = std::fs::read(rom_dir().join(rel)).ok()?;
    Some(Nes::new(&data, Config::default()).unwrap_or_else(|e| panic!("{rel}: {e}")))
}

#[test]
fn nestest_matches_golden_log() {
    let (Some(mut nes), Ok(golden)) = (
        load("other/nestest.nes"),
        std::fs::read_to_string(rom_dir().join("other/nestest.log")),
    ) else {
        return;
    };
    nes.cpu.regs.pc = 0xC000;
    nes.bus.dbg.trace_enabled = true;
    for (i, expected) in golden.lines().enumerate() {
        nes.step_instruction();
        let trace = &nes.bus.dbg.trace;
        let actual = trace.get(trace.len() - 1).unwrap().nestest_line();
        // Registers and disassembly (columns 0-73) and the cycle count must
        // match; the PPU column depends on CPU/PPU power-up alignment, and
        // values shown for $40xx registers on the logger's open-bus model.
        let regs = |s: &str| s.get(48..73).map(str::to_string);
        let cyc = |s: &str| s.find("CYC:").map(|i| s[i..].trim_end().to_string());
        let text = |s: &str| s.get(..48).map(str::to_string);
        let io = expected.get(16..48).is_some_and(|op| op.contains(" $40"));
        let ok = regs(expected) == regs(&actual)
            && cyc(expected) == cyc(&actual)
            && (io || text(expected) == text(&actual));
        assert!(
            ok,
            "line {}:\nexpected {expected}\nactual   {actual}",
            i + 1
        );
    }
    assert_eq!(
        (nes.peek(2), nes.peek(3)),
        (0, 0),
        "nestest reported an error code"
    );
}

#[test]
fn save_state_round_trip_is_deterministic() {
    let Some(mut nes) = load("instr_test-v5/rom_singles/01-basics.nes") else {
        return;
    };
    let run = |nes: &mut Nes| {
        for _ in 0..30 {
            while nes.run_frame() != StopReason::FrameComplete {}
        }
    };
    run(&mut nes);
    let state = nes.save_state();
    run(&mut nes);
    let (expected, expected_cycles) = (testing::frame_hash(&nes), nes.cycles());
    nes.load_state(&state).unwrap();
    run(&mut nes);
    assert_eq!(nes.cycles(), expected_cycles);
    assert_eq!(testing::frame_hash(&nes), expected);
}
