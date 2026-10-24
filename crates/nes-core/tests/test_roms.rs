//! Accuracy test ROMs. These need `test-data/` (run
//! `scripts/fetch-test-data.sh`); each test is skipped when its ROM is absent.

use nes_core::testing::{self, Outcome};
use nes_core::{Config, Nes, StopReason};
use std::path::PathBuf;

fn rom_path(rel: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-data/nes-test-roms")
        .join(rel);
    if p.exists() {
        Some(p)
    } else {
        eprintln!("skipping {rel}: test data not downloaded");
        None
    }
}

fn load(rel: &str) -> Option<Nes> {
    let path = rom_path(rel)?;
    let data = std::fs::read(&path).unwrap();
    Some(Nes::new(&data, Config::default()).unwrap_or_else(|e| panic!("{rel}: {e}")))
}

/// ROMs that report through `$6000`.
fn check_blargg(roms: &[&str]) {
    let mut failures = Vec::new();
    for rel in roms {
        let Some(mut nes) = load(rel) else { continue };
        match testing::run_blargg(&mut nes, 3600) {
            Outcome::Passed(_) => {}
            other => failures.push(format!("{rel}: {other:?}")),
        }
    }
    assert!(failures.is_empty(), "failed:\n{}", failures.join("\n"));
}

/// Older ROMs that only print their result on screen.
fn check_screen(roms: &[(&str, u32, &str)]) {
    let mut failures = Vec::new();
    for (rel, frames, expect) in roms {
        let Some(mut nes) = load(rel) else { continue };
        for _ in 0..*frames {
            while nes.run_frame() != StopReason::FrameComplete {}
        }
        let text = testing::screen_text(&nes);
        if !text.contains(expect) {
            failures.push(format!("{rel}: screen says {text:?}"));
        }
    }
    assert!(failures.is_empty(), "failed:\n{}", failures.join("\n"));
}

