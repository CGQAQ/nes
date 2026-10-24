//! Konami VRC7 (85, with FM audio), VRC1 (75) and VRC3 (73).

use super::vrc_irq::VrcIrq;
use super::{Mapper, MapperInit};
use crate::cart::board::{Board, Mirroring};
use crate::cart::opll::Opll;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Vrc7 {
    board: Board,
    prg: [u8; 3],
    chr: [u8; 8],
    control: u8,
    irq: VrcIrq,
    opll: Opll,
}

impl Vrc7 {
    pub fn new(init: MapperInit) -> Self {
        let mut m = Self {
            board: init.board,
            prg: [0; 3],
            chr: [0; 8],
            control: 0,
            irq: VrcIrq::default(),
            opll: Opll::default(),
        };
        m.update();
        m
    }

    fn update(&mut self) {
        let b = &mut self.board;
        for i in 0..3 {
            b.set_prg_rom_8k(1 + i, self.prg[i] as i32);
        }
        b.set_prg_rom_8k(4, -1);
        for (i, &bank) in self.chr.iter().enumerate() {
            b.set_chr_1k(i, bank as i32);
        }
        b.set_mirroring(match self.control & 3 {
            0 => Mirroring::Vertical,
            1 => Mirroring::Horizontal,
            2 => Mirroring::SingleScreenA,
            _ => Mirroring::SingleScreenB,
        });
        if !b.prg_ram.is_empty() {
            if self.control & 0x80 != 0 {
                b.set_prg_ram_8k(0, 0);
            } else {
                b.unmap_prg(0);
            }
        }
    }
}

impl Mapper for Vrc7 {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        "VRC7"
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr < 0x8000 {
            self.board.write_prg(addr, val);
            return;
        }
        // VRC7a decodes A4, VRC7b decodes A3, for the second register of each pair.
        let hi = addr & 0x18 != 0;
        match (addr & 0xF000, hi) {
            (0x8000, false) => self.prg[0] = val & 0x3F,
            (0x8000, true) => self.prg[1] = val & 0x3F,
            (0x9000, false) => self.prg[2] = val & 0x3F,
            (0x9000, true) => match addr & 0xF030 {
                0x9010 => self.opll.write_address(val),
                0x9030 => self.opll.write_data(val),
                _ => {}
            },
            (0xA000..=0xD000, h) => {
                let slot = (((addr & 0xF000) - 0xA000) >> 11) as usize + h as usize;
                self.chr[slot] = val;
            }
            (0xE000, false) => {
                if val & 0x40 != 0 && self.control & 0x40 == 0 {
                    self.opll.reset();
                }
                self.control = val;
            }
            (0xE000, true) => self.irq.write_latch(val),
            (0xF000, false) => self.irq.write_control(val),
            (0xF000, true) => self.irq.acknowledge(),
            _ => {}
        }
        self.update();
    }

    fn cpu_clock(&mut self) {
        self.irq.clock();
        if self.control & 0x40 == 0 {
            self.opll.clock();
        }
    }

    fn irq(&self) -> bool {
        self.irq.pending
    }

    fn expansion_audio(&self) -> f32 {
        if self.control & 0x40 == 0 {
            self.opll.output()
        } else {
            0.0
        }
    }

    fn debug_state(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "PRG",
                format!(
                    "{:02X} {:02X} {:02X}",
                    self.prg[0], self.prg[1], self.prg[2]
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
            ("Control", format!("${:02X}", self.control)),
            ("IRQ", self.irq.describe()),
        ]
    }
}

/// Mapper 75: Konami VRC1.
#[derive(Clone, Serialize, Deserialize)]
pub struct Vrc1 {
    board: Board,
    prg: [u8; 3],
    chr: [u8; 2],
    control: u8,
}

impl Vrc1 {
    pub fn new(init: MapperInit) -> Self {
        let mut m = Self {
            board: init.board,
            prg: [0; 3],
            chr: [0; 2],
            control: 0,
        };
        m.update();
        m
    }

