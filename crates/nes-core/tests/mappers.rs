//! Robustness tests for every supported mapper, using synthetic ROMs: random
//! register writes must never panic, and save states must round-trip
//! deterministically.

use nes_core::cart::SUPPORTED_MAPPERS;
use nes_core::cpu::CpuBus;
use nes_core::testing::frame_hash;
use nes_core::{Config, Nes, StopReason};

/// A ROM whose every bank is NOPs with vectors pointing into PRG, so the CPU
/// runs harmlessly whatever gets banked in.
fn synthetic_rom(mapper: u16, chr_banks: u8) -> Vec<u8> {
    let prg_banks = 16u8; // 256K
    let mut rom = vec![
        b'N',
        b'E',
        b'S',
        0x1A,
        prg_banks,
        chr_banks,
        ((mapper as u8 & 0x0F) << 4) | 0x02, // battery
        (mapper as u8 & 0xF0) | 0x08,        // NES 2.0
        (mapper >> 8) as u8,
        0,
        0x07,                                  // 8K PRG RAM
        if chr_banks == 0 { 0x09 } else { 0 }, // 32K CHR RAM
        0,
        0,
        0,
        0,
    ];
    for bank in 0..prg_banks as usize {
        let mut b = vec![0xEAu8; 0x4000];
        // Each bank: NOPs, then JMP $8000 at the end, vectors to $8000.
        b[0x3FF7] = 0x4C;
        b[0x3FF8] = 0x00;
        b[0x3FF9] = 0x80;
        for v in [0x3FFA, 0x3FFC, 0x3FFE] {
            b[v] = 0x00;
            b[v + 1] = 0x80;
        }
        b[0] = bank as u8;
        rom.extend(b);
    }
    rom.extend((0..chr_banks as usize * 0x2000).map(|i| (i * 7) as u8));
    rom
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
}

fn run_frames(nes: &mut Nes, n: u32) {
    for _ in 0..n {
        while nes.run_frame() != StopReason::FrameComplete {}
    }
}

#[test]
fn every_mapper_survives_random_writes_and_round_trips_state() {
    for &mapper in SUPPORTED_MAPPERS {
        for chr_banks in [0u8, 16] {
            let rom = synthetic_rom(mapper, chr_banks);
            let mut nes = Nes::new(&rom, Config::default())
                .unwrap_or_else(|e| panic!("mapper {mapper}: {e}"));
            let mut rng = Lcg(mapper as u64 * 977 + chr_banks as u64);
            // Turn rendering on so PPU-snooping mappers see fetches.
            nes.bus.write(0x2001, 0x1E);
            for round in 0..20 {
                for _ in 0..100 {
                    let addr = 0x4020 + (rng.next() % (0x10000 - 0x4020)) as u16;
                    let val = rng.next() as u8;
                    nes.bus.write(addr, val);
                    let _ = nes.bus.read(0x8000 + (rng.next() % 0x8000) as u16);
                }
                nes.bus.write(0x2001, if round % 3 == 0 { 0 } else { 0x1E });
                run_frames(&mut nes, 2);
            }

            let state = nes.save_state();
            run_frames(&mut nes, 10);
            let (hash, cycles) = (frame_hash(&nes), nes.cycles());
            nes.load_state(&state)
                .unwrap_or_else(|e| panic!("mapper {mapper}: {e}"));
            run_frames(&mut nes, 10);
            assert_eq!(
                nes.cycles(),
                cycles,
                "mapper {mapper}: cycle count diverged after load"
            );
            assert_eq!(
                frame_hash(&nes),
                hash,
                "mapper {mapper}: picture diverged after load"
            );
            let _ = nes.save_data();
        }
    }
}
