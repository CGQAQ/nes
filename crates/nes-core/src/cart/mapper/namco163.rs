//! Mapper 19: Namco 129/163 (wavetable audio, 15-bit IRQ counter, CHR banks
//! that can point at nametable RAM), and mapper 210: Namco 175/340 (the same
//! banking without audio or IRQ).

use super::{Mapper, MapperInit};
use crate::cart::board::{Board, Mirroring};
use serde::{Deserialize, Serialize};

/// A full-volume channel swings about 0.27, roughly twice an APU pulse,
/// which matches how loud N163 games are relative to the 2A03.
const AUDIO_GAIN: f32 = 0.0012;

#[derive(Clone, Serialize, Deserialize)]
pub struct Namco163 {
    board: Board,
    /// 210: Namco 175 (submapper 1) / 340 (submapper 2), no audio or IRQ.
    lite: bool,
    n340: bool,
    chr: [u8; 8],
    nt: [u8; 4],
    prg: [u8; 3],
    chr_ram_disable: [bool; 2],
    sound_disable: bool,
    /// 128 bytes of internal RAM: wavetables and channel registers.
    ram: Vec<u8>,
    ram_addr: u8,
    auto_inc: bool,
    irq_counter: u16,
    irq_enabled: bool,
    irq_pending: bool,
    audio_divider: u8,
    audio_channel: u8,
    channel_out: [f32; 8],
    output: f32,
}

impl Namco163 {
    pub fn new(init: MapperInit) -> Self {
        let lite = init.info.mapper == 210;
        let mut m = Self {
            board: init.board,
            lite,
            n340: lite && init.info.submapper == 2,
            chr: [0; 8],
            nt: [0xE0, 0xE1, 0xE0, 0xE1],
            prg: [0, 1, 2],
            chr_ram_disable: [false; 2],
            sound_disable: false,
            ram: vec![0; 128],
            ram_addr: 0,
            auto_inc: false,
            irq_counter: 0,
            irq_enabled: false,
            irq_pending: false,
            audio_divider: 0,
            audio_channel: 0,
            channel_out: [0.0; 8],
            output: 0.0,
        };
        m.update();
        m
    }

    fn update(&mut self) {
        let b = &mut self.board;
        for i in 0..3 {
            b.set_prg_rom_8k(1 + i, (self.prg[i] & 0x3F) as i32);
        }
        b.set_prg_rom_8k(4, -1);
        for (i, &v) in self.chr.iter().enumerate() {
            // Values $E0+ select nametable RAM, unless disabled for this half.
            if !self.lite && v >= 0xE0 && !self.chr_ram_disable[i / 4] {
                b.set_chr_ciram_1k(i, (v & 1) as i32);
            } else {
                b.set_chr_1k(i, v as i32);
            }
        }
        if !self.lite {
            for (i, &v) in self.nt.iter().enumerate() {
                if v >= 0xE0 {
                    b.set_nt_ciram(i, (v & 1) as u32);
                } else {
                    b.set_nt_chr_rom(i, v as i32);
                }
            }
        }
    }

    fn ram_read(&mut self) -> u8 {
        let v = self.ram[(self.ram_addr & 0x7F) as usize];
        if self.auto_inc {
            self.ram_addr = (self.ram_addr + 1) & 0x7F;
        }
        v
    }

    fn clock_audio(&mut self) {
        self.audio_divider += 1;
        if self.audio_divider < 15 {
            return;
        }
        self.audio_divider = 0;
        let enabled = ((self.ram[0x7F] >> 4) & 7) + 1;
        // Channels are serviced round-robin from channel 7 downward.
        let ch = 7 - self.audio_channel;
        self.audio_channel = (self.audio_channel + 1) % enabled;
        let base = 0x40 + ch as usize * 8;
        let r = &self.ram;
        let freq =
            r[base] as u32 | ((r[base + 2] as u32) << 8) | (((r[base + 4] & 3) as u32) << 16);
        let mut phase =
            r[base + 1] as u32 | ((r[base + 3] as u32) << 8) | ((r[base + 5] as u32) << 16);
        let length = 256 - (r[base + 4] & 0xFC) as u32;
        let wave_addr = r[base + 6] as u32;
        let volume = (r[base + 7] & 0x0F) as f32;
        phase = (phase + freq) % (length << 16);
        let sample_addr = ((phase >> 16) + wave_addr) & 0xFF;
        let byte = self.ram[(sample_addr >> 1) as usize & 0x7F];
        let nibble = if sample_addr & 1 == 0 {
            byte & 0x0F
        } else {
            byte >> 4
        };
        self.ram[base + 1] = phase as u8;
        self.ram[base + 3] = (phase >> 8) as u8;
        self.ram[base + 5] = (phase >> 16) as u8;
        self.channel_out[ch as usize] = (nibble as f32 - 8.0) * volume;
        // The chip time-multiplexes channels; average the active ones.
        let sum: f32 = (8 - enabled as usize..8).map(|c| self.channel_out[c]).sum();
        self.output = sum / enabled as f32 * AUDIO_GAIN;
    }
}