    fn update(&mut self) {
        let b = &mut self.board;
        for i in 0..3 {
            b.set_prg_rom_8k(1 + i, self.prg[i] as i32);
        }
        b.set_prg_rom_8k(4, -1);
        let hi0 = ((self.control >> 1) & 1) << 4;
        let hi1 = ((self.control >> 2) & 1) << 4;
        b.set_chr_4k(0, (self.chr[0] | hi0) as i32);
        b.set_chr_4k(4, (self.chr[1] | hi1) as i32);
        if b.mirroring != Mirroring::FourScreen {
            b.set_mirroring(if self.control & 1 != 0 {
                Mirroring::Horizontal
            } else {
                Mirroring::Vertical
            });
        }
    }
}

impl Mapper for Vrc1 {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        "VRC1"
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr & 0xF000 {
            0x8000 => self.prg[0] = val & 0x0F,
            0x9000 => self.control = val,
            0xA000 => self.prg[1] = val & 0x0F,
            0xC000 => self.prg[2] = val & 0x0F,
            0xE000 => self.chr[0] = val & 0x0F,
            0xF000 => self.chr[1] = val & 0x0F,
            _ => {
                self.board.write_prg(addr, val);
                return;
            }
        }
        self.update();
    }
}

/// Mapper 73: Konami VRC3 (Salamander), with a 16-bit cycle IRQ counter.
#[derive(Clone, Serialize, Deserialize)]
pub struct Vrc3 {
    board: Board,
    irq_latch: u16,
    irq_counter: u16,
    irq_enabled: bool,
    irq_enable_after_ack: bool,
    irq_8bit: bool,
    irq_pending: bool,
}

impl Vrc3 {
    pub fn new(init: MapperInit) -> Self {
        Self {
            board: init.board,
            irq_latch: 0,
            irq_counter: 0,
            irq_enabled: false,
            irq_enable_after_ack: false,
            irq_8bit: false,
            irq_pending: false,
        }
    }
}

impl Mapper for Vrc3 {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        "VRC3"
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        let nibble =
            |latch: u16, shift: u16| (latch & !(0xF << shift)) | ((val as u16 & 0xF) << shift);
        match addr & 0xF000 {
            0x8000 => self.irq_latch = nibble(self.irq_latch, 0),
            0x9000 => self.irq_latch = nibble(self.irq_latch, 4),
            0xA000 => self.irq_latch = nibble(self.irq_latch, 8),
            0xB000 => self.irq_latch = nibble(self.irq_latch, 12),
            0xC000 => {
                self.irq_pending = false;
                self.irq_enable_after_ack = val & 1 != 0;
                self.irq_enabled = val & 2 != 0;
                self.irq_8bit = val & 4 != 0;
                if self.irq_enabled {
                    self.irq_counter = self.irq_latch;
                }
            }
            0xD000 => {
                self.irq_pending = false;
                self.irq_enabled = self.irq_enable_after_ack;
            }
            0xF000 => self.board.set_prg_rom_16k(0x8000, (val & 0x07) as i32),
            _ => self.board.write_prg(addr, val),
        }
    }

    fn cpu_clock(&mut self) {
        if !self.irq_enabled {
            return;
        }
        if self.irq_8bit {
            let low = (self.irq_counter & 0xFF) as u8;
            if low == 0xFF {
                self.irq_counter = (self.irq_counter & 0xFF00) | (self.irq_latch & 0xFF);
                self.irq_pending = true;
            } else {
                self.irq_counter = (self.irq_counter & 0xFF00) | (low + 1) as u16;
            }
        } else if self.irq_counter == 0xFFFF {
            self.irq_counter = self.irq_latch;
            self.irq_pending = true;
        } else {
            self.irq_counter += 1;
        }
    }

    fn irq(&self) -> bool {
        self.irq_pending
    }
}
