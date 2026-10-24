//! Smaller Japanese ASIC boards: Jaleco SS88006 (18), Irem G-101 (32) and
//! H3001 (65), Taito TC0190 (33), TC0690 (48) and X1-005 (80), and
//! Sunsoft-4 (68).

use super::{Mapper, MapperInit};
use crate::cart::board::{Board, Mirroring};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Jaleco18,
    IremG101,
    IremH3001,
    Tc0190,
    Tc0690,
    X1005,
    Sunsoft4,
}

impl Kind {
    pub fn from_mapper(mapper: u16) -> Option<Kind> {
        Some(match mapper {
            18 => Kind::Jaleco18,
            32 => Kind::IremG101,
            65 => Kind::IremH3001,
            33 => Kind::Tc0190,
            48 => Kind::Tc0690,
            80 | 207 => Kind::X1005,
            68 => Kind::Sunsoft4,
            _ => return None,
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct JpMisc {
    board: Board,
    kind: Kind,
    submapper: u8,
    prg: [u8; 4],
    chr: [u8; 8],
    nt: [u8; 2],
    mode: u8,
    irq_latch: u16,
    irq_counter: u16,
    irq_enabled: bool,
    irq_pending: bool,
    /// TC0690 scanline counter: A12 tracking like MMC3.
    irq_reload: bool,
    a12: bool,
    a12_low: u8,
    /// X1-005 internal RAM ($7F00-$7FFF) and its enable.
    x1_ram: Vec<u8>,
    x1_ram_enabled: bool,
}

fn hv(vertical: bool) -> Mirroring {
    if vertical {
        Mirroring::Vertical
    } else {
        Mirroring::Horizontal
    }
}

impl JpMisc {
    pub fn new(init: MapperInit, kind: Kind) -> Self {
        let mut m = Self {
            board: init.board,
            kind,
            submapper: init.info.submapper,
            prg: [0, 1, 0xFE, 0xFF],
            chr: [0, 1, 2, 3, 4, 5, 6, 7],
            nt: [0x80, 0x80],
            mode: 0,
            irq_latch: 0,
            irq_counter: 0,
            irq_enabled: false,
            irq_pending: false,
            irq_reload: false,
            a12: false,
            a12_low: 0,
            x1_ram: if kind == Kind::X1005 {
                vec![0; 128]
            } else {
                Vec::new()
            },
            x1_ram_enabled: false,
        };
        if kind == Kind::Jaleco18 || kind == Kind::IremH3001 {
            m.prg = [0, 1, 2, 0xFF];
        }
        m.update();
        m
    }

    fn update(&mut self) {
        let b = &mut self.board;
        match self.kind {
            Kind::Jaleco18 | Kind::IremH3001 | Kind::X1005 => {
                for i in 0..3 {
                    b.set_prg_rom_8k(1 + i, self.prg[i] as i32);
                }
                b.set_prg_rom_8k(4, -1);
                if self.kind == Kind::X1005 {
                    b.set_chr_2k(0, (self.chr[0] >> 1) as i32);
                    b.set_chr_2k(2, (self.chr[1] >> 1) as i32);
                    for i in 0..4 {
                        b.set_chr_1k(4 + i, self.chr[2 + i] as i32);
                    }
                } else {
                    for (i, &bank) in self.chr.iter().enumerate() {
                        b.set_chr_1k(i, bank as i32);
                    }
                }
            }
            Kind::IremG101 => {
                if self.mode & 2 != 0 {
                    b.set_prg_rom_8k(1, -2);
                    b.set_prg_rom_8k(3, self.prg[0] as i32);
                } else {
                    b.set_prg_rom_8k(1, self.prg[0] as i32);
                    b.set_prg_rom_8k(3, -2);
                }
                b.set_prg_rom_8k(2, self.prg[1] as i32);
                b.set_prg_rom_8k(4, -1);
                for (i, &bank) in self.chr.iter().enumerate() {
                    b.set_chr_1k(i, bank as i32);
                }
                if self.submapper == 1 {
                    b.set_mirroring(Mirroring::SingleScreenA);
                }
            }
            Kind::Tc0190 | Kind::Tc0690 => {
                b.set_prg_rom_8k(1, (self.prg[0] & 0x3F) as i32);
                b.set_prg_rom_8k(2, (self.prg[1] & 0x3F) as i32);
                b.set_prg_rom_8k(3, -2);
                b.set_prg_rom_8k(4, -1);
                b.set_chr_2k(0, self.chr[0] as i32);
                b.set_chr_2k(2, self.chr[1] as i32);
                for i in 0..4 {
                    b.set_chr_1k(4 + i, self.chr[2 + i] as i32);
                }
            }
            Kind::Sunsoft4 => {
                for i in 0..4 {
                    b.set_chr_2k(i * 2, self.chr[i] as i32);
                }
                b.set_prg_rom_16k(0x8000, (self.prg[0] & 0x0F) as i32);
                b.set_prg_rom_16k(0xC000, -1);
                if !b.prg_ram.is_empty() {
                    if self.prg[0] & 0x10 != 0 {
                        b.set_prg_ram_8k(0, 0);
                    } else {
                        b.unmap_prg(0);
                    }
                }
                let mirroring = match self.mode & 3 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::SingleScreenA,
                    _ => Mirroring::SingleScreenB,
                };
                b.set_mirroring(mirroring);
                if self.mode & 0x10 != 0 {
                    // Nametables come from CHR ROM ($80 + register).
                    let pages: [usize; 4] = match self.mode & 3 {
                        0 => [0, 1, 0, 1],
                        1 => [0, 0, 1, 1],
                        2 => [0, 0, 0, 0],
                        _ => [1, 1, 1, 1],
                    };
                    for (slot, p) in pages.iter().enumerate() {
                        b.set_nt_chr_rom(slot, (self.nt[*p] | 0x80) as i32);
                    }
                }
            }
        }
    }

    fn clock_a12_irq(&mut self) {
        if self.irq_counter == 0 || self.irq_reload {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 && self.irq_enabled {
            self.irq_pending = true;
        }
    }

    fn watch_a12(&mut self, addr: u16) {
        if self.kind != Kind::Tc0690 {
            return;
        }
        let a12 = addr & 0x1000 != 0;
        if a12 && !self.a12 && self.a12_low >= 4 {
            self.clock_a12_irq();
        }
        if a12 {
            self.a12_low = 0;
        }
        self.a12 = a12;
    }

    fn write_jaleco(&mut self, addr: u16, val: u8) {
        let lo = val & 0x0F;
        let nib = |cur: u8, hi: bool| {
            if hi {
                (cur & 0x0F) | (lo << 4)
            } else {
                (cur & 0xF0) | lo
            }
        };
        let n = addr & 3;
        match addr & 0xF000 {
            0x8000 => {
                let i = (n >> 1) as usize;
                self.prg[i] = nib(self.prg[i], n & 1 != 0);
            }
            0x9000 => match n {
                0 | 1 => self.prg[2] = nib(self.prg[2], n & 1 != 0),
                _ => {
                    if !self.board.prg_ram.is_empty() {
                        if val & 1 != 0 {
                            self.board.set_prg_ram_8k(0, 0);
                            self.board.set_prg_writable(0, val & 2 != 0);
                        } else {
                            self.board.unmap_prg(0);
                        }
                    }
                }
            },
            0xA000..=0xD000 => {
                let i = (((addr & 0xF000) - 0xA000) >> 11) as usize + (n >> 1) as usize;
                self.chr[i] = nib(self.chr[i], n & 1 != 0);
            }
            0xE000 => {
                let shift = n * 4;
                self.irq_latch = (self.irq_latch & !(0xF << shift)) | ((lo as u16) << shift);
            }
            0xF000 => match n {
                0 => {
                    self.irq_pending = false;
                    self.irq_counter = self.irq_latch;
                }
                1 => {
                    self.irq_pending = false;
                    self.irq_enabled = val & 1 != 0;
                    self.mode = val >> 1;
                }
                2 => self.board.set_mirroring(match val & 3 {
                    0 => Mirroring::Horizontal,
                    1 => Mirroring::Vertical,
                    2 => Mirroring::SingleScreenA,
                    _ => Mirroring::SingleScreenB,
                }),
                _ => {} // uPD7756 speech: not emulated
            },
            _ => {}
        }
    }
}

impl Mapper for JpMisc {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        match self.kind {
            Kind::Jaleco18 => "Jaleco SS88006",
            Kind::IremG101 => "Irem G-101",
            Kind::IremH3001 => "Irem H3001",
            Kind::Tc0190 => "Taito TC0190",
            Kind::Tc0690 => "Taito TC0690",
            Kind::X1005 => "Taito X1-005",
            Kind::Sunsoft4 => "Sunsoft-4",
        }
    }

    fn cpu_read(&mut self, addr: u16) -> Option<u8> {
        self.cpu_peek(addr)
    }

    fn cpu_peek(&self, addr: u16) -> Option<u8> {
        if self.kind == Kind::X1005 && (0x7F00..0x8000).contains(&addr) {
            return self
                .x1_ram_enabled
                .then(|| self.x1_ram[(addr & 0x7F) as usize]);
        }
        self.board.read_prg(addr)
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match self.kind {
            Kind::Jaleco18 => {
                if addr >= 0x8000 {
                    self.write_jaleco(addr, val);
                } else {
                    self.board.write_prg(addr, val);
                }
            }
            Kind::IremG101 => match addr & 0xF000 {
                0x8000 => self.prg[0] = val & 0x1F,
                0x9000 => {
                    self.mode = val;
                    self.board.set_mirroring(if val & 1 != 0 {
                        Mirroring::Horizontal
                    } else {
                        Mirroring::Vertical
                    });
                }
                0xA000 => self.prg[1] = val & 0x1F,
                0xB000 => self.chr[(addr & 7) as usize] = val,
                _ => self.board.write_prg(addr, val),
            },
            Kind::IremH3001 => match addr {
                0x8000 => self.prg[0] = val,
                0x9001 => self.board.set_mirroring(hv(val & 0x80 == 0)),
                0x9003 => {
                    self.irq_enabled = val & 0x80 != 0;
                    self.irq_pending = false;
                }
                0x9004 => {
                    self.irq_counter = self.irq_latch;
                    self.irq_pending = false;
                }
                0x9005 => self.irq_latch = (self.irq_latch & 0x00FF) | ((val as u16) << 8),
                0x9006 => self.irq_latch = (self.irq_latch & 0xFF00) | val as u16,
                0xA000 => self.prg[1] = val,
                0xB000..=0xB007 => self.chr[(addr & 7) as usize] = val,
                0xC000 => self.prg[2] = val,
                _ => self.board.write_prg(addr, val),
            },
            Kind::Tc0190 | Kind::Tc0690 => match addr & 0xE003 {
                0x8000 => {
                    self.prg[0] = val;
                    if self.kind == Kind::Tc0190 {
                        self.board.set_mirroring(hv(val & 0x40 == 0));
                    }
                }
                0x8001 => self.prg[1] = val,
                0x8002 => self.chr[0] = val,
                0x8003 => self.chr[1] = val,
                0xA000..=0xA003 => self.chr[2 + (addr & 3) as usize] = val,
                0xC000 if self.kind == Kind::Tc0690 => self.irq_latch = (val ^ 0xFF) as u16,
                0xC001 if self.kind == Kind::Tc0690 => {
                    self.irq_counter = 0;
                    self.irq_reload = true;
                }
                0xC002 if self.kind == Kind::Tc0690 => self.irq_enabled = true,
                0xC003 if self.kind == Kind::Tc0690 => {
                    self.irq_enabled = false;
                    self.irq_pending = false;
                }
                0xE000 if self.kind == Kind::Tc0690 => {
                    self.board.set_mirroring(hv(val & 0x40 == 0))
                }
                _ => self.board.write_prg(addr, val),
            },
            Kind::X1005 => match addr {
                0x7EF0 | 0x7EF1 => self.chr[(addr & 1) as usize] = val,
                0x7EF2..=0x7EF5 => self.chr[(addr - 0x7EF0) as usize] = val,
                0x7EF6 | 0x7EF7 => self.board.set_mirroring(hv(val & 1 != 0)),
                0x7EF8 | 0x7EF9 => self.x1_ram_enabled = val == 0xA3,
                0x7EFA | 0x7EFB => self.prg[0] = val,
                0x7EFC | 0x7EFD => self.prg[1] = val,
                0x7EFE | 0x7EFF => self.prg[2] = val,
                0x7F00..=0x7FFF => {
                    if self.x1_ram_enabled {
                        self.x1_ram[(addr & 0x7F) as usize] = val;
                    }
                }
                _ => self.board.write_prg(addr, val),
            },
            Kind::Sunsoft4 => match addr & 0xF000 {
                0x8000 | 0x9000 | 0xA000 | 0xB000 => {
                    self.chr[((addr - 0x8000) >> 12) as usize] = val
                }
                0xC000 => self.nt[0] = val,
                0xD000 => self.nt[1] = val,
                0xE000 => self.mode = val,
                0xF000 => self.prg[0] = val,
                _ => self.board.write_prg(addr, val),
            },
        }
        self.update();
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.watch_a12(addr);
        self.board.read_ppu(addr)
    }

    fn ppu_address(&mut self, addr: u16) {
        self.watch_a12(addr);
    }

    fn cpu_clock(&mut self) {
        match self.kind {
            Kind::Tc0690 => {
                if !self.a12 {
                    self.a12_low = self.a12_low.saturating_add(1);
                }
            }
            Kind::IremH3001 => {
                if self.irq_enabled && self.irq_counter > 0 {
                    self.irq_counter -= 1;
                    if self.irq_counter == 0 {
                        self.irq_pending = true;
                        self.irq_enabled = false;
                    }
                }
            }
            Kind::Jaleco18 if self.irq_enabled => {
                let mask: u16 = match self.mode & 7 {
                    m if m & 4 != 0 => 0x000F,
                    m if m & 2 != 0 => 0x00FF,
                    m if m & 1 != 0 => 0x0FFF,
                    _ => 0xFFFF,
                };
                let low = (self.irq_counter & mask).wrapping_sub(1) & mask;
                self.irq_counter = (self.irq_counter & !mask) | low;
                if low == 0 {
                    self.irq_pending = true;
                }
            }
            _ => {}
        }
    }

    fn irq(&self) -> bool {
        self.irq_pending
    }

    fn save_data(&self) -> Option<Vec<u8>> {
        let b = &self.board;
        if self.kind == Kind::X1005 {
            return b.battery.then(|| self.x1_ram.clone());
        }
        b.battery.then(|| b.prg_ram.clone())
    }

    fn load_save_data(&mut self, data: &[u8]) {
        let target = if self.kind == Kind::X1005 {
            &mut self.x1_ram
        } else {
            &mut self.board.prg_ram
        };
        let n = target.len().min(data.len());
        target[..n].copy_from_slice(&data[..n]);
    }

    fn debug_state(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "PRG",
                self.prg
                    .iter()
                    .map(|c| format!("{c:02X}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            (
                "CHR",
                self.chr
                    .iter()
                    .map(|c| format!("{c:02X}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            ("Mode", format!("${:02X}", self.mode)),
            (
                "IRQ",
                format!(
                    "latch {} counter {} {} {}",
                    self.irq_latch, self.irq_counter, self.irq_enabled, self.irq_pending
                ),
            ),
        ]
    }
}
