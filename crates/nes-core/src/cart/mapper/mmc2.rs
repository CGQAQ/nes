//! Mappers 9 and 10: Nintendo MMC2 (PxROM) and MMC4 (FxROM).
//!
//! Each 4K CHR half has two bank registers; which one is live is chosen by a
//! latch that flips when the PPU fetches tile $FD or $FE from that half. The
//! switch happens after the triggering fetch, so that tile still comes from
//! the old bank.

use super::{Mapper, MapperInit};
use crate::cart::board::{Board, Mirroring};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Mmc2 {
    board: Board,
    /// MMC4: 16K PRG banking and latch ranges for both halves.
    mmc4: bool,
    prg: u8,
    /// [half][latch]: banks for latch = $FD (0) and $FE (1).
    chr: [[u8; 2]; 2],
    latch: [usize; 2],
}

impl Mmc2 {
    pub fn new(init: MapperInit, mmc4: bool) -> Self {
        let mut m = Self {
            board: init.board,
            mmc4,
            prg: 0,
            chr: [[0; 2]; 2],
            latch: [1, 1],
        };
        if !mmc4 {
            // MMC2: 8K switchable at $8000, the last three banks fixed.
            m.board.set_prg_rom_8k(2, -3);
            m.board.set_prg_rom_8k(3, -2);
            m.board.set_prg_rom_8k(4, -1);
        }
        m.update();
        m
    }

    fn update(&mut self) {
        if self.mmc4 {
            self.board.set_prg_rom_16k(0x8000, self.prg as i32);
        } else {
            self.board.set_prg_rom_8k(1, self.prg as i32);
        }
        for half in 0..2 {
            let bank = self.chr[half][self.latch[half]];
            self.board.set_chr_4k(half * 4, bank as i32);
        }
    }

    fn watch(&mut self, addr: u16) {
        let half = (addr >> 12) as usize & 1;
        let tile = addr & 0x0FF8;
        // MMC2's first half only reacts to the exact addresses $0FD8/$0FE8.
        let exact = !self.mmc4 && half == 0;
        let hit = |t: u16| if exact { addr == t } else { tile == t };
        let new = if hit(0x0FD8) {
            0
        } else if hit(0x0FE8) {
            1
        } else {
            return;
        };
        if self.latch[half] != new {
            self.latch[half] = new;
            self.update();
        }
    }
}

impl Mapper for Mmc2 {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        if self.mmc4 { "MMC4" } else { "MMC2" }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0xA000..=0xAFFF => self.prg = val & 0x0F,
            0xB000..=0xBFFF => self.chr[0][0] = val & 0x1F,
            0xC000..=0xCFFF => self.chr[0][1] = val & 0x1F,
            0xD000..=0xDFFF => self.chr[1][0] = val & 0x1F,
            0xE000..=0xEFFF => self.chr[1][1] = val & 0x1F,
            0xF000..=0xFFFF => {
                self.board.set_mirroring(if val & 1 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                });
            }
            _ => {
                self.board.write_prg(addr, val);
                return;
            }
        }
        self.update();
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        let v = self.board.read_ppu(addr);
        if addr < 0x2000 {
            self.watch(addr);
        }
        v
    }

    fn debug_state(&self) -> Vec<(&'static str, String)> {
        vec![
            ("PRG", format!("${:02X}", self.prg)),
            (
                "CHR $0000 FD/FE",
                format!("${:02X} / ${:02X}", self.chr[0][0], self.chr[0][1]),
            ),
            (
                "CHR $1000 FD/FE",
                format!("${:02X} / ${:02X}", self.chr[1][0], self.chr[1][1]),
            ),
            (
                "Latches",
                format!(
                    "{} / {}",
                    ["FD", "FE"][self.latch[0]],
                    ["FD", "FE"][self.latch[1]]
                ),
            ),
        ]
    }
}