impl Mapper for Namco163 {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        if self.lite {
            "Namco 175/340"
        } else {
            "Namco 163"
        }
    }

    fn cpu_read(&mut self, addr: u16) -> Option<u8> {
        if self.lite {
            return self.board.read_prg(addr);
        }
        match addr & 0xF800 {
            0x4800 => Some(self.ram_read()),
            0x5000 => Some(self.irq_counter as u8),
            0x5800 => {
                Some(((self.irq_counter >> 8) as u8 & 0x7F) | ((self.irq_enabled as u8) << 7))
            }
            _ => self.board.read_prg(addr),
        }
    }

    fn cpu_peek(&self, addr: u16) -> Option<u8> {
        if !self.lite && addr & 0xF800 == 0x4800 {
            return Some(self.ram[(self.ram_addr & 0x7F) as usize]);
        }
        self.board.read_prg(addr)
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        let lite = self.lite;
        match addr & 0xF800 {
            0x4800 if !lite => {
                self.ram[(self.ram_addr & 0x7F) as usize] = val;
                if self.auto_inc {
                    self.ram_addr = (self.ram_addr + 1) & 0x7F;
                }
            }
            0x5000 if !lite => {
                self.irq_counter = (self.irq_counter & 0x7F00) | val as u16;
                self.irq_pending = false;
            }
            0x5800 if !lite => {
                self.irq_counter = (self.irq_counter & 0x00FF) | ((val as u16 & 0x7F) << 8);
                self.irq_enabled = val & 0x80 != 0;
                self.irq_pending = false;
            }
            0x8000..=0xB800 => {
                self.chr[((addr - 0x8000) >> 11) as usize] = val;
                self.update();
            }
            0xC000..=0xD800 if !lite => {
                self.nt[((addr - 0xC000) >> 11) as usize] = val;
                self.update();
            }
            0xE000 => {
                self.prg[0] = val & 0x3F;
                self.sound_disable = val & 0x40 != 0;
                if self.n340 {
                    self.board.set_mirroring(match val >> 6 {
                        0 => Mirroring::SingleScreenA,
                        1 => Mirroring::Vertical,
                        2 => Mirroring::Horizontal,
                        _ => Mirroring::SingleScreenB,
                    });
                }
                self.update();
            }
            0xE800 => {
                self.prg[1] = val & 0x3F;
                self.chr_ram_disable = [val & 0x40 != 0, val & 0x80 != 0];
                self.update();
            }
            0xF000 => {
                self.prg[2] = val & 0x3F;
                self.update();
            }
            0xF800 if !lite => {
                self.ram_addr = val & 0x7F;
                self.auto_inc = val & 0x80 != 0;
            }
            _ => self.board.write_prg(addr, val),
        }
    }

    fn cpu_clock(&mut self) {
        if self.lite {
            return;
        }
        if self.irq_enabled && self.irq_counter < 0x7FFF {
            self.irq_counter += 1;
            if self.irq_counter == 0x7FFF {
                self.irq_pending = true;
            }
        }
        if !self.sound_disable {
            self.clock_audio();
        }
    }

    fn irq(&self) -> bool {
        self.irq_pending
    }

    fn expansion_audio(&self) -> f32 {
        if self.sound_disable { 0.0 } else { self.output }
    }

    fn save_data(&self) -> Option<Vec<u8>> {
        let b = &self.board;
        b.battery.then(|| {
            // Some games keep save data in the internal RAM too.
            let mut v = b.prg_ram.clone();
            v.extend_from_slice(&self.ram);
            v
        })
    }

    fn load_save_data(&mut self, data: &[u8]) {
        let n = self.board.prg_ram.len().min(data.len());
        self.board.prg_ram[..n].copy_from_slice(&data[..n]);
        if data.len() >= n + 128 {
            self.ram.copy_from_slice(&data[n..n + 128]);
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
            (
                "NT",
                self.nt
                    .iter()
                    .map(|c| format!("{c:02X}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            (
                "IRQ",
                format!(
                    "counter {:04X} {} {}",
                    self.irq_counter, self.irq_enabled, self.irq_pending
                ),
            ),
            (
                "Audio channels",
                format!("{}", ((self.ram[0x7F] >> 4) & 7) + 1),
            ),
        ]
    }
}