#[test]
fn nestest_matches_golden_log() {
    let (Some(rom), Some(log)) = (rom_path("other/nestest.nes"), rom_path("other/nestest.log"))
    else {
        return;
    };
    let mut nes = Nes::new(&std::fs::read(rom).unwrap(), Config::default()).unwrap();
    let golden = std::fs::read_to_string(log).unwrap();
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
fn cpu_instructions() {
    check_blargg(&[
        "instr_test-v5/rom_singles/01-basics.nes",
        "instr_test-v5/rom_singles/02-implied.nes",
        "instr_test-v5/rom_singles/03-immediate.nes",
        "instr_test-v5/rom_singles/04-zero_page.nes",
        "instr_test-v5/rom_singles/05-zp_xy.nes",
        "instr_test-v5/rom_singles/06-absolute.nes",
        "instr_test-v5/rom_singles/07-abs_xy.nes",
        "instr_test-v5/rom_singles/08-ind_x.nes",
        "instr_test-v5/rom_singles/09-ind_y.nes",
        "instr_test-v5/rom_singles/10-branches.nes",
        "instr_test-v5/rom_singles/11-stack.nes",
        "instr_test-v5/rom_singles/12-jmp_jsr.nes",
        "instr_test-v5/rom_singles/13-rts.nes",
        "instr_test-v5/rom_singles/14-rti.nes",
        "instr_test-v5/rom_singles/15-brk.nes",
        "instr_test-v5/rom_singles/16-special.nes",
        "instr_misc/rom_singles/01-abs_x_wrap.nes",
        "instr_misc/rom_singles/02-branch_wrap.nes",
        "instr_misc/rom_singles/03-dummy_reads.nes",
        "instr_misc/rom_singles/04-dummy_reads_apu.nes",
    ]);
}

#[test]
fn cpu_timing_and_interrupts() {
    check_blargg(&[
        "instr_timing/rom_singles/1-instr_timing.nes",
        "instr_timing/rom_singles/2-branch_timing.nes",
        "cpu_interrupts_v2/rom_singles/1-cli_latency.nes",
        "cpu_interrupts_v2/rom_singles/2-nmi_and_brk.nes",
        "cpu_interrupts_v2/rom_singles/3-nmi_and_irq.nes",
        "cpu_interrupts_v2/rom_singles/4-irq_and_dma.nes",
        "cpu_interrupts_v2/rom_singles/5-branch_delays_irq.nes",
        "cpu_dummy_writes/cpu_dummy_writes_oam.nes",
        "cpu_dummy_writes/cpu_dummy_writes_ppumem.nes",
        "cpu_exec_space/test_cpu_exec_space_apu.nes",
        "cpu_exec_space/test_cpu_exec_space_ppuio.nes",
        "cpu_reset/ram_after_reset.nes",
        "cpu_reset/registers.nes",
    ]);
}

#[test]
fn apu() {
    check_blargg(&[
        "apu_test/rom_singles/1-len_ctr.nes",
        "apu_test/rom_singles/2-len_table.nes",
        "apu_test/rom_singles/3-irq_flag.nes",
        "apu_test/rom_singles/4-jitter.nes",
        "apu_test/rom_singles/5-len_timing.nes",
        "apu_test/rom_singles/6-irq_flag_timing.nes",
        "apu_test/rom_singles/7-dmc_basics.nes",
        "apu_test/rom_singles/8-dmc_rates.nes",
        "apu_reset/4015_cleared.nes",
        "apu_reset/4017_timing.nes",
        "apu_reset/4017_written.nes",
        "apu_reset/irq_flag_cleared.nes",
        "apu_reset/len_ctrs_enabled.nes",
        "apu_reset/works_immediately.nes",
        "sprdma_and_dmc_dma/sprdma_and_dmc_dma.nes",
        "sprdma_and_dmc_dma/sprdma_and_dmc_dma_512.nes",
    ]);
}

#[test]
fn ppu() {
    check_blargg(&[
        "ppu_vbl_nmi/rom_singles/01-vbl_basics.nes",
        "ppu_vbl_nmi/rom_singles/02-vbl_set_time.nes",
        "ppu_vbl_nmi/rom_singles/03-vbl_clear_time.nes",
        "ppu_vbl_nmi/rom_singles/04-nmi_control.nes",
        "ppu_vbl_nmi/rom_singles/05-nmi_timing.nes",
        "ppu_vbl_nmi/rom_singles/06-suppression.nes",
        "ppu_vbl_nmi/rom_singles/07-nmi_on_timing.nes",
        "ppu_vbl_nmi/rom_singles/08-nmi_off_timing.nes",
        "ppu_vbl_nmi/rom_singles/09-even_odd_frames.nes",
        "ppu_vbl_nmi/rom_singles/10-even_odd_timing.nes",
        "ppu_open_bus/ppu_open_bus.nes",
        "oam_read/oam_read.nes",
        "oam_stress/oam_stress.nes",
    ]);
}

#[test]
fn ppu_on_screen() {
    check_screen(&[
        ("sprite_hit_tests_2005.10.05/01.basics.nes", 120, "PASSED"),
        (
            "sprite_hit_tests_2005.10.05/02.alignment.nes",
            120,
            "PASSED",
        ),
        ("sprite_hit_tests_2005.10.05/03.corners.nes", 120, "PASSED"),
        ("sprite_hit_tests_2005.10.05/04.flip.nes", 120, "PASSED"),
        (
            "sprite_hit_tests_2005.10.05/05.left_clip.nes",
            120,
            "PASSED",
        ),
        (
            "sprite_hit_tests_2005.10.05/06.right_edge.nes",
            120,
            "PASSED",
        ),
        (
            "sprite_hit_tests_2005.10.05/07.screen_bottom.nes",
            120,
            "PASSED",
        ),
        (
            "sprite_hit_tests_2005.10.05/08.double_height.nes",
            120,
            "PASSED",
        ),
        (
            "sprite_hit_tests_2005.10.05/09.timing_basics.nes",
            120,
            "PASSED",
        ),
        (
            "sprite_hit_tests_2005.10.05/10.timing_order.nes",
            120,
            "PASSED",
        ),
        (
            "sprite_hit_tests_2005.10.05/11.edge_timing.nes",
            120,
            "PASSED",
        ),
        ("sprite_overflow_tests/1.Basics.nes", 120, "PASSED"),
        ("sprite_overflow_tests/2.Details.nes", 120, "PASSED"),
        ("sprite_overflow_tests/3.Timing.nes", 1500, "PASSED"),
        ("sprite_overflow_tests/4.Obscure.nes", 120, "PASSED"),
        ("sprite_overflow_tests/5.Emulator.nes", 120, "PASSED"),
        ("vbl_nmi_timing/1.frame_basics.nes", 1500, "PASSED"),
        ("vbl_nmi_timing/2.vbl_timing.nes", 1500, "PASSED"),
        ("vbl_nmi_timing/3.even_odd_frames.nes", 300, "PASSED"),
        ("vbl_nmi_timing/4.vbl_clear_timing.nes", 300, "PASSED"),
        ("vbl_nmi_timing/5.nmi_suppression.nes", 1500, "PASSED"),
        ("vbl_nmi_timing/6.nmi_disable.nes", 300, "PASSED"),
        ("vbl_nmi_timing/7.nmi_timing.nes", 300, "PASSED"),
        ("blargg_ppu_tests_2005.09.15b/palette_ram.nes", 120, "$01"),
        (
            "blargg_ppu_tests_2005.09.15b/power_up_palette.nes",
            120,
            "$01",
        ),
        ("blargg_ppu_tests_2005.09.15b/sprite_ram.nes", 120, "$01"),
        (
            "blargg_ppu_tests_2005.09.15b/vbl_clear_time.nes",
            120,
            "$01",
        ),
        ("blargg_ppu_tests_2005.09.15b/vram_access.nes", 120, "$01"),
    ]);
}

#[test]
fn save_state_round_trip_is_deterministic() {
    let Some(mut nes) = load("instr_test-v5/rom_singles/01-basics.nes") else {
        return;
    };
    for _ in 0..30 {
        while nes.run_frame() != StopReason::FrameComplete {}
    }
    let state = nes.save_state();
    for _ in 0..30 {
        while nes.run_frame() != StopReason::FrameComplete {}
    }
    let expected = testing::frame_hash(&nes);
    let expected_cycles = nes.cycles();
    nes.load_state(&state).unwrap();
    for _ in 0..30 {
        while nes.run_frame() != StopReason::FrameComplete {}
    }
    assert_eq!(nes.cycles(), expected_cycles);
    assert_eq!(testing::frame_hash(&nes), expected);
}
