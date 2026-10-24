//! The IRQ counter shared by Konami's VRC4, VRC6 and VRC7.
//!
//! An 8-bit up-counter that reloads from a latch on overflow and raises IRQ.
//! In scanline mode a prescaler divides the CPU clock by 113.667 (341/3) so
//! the counter ticks about once per scanline; in cycle mode it ticks every
//! CPU cycle.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VrcIrq {
    pub latch: u8,
    pub counter: u8,
    prescaler: i16,
    pub enabled: bool,
    enable_after_ack: bool,
    cycle_mode: bool,
    pub pending: bool,
}

impl VrcIrq {
    pub fn write_latch_lo(&mut self, v: u8) {
        self.latch = (self.latch & 0xF0) | (v & 0x0F);
    }

    pub fn write_latch_hi(&mut self, v: u8) {
        self.latch = (self.latch & 0x0F) | ((v & 0x0F) << 4);
    }

    pub fn write_latch(&mut self, v: u8) {
        self.latch = v;
    }

    pub fn write_control(&mut self, v: u8) {
        self.enable_after_ack = v & 1 != 0;
        self.enabled = v & 2 != 0;
        self.cycle_mode = v & 4 != 0;
        if self.enabled {
            self.counter = self.latch;
            self.prescaler = 341;
        }
        self.pending = false;
    }

    pub fn acknowledge(&mut self) {
        self.pending = false;
        self.enabled = self.enable_after_ack;
    }

    pub fn clock(&mut self) {
        if !self.enabled {
            return;
        }
        if self.cycle_mode {
            self.tick();
        } else {
            self.prescaler -= 3;
            if self.prescaler <= 0 {
                self.prescaler += 341;
                self.tick();
            }
        }
    }

    fn tick(&mut self) {
        if self.counter == 0xFF {
            self.counter = self.latch;
            self.pending = true;
        } else {
            self.counter += 1;
        }
    }

    pub fn describe(&self) -> String {
        format!(
            "latch {} counter {} {}{}{}",
            self.latch,
            self.counter,
            if self.enabled { "on" } else { "off" },
            if self.cycle_mode {
                " cycle"
            } else {
                " scanline"
            },
            if self.pending { " PENDING" } else { "" }
        )
    }
}
