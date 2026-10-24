//! Mappers 24 and 26: Konami VRC6, with its two pulse channels and sawtooth.

use super::vrc_irq::VrcIrq;
use super::{Mapper, MapperInit};
use crate::cart::board::{Board, Mirroring};
use serde::{Deserialize, Serialize};

/// Output level of one VRC6 volume step, relative to the APU mixer (a VRC6
/// pulse at full volume is about as loud as an APU pulse at full volume).
const VRC6_STEP: f32 = 0.0099;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Pulse {
    volume: u8,
    duty: u8,
    constant: bool,
    period: u16,
    enabled: bool,
    timer: u16,
    step: u8,
}

impl Pulse {
    fn write(&mut self, reg: u16, v: u8) {
        match reg {
            0 => {
                self.constant = v & 0x80 != 0;
                self.duty = (v >> 4) & 7;
                self.volume = v & 0x0F;
            }
            1 => self.period = (self.period & 0xF00) | v as u16,
            _ => {
                self.period = (self.period & 0x0FF) | ((v as u16 & 0x0F) << 8);
                self.enabled = v & 0x80 != 0;
                if !self.enabled {
                    self.step = 0;
                }
            }
        }
    }

    fn clock(&mut self, shift: u8) {
        if !self.enabled {
            return;
        }
        if self.timer == 0 {
            self.timer = self.period >> shift;
            self.step = (self.step + 1) & 15;
        } else {
            self.timer -= 1;
        }
    }

    fn output(&self) -> u8 {
        if self.enabled && (self.constant || self.step <= self.duty) {
            self.volume
        } else {
            0
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Saw {
    rate: u8,
    period: u16,
    enabled: bool,
    timer: u16,
    step: u8,
    acc: u8,
}

impl Saw {
    fn write(&mut self, reg: u16, v: u8) {
        match reg {
            0 => self.rate = v & 0x3F,
            1 => self.period = (self.period & 0xF00) | v as u16,
            _ => {
                self.period = (self.period & 0x0FF) | ((v as u16 & 0x0F) << 8);
                self.enabled = v & 0x80 != 0;
                if !self.enabled {
                    self.acc = 0;
                    self.step = 0;
                }
            }
        }
    }

    fn clock(&mut self, shift: u8) {
        if !self.enabled {
            return;
        }
        if self.timer == 0 {
            self.timer = self.period >> shift;
            // The accumulator gains `rate` on every other step and resets
            // after the 14th.
            self.step += 1;
            if self.step == 14 {
                self.step = 0;
                self.acc = 0;
            } else if self.step & 1 == 0 {
                self.acc = self.acc.wrapping_add(self.rate);
            }
        } else {
            self.timer -= 1;
        }
    }

    fn output(&self) -> u8 {
        if self.enabled { self.acc >> 3 } else { 0 }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Vrc6 {
    board: Board,
    swap_lines: bool,
    prg16: u8,
    prg8: u8,
    chr: [u8; 8],
    control: u8,
    irq: VrcIrq,
    pulse: [Pulse; 2],
    saw: Saw,
    halt: bool,
    shift: u8,
}

impl Vrc6 {
    pub fn new(init: MapperInit) -> Self {
        let mut m = Self {
            board: init.board,
            swap_lines: init.info.mapper == 26,
            prg16: 0,
            prg8: 0,
            chr: [0; 8],
            control: 0,
            irq: VrcIrq::default(),
            pulse: Default::default(),
            saw: Saw::default(),
            halt: false,
            shift: 0,
        };
        m.update();
        m
    }

    fn update(&mut self) {
        let b = &mut self.board;
        b.set_prg_rom_16k(0x8000, self.prg16 as i32);
        b.set_prg_rom_8k(3, self.prg8 as i32);
        b.set_prg_rom_8k(4, -1);
        for (i, &bank) in self.chr.iter().enumerate() {
            b.set_chr_1k(i, bank as i32);
        }
        b.set_mirroring(match (self.control >> 2) & 3 {
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

impl Mapper for Vrc6 {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        "VRC6"
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr < 0x8000 {
            self.board.write_prg(addr, val);
            return;
        }
        let reg = if self.swap_lines {
            ((addr & 1) << 1) | ((addr >> 1) & 1)
        } else {
            addr & 3
        };
        match (addr & 0xF000, reg) {
            (0x8000, _) => self.prg16 = val & 0x0F,
            (0x9000, 3) => {
                self.halt = val & 1 != 0;
                self.shift = if val & 4 != 0 {
                    8
                } else if val & 2 != 0 {
                    4
                } else {
                    0
                };
            }
            (0x9000, r) => self.pulse[0].write(r, val),
            (0xA000, 3) => {}
            (0xA000, r) => self.pulse[1].write(r, val),
            (0xB000, 3) => self.control = val,
            (0xB000, r) => self.saw.write(r, val),
            (0xC000, _) => self.prg8 = val & 0x1F,
            (0xD000, r) => self.chr[r as usize] = val,
            (0xE000, r) => self.chr[4 + r as usize] = val,
            (0xF000, 0) => self.irq.write_latch(val),
            (0xF000, 1) => self.irq.write_control(val),
            (0xF000, 2) => self.irq.acknowledge(),
            _ => {}
        }
        self.update();
    }

    fn cpu_clock(&mut self) {
        self.irq.clock();
        if !self.halt {
            self.pulse[0].clock(self.shift);
            self.pulse[1].clock(self.shift);
            self.saw.clock(self.shift);
        }
    }

    fn irq(&self) -> bool {
        self.irq.pending
    }

    fn expansion_audio(&self) -> f32 {
        let sum = self.pulse[0].output() as f32
            + self.pulse[1].output() as f32
            + self.saw.output() as f32;
        sum * VRC6_STEP
    }

    fn debug_state(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "PRG",
                format!("16K {:02X} 8K {:02X}", self.prg16, self.prg8),
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
            (
                "Audio",
                format!(
                    "pulse {} / {}  saw {}",
                    self.pulse[0].output(),
                    self.pulse[1].output(),
                    self.saw.output()
                ),
            ),
        ]
    }
}
