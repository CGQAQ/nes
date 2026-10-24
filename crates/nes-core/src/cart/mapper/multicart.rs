//! Homebrew and multicart boards: Action 53 (28), UNROM 512 (30),
//! Camerica Quattro (232) and Action 52 (228).

use super::{Mapper, MapperInit};
use crate::cart::board::{Board, Mirroring};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Action53,
    Unrom512,
    Quattro,
    Action52,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Multicart {
    board: Board,
    kind: Kind,
    regs: [u8; 4],
    select: u8,
    /// UNROM 512 boards wired for one-screen mirroring control.
    one_screen: bool,
}

impl Multicart {
    pub fn new(init: MapperInit, kind: Kind) -> Self {
        let mut board = init.board;
        // Both boards bank 32K of CHR RAM, which iNES headers can't express.
        if matches!(kind, Kind::Unrom512 | Kind::Action53)
            && board.chr_rom.is_empty()
            && board.chr_ram.len() < 0x8000
        {
            board.chr_ram = vec![0; 0x8000];
        }
        let one_screen = kind == Kind::Unrom512 && board.mirroring == Mirroring::FourScreen;
        if one_screen {
            board.set_mirroring(Mirroring::SingleScreenA);
        }
        let mut m = Self {
            board,
            kind,
            // Action 53 powers up with the last bank in reach.
            regs: [0, 0, 0, 0x3F],
            select: 0,
            one_screen,
        };
        if kind == Kind::Action53 {
            m.regs = [0, 0x0F, 0x02, 0x3F];
        }
        m.update();
        m
    }

    fn update(&mut self) {
        let b = &mut self.board;
        match self.kind {
            Kind::Action53 => {
                let [chr, inner, mode, outer] = self.regs;
                b.set_chr_8k((chr & 3) as i32);
                let mirroring = match mode & 3 {
                    0 => Mirroring::SingleScreenA,
                    1 => Mirroring::SingleScreenB,
                    2 => Mirroring::Vertical,
                    _ => Mirroring::Horizontal,
                };
                b.set_mirroring(mirroring);
                // Game size: 32K, 64K, 128K or 256K, within a 256K outer bank;
                // PRG mode chooses which half is fixed.
                let size = (mode >> 4) & 3;
                let prg_mode = (mode >> 2) & 3;
                let outer16 = (outer as i32) << 1;
                let mask = (1i32 << (size + 1)) - 1;
                let bank_for = |sel: i32| (outer16 & !mask) | (sel & mask);
                match prg_mode {
                    0 | 1 => {
                        let base = bank_for((inner as i32) << 1);
                        b.set_prg_rom_16k(0x8000, base);
                        b.set_prg_rom_16k(0xC000, base | 1);
                    }
                    2 => {
                        b.set_prg_rom_16k(0x8000, outer16);
                        b.set_prg_rom_16k(0xC000, bank_for(inner as i32));
                    }
                    _ => {
                        b.set_prg_rom_16k(0x8000, bank_for(inner as i32));
                        b.set_prg_rom_16k(0xC000, outer16 | 1);
                    }
                }
            }
            Kind::Unrom512 => {
                let v = self.regs[0];
                b.set_prg_rom_16k(0x8000, (v & 0x1F) as i32);
                b.set_prg_rom_16k(0xC000, -1);
                b.set_chr_8k(((v >> 5) & 3) as i32);
                if self.one_screen {
                    b.set_mirroring(if v & 0x80 != 0 {
                        Mirroring::SingleScreenB
                    } else {
                        Mirroring::SingleScreenA
                    });
                }
            }
            Kind::Quattro => {
                let outer = ((self.regs[0] >> 3) & 3) as i32;
                let inner = (self.regs[1] & 3) as i32;
                b.set_prg_rom_16k(0x8000, (outer << 2) | inner);
                b.set_prg_rom_16k(0xC000, (outer << 2) | 3);
            }
            Kind::Action52 => {
                // Latched from the write address: A13 mirroring, A11-A12 chip,
                // A6-A10 bank, A5 16K mode; data selects CHR.
                let addr = u16::from_le_bytes([self.regs[0], self.regs[1]]);
                let chr = ((addr & 0x0F) << 2) as i32 | (self.regs[2] & 3) as i32;
                b.set_chr_8k(chr);
                let chip = match (addr >> 11) & 3 {
                    3 => 2,
                    c => c,
                } as i32;
                let bank = ((addr >> 6) & 0x1F) as i32 | (chip << 5);
                if addr & 0x20 != 0 {
                    b.set_prg_rom_16k(0x8000, bank);
                    b.set_prg_rom_16k(0xC000, bank);
                } else {
                    b.set_prg_rom_32k(bank >> 1);
                }
                b.set_mirroring(if addr & 0x2000 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                });
            }
        }
    }
}

impl Mapper for Multicart {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        match self.kind {
            Kind::Action53 => "Action 53",
            Kind::Unrom512 => "UNROM 512",
            Kind::Quattro => "Camerica Quattro",
            Kind::Action52 => "Action 52",
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match self.kind {
            Kind::Action53 => {
                if (0x5000..0x6000).contains(&addr) {
                    self.select = val & 0x81;
                } else if addr >= 0x8000 {
                    let reg = match self.select {
                        0x00 => 0,
                        0x01 => 1,
                        0x80 => 2,
                        _ => 3,
                    };
                    // One-screen modes take their nametable from data bit 4.
                    if reg <= 1 && self.regs[2] & 2 == 0 {
                        self.regs[2] = (self.regs[2] & 0xFE) | ((val >> 4) & 1);
                    }
                    self.regs[reg] = val;
                } else {
                    self.board.write_prg(addr, val);
                }
            }
            Kind::Unrom512 => {
                if addr >= 0x8000 {
                    self.regs[0] = val;
                }
            }
            Kind::Quattro => match addr {
                0x8000..=0xBFFF => self.regs[0] = val,
                0xC000..=0xFFFF => self.regs[1] = val,
                _ => self.board.write_prg(addr, val),
            },
            Kind::Action52 => {
                if addr >= 0x8000 {
                    let [lo, hi] = addr.to_le_bytes();
                    self.regs[0] = lo;
                    self.regs[1] = hi;
                    self.regs[2] = val;
                }
            }
        }
        self.update();
    }
}
