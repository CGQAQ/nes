//! Mapper 1 (and 155): Nintendo MMC1 — SxROM boards.
//!
//! Registers are loaded serially, one bit per write. The chip ignores a write
//! on the CPU cycle right after another (so read-modify-write instructions,
//! which write twice, only count once — some games rely on this). Large
//! boards reuse CHR register bits for PRG ROM/RAM banking: SUROM (512K PRG),
//! SOROM (16K PRG RAM) and SXROM (32K PRG RAM).

use super::{Mapper, MapperInit};
use crate::cart::board::{Board, Mirroring};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Mmc1 {
    board: Board,
    shift: u8,
    shift_count: u8,
    control: u8,
    chr0: u8,
    chr1: u8,
    prg: u8,
    cycle: u64,
    last_write: u64,
    /// MMC1A has no PRG RAM disable bit.
    ram_disable_supported: bool,
}

impl Mmc1 {
    pub fn new(init: MapperInit) -> Self {
        let mut m = Self {
            board: init.board,
            shift: 0,
            shift_count: 0,
            control: 0x0C,
            chr0: 0,
            chr1: 0,
            prg: 0,
            cycle: 0,
            last_write: u64::MAX - 1,
            ram_disable_supported: init.info.mapper != 155,
        };
        m.update();
        m
    }

    fn update(&mut self) {
        let b = &mut self.board;
        b.set_mirroring(match self.control & 3 {
            0 => Mirroring::SingleScreenA,
            1 => Mirroring::SingleScreenB,
            2 => Mirroring::Vertical,
            _ => Mirroring::Horizontal,
        });

        // SUROM/SXROM: CHR bank 0 bit 4 selects the 256K PRG half.
        let outer = if b.prg_rom_len() > 0x40000 {
            (self.chr0 & 0x10) as i32
        } else {
            0
        };
        let bank = (self.prg & 0x0F) as i32;
        match (self.control >> 2) & 3 {
            0 | 1 => b.set_prg_rom_32k((outer | bank) >> 1),
            2 => {
                b.set_prg_rom_16k(0x8000, outer);
                b.set_prg_rom_16k(0xC000, outer | bank);
            }
            _ => {
                b.set_prg_rom_16k(0x8000, outer | bank);
                b.set_prg_rom_16k(0xC000, outer | 0x0F);
            }
        }

        if self.control & 0x10 != 0 {
            b.set_chr_4k(0, self.chr0 as i32);
            b.set_chr_4k(4, self.chr1 as i32);
        } else {
            b.set_chr_8k((self.chr0 >> 1) as i32);
        }

        // PRG RAM banking on SOROM (16K) and SXROM (32K).
        let ram_bank = match b.prg_ram.len() {
            0x4000 => ((self.chr0 >> 3) & 1) as i32,
            0x8000 => ((self.chr0 >> 2) & 3) as i32,
            _ => 0,
        };
        if !b.prg_ram.is_empty() {
            if self.ram_disable_supported && self.prg & 0x10 != 0 {
                b.unmap_prg(0);
            } else {
                b.set_prg_ram_8k(0, ram_bank);
            }
        }
    }
}

impl Mapper for Mmc1 {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        "MMC1"
    }

    fn cpu_clock(&mut self) {
        self.cycle += 1;
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr < 0x8000 {
            self.board.write_prg(addr, val);
            return;
        }
        let consecutive = self.cycle == self.last_write + 1;
        self.last_write = self.cycle;
        if consecutive {
            return;
        }
        if val & 0x80 != 0 {
            self.shift = 0;
            self.shift_count = 0;
            self.control |= 0x0C;
            self.update();
            return;
        }
        self.shift |= (val & 1) << self.shift_count;
        self.shift_count += 1;
        if self.shift_count == 5 {
            let v = self.shift;
            match (addr >> 13) & 3 {
                0 => self.control = v,
                1 => self.chr0 = v,
                2 => self.chr1 = v,
                _ => self.prg = v,
            }
            self.shift = 0;
            self.shift_count = 0;
            self.update();
        }
    }

    fn debug_state(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Control", format!("${:02X}", self.control)),
            ("CHR 0", format!("${:02X}", self.chr0)),
            ("CHR 1", format!("${:02X}", self.chr1)),
            ("PRG", format!("${:02X}", self.prg)),
            (
                "Shift",
                format!("{:05b} ({} bits)", self.shift, self.shift_count),
            ),
        ]
    }
}
