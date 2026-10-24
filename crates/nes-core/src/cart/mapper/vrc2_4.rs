//! Mappers 21, 22, 23, 25: Konami VRC2 and VRC4.
//!
//! The same chip family appears with its two register-select pins wired to
//! different CPU address lines on different boards. NES 2.0 submappers say
//! which; without one, both candidate lines are ORed together, which works
//! for every known game.

use super::vrc_irq::VrcIrq;
use super::{Mapper, MapperInit};
use crate::cart::board::{Board, Mirroring};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Vrc24 {
    board: Board,
    mapper: u16,
    submapper: u8,
    vrc2: bool,
    prg: [u8; 2],
    chr: [u16; 8],
    swap_mode: bool,
    wram_enabled: bool,
    /// VRC2's one-bit microwire latch at $6000, used for copy protection.
    latch: u8,
    irq: VrcIrq,
}

impl Vrc24 {
    pub fn new(init: MapperInit) -> Self {
        let mapper = init.info.mapper;
        let submapper = init.info.submapper;
        let vrc2 =
            mapper == 22 || (mapper == 23 && submapper == 3) || (mapper == 25 && submapper == 3);
        let mut m = Self {
            board: init.board,
            mapper,
            submapper,
            vrc2,
            prg: [0, 1],
            chr: [0; 8],
            swap_mode: false,
            wram_enabled: true,
            latch: 0,
            irq: VrcIrq::default(),
        };
        m.update();
        m
    }

    /// Maps a CPU address to the chip's register index (0-3).
    fn register(&self, addr: u16) -> u16 {
        let bit = |n: u16| (addr >> n) & 1;
        let (a0, a1) = match (self.mapper, self.submapper) {
            (21, 1) => (bit(1), bit(2)),
            (21, 2) => (bit(6), bit(7)),
            (21, _) => (bit(1) | bit(6), bit(2) | bit(7)),
            (22, _) => (bit(1), bit(0)),
            (23, 1) | (23, 3) => (bit(0), bit(1)),
            (23, 2) => (bit(2), bit(3)),
            (23, _) => (bit(0) | bit(2), bit(1) | bit(3)),
            (25, 1) | (25, 3) => (bit(1), bit(0)),
            (25, 2) => (bit(3), bit(2)),
            _ => (bit(1) | bit(3), bit(0) | bit(2)),
        };
        a0 | (a1 << 1)
    }

    fn update(&mut self) {
        let b = &mut self.board;
        if self.swap_mode && !self.vrc2 {
            b.set_prg_rom_8k(1, -2);
            b.set_prg_rom_8k(3, self.prg[0] as i32);
        } else {
            b.set_prg_rom_8k(1, self.prg[0] as i32);
            b.set_prg_rom_8k(3, -2);
        }
        b.set_prg_rom_8k(2, self.prg[1] as i32);
        b.set_prg_rom_8k(4, -1);
        for (i, &bank) in self.chr.iter().enumerate() {
            // VRC2a ignores the low CHR bank bit.
            let bank = if self.mapper == 22 { bank >> 1 } else { bank };
            b.set_chr_1k(i, bank as i32);
        }
        if !b.prg_ram.is_empty() {
            if self.wram_enabled || self.vrc2 {
                b.set_prg_ram_8k(0, 0);
            } else {
                b.unmap_prg(0);
            }
        }
    }
}

impl Mapper for Vrc24 {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        if self.vrc2 { "VRC2" } else { "VRC4" }
    }

    fn cpu_read(&mut self, addr: u16) -> Option<u8> {
        self.cpu_peek(addr)
    }

    fn cpu_peek(&self, addr: u16) -> Option<u8> {
        if self.vrc2 && (0x6000..0x7000).contains(&addr) && self.board.prg_ram.is_empty() {
            // Only bit 0 is driven; the rest is typically $6x open bus.
            return Some(0x60 | self.latch);
        }
        self.board.read_prg(addr)
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr < 0x8000 {
            if self.vrc2 && (0x6000..0x7000).contains(&addr) && self.board.prg_ram.is_empty() {
                self.latch = val & 1;
            } else {
                self.board.write_prg(addr, val);
            }
            return;
        }
        let reg = self.register(addr);
        match (addr & 0xF000, reg) {
            (0x8000, _) => self.prg[0] = val & 0x1F,
            (0x9000, 0) | (0x9000, 1) => {
                let mode = if self.vrc2 { val & 1 } else { val & 3 };
                self.board.set_mirroring(match mode {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::SingleScreenA,
                    _ => Mirroring::SingleScreenB,
                });
            }
            (0x9000, _) => {
                if !self.vrc2 {
                    self.wram_enabled = val & 1 != 0;
                    self.swap_mode = val & 2 != 0;
                }
            }
            (0xA000, _) => self.prg[1] = val & 0x1F,
            (0xB000..=0xE000, r) => {
                let slot = (((addr & 0xF000) - 0xB000) >> 11) as usize + (r >> 1) as usize;
                let cur = self.chr[slot];
                self.chr[slot] = if r & 1 == 0 {
                    (cur & 0x1F0) | (val as u16 & 0x0F)
                } else {
                    (cur & 0x0F) | ((val as u16 & 0x1F) << 4)
                };
            }
            (0xF000, 0) => self.irq.write_latch_lo(val),
            (0xF000, 1) => self.irq.write_latch_hi(val),
            (0xF000, 2) => self.irq.write_control(val),
            (0xF000, _) => self.irq.acknowledge(),
            _ => {}
        }
        self.update();
    }

    fn cpu_clock(&mut self) {
        if !self.vrc2 {
            self.irq.clock();
        }
    }

    fn irq(&self) -> bool {
        self.irq.pending
    }

    fn debug_state(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "PRG",
                format!(
                    "{:02X} {:02X} swap={}",
                    self.prg[0], self.prg[1], self.swap_mode
                ),
            ),
            (
                "CHR",
                self.chr
                    .iter()
                    .map(|c| format!("{c:02X}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            ("IRQ", self.irq.describe()),
        ]
    }
}
