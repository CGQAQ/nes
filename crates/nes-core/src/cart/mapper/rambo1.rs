//! Mapper 64: Tengen RAMBO-1. An MMC3 relative with extra CHR/PRG
//! registers, a 1K CHR mode, and an IRQ counter that can count CPU cycles.

use super::{Mapper, MapperInit};
use crate::cart::board::{Board, Mirroring};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Rambo1 {
    board: Board,
    select: u8,
    regs: [u8; 16],
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_pending: bool,
    cycle_mode: bool,
    prescaler: u8,
    a12: bool,
    a12_low: u8,
}

impl Rambo1 {
    pub fn new(init: MapperInit) -> Self {
        let mut m = Self {
            board: init.board,
            select: 0,
            regs: [0; 16],
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_pending: false,
            cycle_mode: false,
            prescaler: 0,
            a12: false,
            a12_low: 0,
        };
        m.update();
        m
    }

    fn update(&mut self) {
        let r = self.regs;
        let b = &mut self.board;
        let invert = if self.select & 0x80 != 0 { 4 } else { 0 };
        let banks = if self.select & 0x20 != 0 {
            [r[0], r[8], r[1], r[9], r[2], r[3], r[4], r[5]]
        } else {
            [
                r[0] & 0xFE,
                r[0] | 1,
                r[1] & 0xFE,
                r[1] | 1,
                r[2],
                r[3],
                r[4],
                r[5],
            ]
        };
        for (slot, &bank) in banks.iter().enumerate() {
            b.set_chr_1k(slot ^ invert, bank as i32);
        }
        if self.select & 0x40 != 0 {
            b.set_prg_rom_8k(1, r[15] as i32);
            b.set_prg_rom_8k(2, r[6] as i32);
            b.set_prg_rom_8k(3, r[7] as i32);
        } else {
            b.set_prg_rom_8k(1, r[6] as i32);
            b.set_prg_rom_8k(2, r[7] as i32);
            b.set_prg_rom_8k(3, r[15] as i32);
        }
        b.set_prg_rom_8k(4, -1);
    }

    fn clock_irq(&mut self) {
        if self.irq_reload {
            // RAMBO-1 reloads with latch + 1 when the latch is nonzero.
            self.irq_counter = if self.irq_latch == 0 {
                0
            } else {
                self.irq_latch | 1
            };
            self.irq_reload = false;
        } else if self.irq_counter == 0 {
            self.irq_counter = self.irq_latch;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 && self.irq_enabled {
            self.irq_pending = true;
        }
    }

    fn watch_a12(&mut self, addr: u16) {
        let a12 = addr & 0x1000 != 0;
        if !self.cycle_mode && a12 && !self.a12 && self.a12_low >= 4 {
            self.clock_irq();
        }
        if a12 {
            self.a12_low = 0;
        }
        self.a12 = a12;
    }
}

impl Mapper for Rambo1 {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        "Tengen RAMBO-1"
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr < 0x8000 {
            self.board.write_prg(addr, val);
            return;
        }
        match addr & 0xE001 {
            0x8000 => self.select = val,
            0x8001 => self.regs[(self.select & 0x0F) as usize] = val,
            0xA000 => self.board.set_mirroring(if val & 1 != 0 {
                Mirroring::Horizontal
            } else {
                Mirroring::Vertical
            }),
            0xC000 => self.irq_latch = val,
            0xC001 => {
                self.cycle_mode = val & 1 != 0;
                self.irq_reload = true;
                self.prescaler = 0;
            }
            0xE000 => {
                self.irq_enabled = false;
                self.irq_pending = false;
            }
            0xE001 => self.irq_enabled = true,
            _ => {}
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
        if !self.a12 {
            self.a12_low = self.a12_low.saturating_add(1);
        }
        if self.cycle_mode {
            self.prescaler = (self.prescaler + 1) & 3;
            if self.prescaler == 0 {
                self.clock_irq();
            }
        }
    }

    fn irq(&self) -> bool {
        self.irq_pending
    }
}
