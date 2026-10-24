//! Mapper 69: Sunsoft FME-7, and the Sunsoft 5B (FME-7 + a YM2149-style
//! sound generator: three square channels, noise and an envelope).

use super::{Mapper, MapperInit};
use crate::cart::board::{Board, Mirroring, Window};
use serde::{Deserialize, Serialize};

/// Amplitude of each 5B volume level (3 dB steps; level 0 is silent),
/// scaled so a full-volume channel is close to a full APU pulse.
static VOLUMES: std::sync::LazyLock<[f32; 32]> = std::sync::LazyLock::new(volume_table);

fn volume_table() -> [f32; 32] {
    let mut t = [0.0; 32];
    for (i, v) in t.iter_mut().enumerate().skip(1) {
        // 32 envelope steps of 1.5 dB; 4-bit volume v maps to step 2v+1.
        *v = 0.13 * 10f32.powf((i as f32 - 31.0) * 1.5 / 20.0);
    }
    t
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Sunsoft5b {
    regs: [u8; 16],
    address: u8,
    tone_timer: [u16; 3],
    tone_out: [bool; 3],
    noise_timer: u16,
    noise_lfsr: u32,
    env_timer: u32,
    env_step: u8,
    env_holding: bool,
    /// The chip runs at half the CPU clock.
    prescale: bool,
}

impl Sunsoft5b {
    fn write(&mut self, v: u8) {
        let r = (self.address & 0x0F) as usize;
        self.regs[r] = v;
        if r == 13 {
            self.env_step = 0;
            self.env_holding = false;
            self.env_timer = 0;
        }
    }

    fn tone_period(&self, ch: usize) -> u16 {
        (self.regs[ch * 2] as u16 | ((self.regs[ch * 2 + 1] as u16 & 0x0F) << 8)).max(1)
    }

    fn clock(&mut self) {
        self.prescale = !self.prescale;
        if !self.prescale {
            return;
        }
        for ch in 0..3 {
            // Tone toggles every 8 half-rate clocks per unit of period.
            self.tone_timer[ch] += 1;
            if self.tone_timer[ch] >= self.tone_period(ch) * 8 {
                self.tone_timer[ch] = 0;
                self.tone_out[ch] = !self.tone_out[ch];
            }
        }
        self.noise_timer += 1;
        let np = ((self.regs[6] & 0x1F) as u16).max(1) * 8;
        if self.noise_timer >= np * 2 {
            self.noise_timer = 0;
            if self.noise_lfsr == 0 {
                self.noise_lfsr = 1;
            }
            let bit = (self.noise_lfsr ^ (self.noise_lfsr >> 3)) & 1;
            self.noise_lfsr = (self.noise_lfsr >> 1) | (bit << 16);
        }
        let ep = (self.regs[11] as u32 | ((self.regs[12] as u32) << 8)).max(1) * 8;
        self.env_timer += 1;
        if self.env_timer >= ep && !self.env_holding {
            self.env_timer = 0;
            self.env_step += 1;
            if self.env_step >= 32 {
                let shape = self.regs[13];
                let cont = shape & 8 != 0;
                let hold = shape & 1 != 0;
                if !cont || hold {
                    self.env_holding = true;
                    self.env_step = 31;
                } else {
                    self.env_step = 0;
                }
            }
        }
    }

    fn env_level(&self) -> usize {
        let shape = self.regs[13];
        let attack = shape & 4 != 0;
        let alt = shape & 2 != 0;
        let cont = shape & 8 != 0;
        let step = self.env_step as usize;
        if self.env_holding {
            // Hold at the end level (inverted by "alternate" when continuing).
            let end_high = attack ^ (cont && alt);
            return if !cont {
                0
            } else if end_high {
                31
            } else {
                0
            };
        }
        if attack { step } else { 31 - step }
    }

    fn output(&self) -> f32 {
        let vols: &[f32; 32] = &VOLUMES;
        let env = self.env_level();
        let mixer = self.regs[7];
        let noise = self.noise_lfsr & 1 != 0;
        let mut out = 0.0;
        for ch in 0..3 {
            let tone_on = mixer & (1 << ch) == 0;
            let noise_on = mixer & (8 << ch) == 0;
            let gate = (!tone_on || self.tone_out[ch]) && (!noise_on || noise);
            if !gate {
                continue;
            }
            let reg = self.regs[8 + ch];
            let level = if reg & 0x10 != 0 {
                env
            } else if reg & 0x0F == 0 {
                0
            } else {
                (reg as usize & 0x0F) * 2 + 1
            };
            out += vols[level];
        }
        out
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Fme7 {
    board: Board,
    command: u8,
    chr: [u8; 8],
    prg: [u8; 4],
    irq_enabled: bool,
    irq_counter_enabled: bool,
    irq_counter: u16,
    irq_pending: bool,
    audio: Sunsoft5b,
    audio_out: f32,
}

impl Fme7 {
    pub fn new(init: MapperInit) -> Self {
        let mut m = Self {
            board: init.board,
            command: 0,
            chr: [0; 8],
            prg: [0; 4],
            irq_enabled: false,
            irq_counter_enabled: false,
            irq_counter: 0,
            irq_pending: false,
            audio: Sunsoft5b::default(),
            audio_out: 0.0,
        };
        m.update();
        m
    }

    fn update(&mut self) {
        let b = &mut self.board;
        for (i, &bank) in self.chr.iter().enumerate() {
            b.set_chr_1k(i, bank as i32);
        }
        // $6000: ROM, RAM, or nothing.
        let p6 = self.prg[0];
        if p6 & 0x40 == 0 {
            b.set_prg_rom_8k(0, (p6 & 0x3F) as i32);
        } else if p6 & 0x80 != 0 && !b.prg_ram.is_empty() {
            b.set_prg_ram_8k(0, (p6 & 0x3F) as i32);
        } else {
            b.set_prg_window(0, Window::default());
        }
        for i in 1..4 {
            b.set_prg_rom_8k(i, (self.prg[i] & 0x3F) as i32);
        }
        b.set_prg_rom_8k(4, -1);
    }
}

impl Mapper for Fme7 {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        "FME-7 / Sunsoft 5B"
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr & 0xE000 {
            0x6000 => self.board.write_prg(addr, val),
            0x8000 => self.command = val & 0x0F,
            0xA000 => {
                match self.command {
                    0..=7 => self.chr[self.command as usize] = val,
                    8..=0xB => self.prg[(self.command - 8) as usize] = val,
                    0xC => self.board.set_mirroring(match val & 3 {
                        0 => Mirroring::Vertical,
                        1 => Mirroring::Horizontal,
                        2 => Mirroring::SingleScreenA,
                        _ => Mirroring::SingleScreenB,
                    }),
                    0xD => {
                        self.irq_enabled = val & 1 != 0;
                        self.irq_counter_enabled = val & 0x80 != 0;
                        self.irq_pending = false;
                    }
                    0xE => self.irq_counter = (self.irq_counter & 0xFF00) | val as u16,
                    _ => self.irq_counter = (self.irq_counter & 0x00FF) | ((val as u16) << 8),
                }
                self.update();
            }
            0xC000 => self.audio.address = val,
            0xE000 => self.audio.write(val),
            _ => {}
        }
    }

    fn cpu_clock(&mut self) {
        if self.irq_counter_enabled {
            self.irq_counter = self.irq_counter.wrapping_sub(1);
            if self.irq_counter == 0xFFFF && self.irq_enabled {
                self.irq_pending = true;
            }
        }
        self.audio.clock();
        self.audio_out = self.audio.output();
    }

    fn irq(&self) -> bool {
        self.irq_pending
    }

    fn expansion_audio(&self) -> f32 {
        self.audio_out
    }

    fn debug_state(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Command", format!("{:X}", self.command)),
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
            (
                "IRQ",
                format!(
                    "counter {} {} {}",
                    self.irq_counter, self.irq_enabled, self.irq_pending
                ),
            ),
        ]
    }
}
