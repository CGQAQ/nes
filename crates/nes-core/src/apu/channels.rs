//! The five 2A03 sound channels and their shared building blocks.

use serde::{Deserialize, Serialize};

pub const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

const DUTY: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 0, 0, 0],
    [1, 0, 0, 1, 1, 1, 1, 1],
];

const TRIANGLE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    13, 14, 15,
];

pub const NOISE_PERIOD_NTSC: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];
pub const NOISE_PERIOD_PAL: [u16; 16] = [
    4, 8, 14, 30, 60, 88, 118, 148, 188, 236, 354, 472, 708, 944, 1890, 3778,
];
pub const DMC_PERIOD_NTSC: [u16; 16] = [
    428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106, 84, 72, 54,
];
pub const DMC_PERIOD_PAL: [u16; 16] = [
    398, 354, 316, 298, 276, 236, 210, 198, 176, 148, 132, 118, 98, 78, 66, 50,
];

/// Length counter. Writes take effect at the end of the CPU cycle so that a
/// reload on the same cycle as a half-frame clock behaves like hardware: the
/// reload is dropped if the clock changed the counter.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Length {
    pub enabled: bool,
    pub counter: u8,
    halt: bool,
    new_halt: bool,
    reload: u8,
    previous: u8,
}

impl Length {
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.counter = 0;
        }
    }

    pub fn load(&mut self, index: u8) {
        if self.enabled {
            self.reload = LENGTH_TABLE[(index >> 3) as usize];
            self.previous = self.counter;
        }
    }

    pub fn set_halt(&mut self, halt: bool) {
        self.new_halt = halt;
    }

    pub fn clock(&mut self) {
        if self.counter > 0 && !self.halt {
            self.counter -= 1;
        }
    }

    /// End-of-cycle commit of pending writes.
    pub fn commit(&mut self) {
        if self.reload != 0 {
            if self.counter == self.previous {
                self.counter = self.reload;
            }
            self.reload = 0;
        }
        self.halt = self.new_halt;
    }

    pub fn active(&self) -> bool {
        self.counter > 0
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Envelope {
    pub constant: bool,
    pub looping: bool,
    pub volume: u8,
    start: bool,
    divider: u8,
    pub decay: u8,
}

impl Envelope {
    pub fn write(&mut self, val: u8) {
        self.looping = val & 0x20 != 0;
        self.constant = val & 0x10 != 0;
        self.volume = val & 0x0F;
    }

    pub fn restart(&mut self) {
        self.start = true;
    }

    pub fn clock(&mut self) {
        if self.start {
            self.start = false;
            self.decay = 15;
            self.divider = self.volume;
        } else if self.divider == 0 {
            self.divider = self.volume;
            if self.decay > 0 {
                self.decay -= 1;
            } else if self.looping {
                self.decay = 15;
            }
        } else {
            self.divider -= 1;
        }
    }

    pub fn output(&self) -> u8 {
        if self.constant {
            self.volume
        } else {
            self.decay
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Pulse {
    /// Pulse 1 negates with ones' complement, pulse 2 with two's complement.
    second: bool,
    pub duty: u8,
    step: u8,
    pub period: u16,
    timer: u16,
    pub length: Length,
    pub envelope: Envelope,
    sweep_enabled: bool,
    sweep_period: u8,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_reload: bool,
    sweep_divider: u8,
}

impl Pulse {
    pub fn new(second: bool) -> Self {
        Self {
            second,
            ..Default::default()
        }
    }

    pub fn write(&mut self, reg: u16, val: u8) {
        match reg & 3 {
            0 => {
                self.duty = val >> 6;
                self.length.set_halt(val & 0x20 != 0);
                self.envelope.write(val);
            }
            1 => {
                self.sweep_enabled = val & 0x80 != 0;
                self.sweep_period = (val >> 4) & 7;
                self.sweep_negate = val & 0x08 != 0;
                self.sweep_shift = val & 7;
                self.sweep_reload = true;
            }
            2 => self.period = (self.period & 0x700) | val as u16,
            _ => {
                self.period = (self.period & 0x0FF) | ((val as u16 & 7) << 8);
                self.length.load(val);
                self.envelope.restart();
                self.step = 0;
            }
        }
    }

    fn sweep_target(&self) -> u16 {
        let delta = self.period >> self.sweep_shift;
        if self.sweep_negate {
            let neg = if self.second { delta } else { delta + 1 };
            self.period.saturating_sub(neg)
        } else {
            self.period + delta
        }
    }

    fn muted(&self) -> bool {
        self.period < 8 || (!self.sweep_negate && self.sweep_target() > 0x7FF)
    }

    pub fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = (self.period + 1) * 2 - 1;
            self.step = self.step.wrapping_sub(1) & 7;
        } else {
            self.timer -= 1;
        }
    }

    pub fn clock_sweep(&mut self) {
        if self.sweep_divider == 0 && self.sweep_enabled && self.sweep_shift > 0 && !self.muted() {
            self.period = self.sweep_target();
        }
        if self.sweep_divider == 0 || self.sweep_reload {
            self.sweep_divider = self.sweep_period;
            self.sweep_reload = false;
        } else {
            self.sweep_divider -= 1;
        }
    }

    pub fn output(&self) -> u8 {
        if !self.length.active()
            || self.muted()
            || DUTY[self.duty as usize][self.step as usize] == 0
        {
            0
        } else {
            self.envelope.output()
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Triangle {
    pub period: u16,
    timer: u16,
    step: u8,
    pub length: Length,
    control: bool,
    linear_reload_value: u8,
    pub linear: u8,
    linear_reload: bool,
}

impl Triangle {
    pub fn write(&mut self, reg: u16, val: u8) {
        match reg & 3 {
            0 => {
                self.control = val & 0x80 != 0;
                self.length.set_halt(self.control);
                self.linear_reload_value = val & 0x7F;
            }
            1 => {}
            2 => self.period = (self.period & 0x700) | val as u16,
            _ => {
                self.period = (self.period & 0x0FF) | ((val as u16 & 7) << 8);
                self.length.load(val);
                self.linear_reload = true;
            }
        }
    }

    pub fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.period;
            // Periods below 2 are ultrasonic; real TVs filter them, emulators
            // just hold the output to avoid aliasing noise.
            if self.length.active() && self.linear > 0 && self.period >= 2 {
                self.step = (self.step + 1) & 31;
            }
        } else {
            self.timer -= 1;
        }
    }

    pub fn clock_linear(&mut self) {
        if self.linear_reload {
            self.linear = self.linear_reload_value;
        } else if self.linear > 0 {
            self.linear -= 1;
        }
        if !self.control {
            self.linear_reload = false;
        }
    }

    pub fn output(&self) -> u8 {
        TRIANGLE[self.step as usize]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Noise {
    pal: bool,
    pub mode: bool,
    pub period_index: u8,
    timer: u16,
    shift: u16,
    pub length: Length,
    pub envelope: Envelope,
}

impl Noise {
    pub fn new(pal: bool) -> Self {
        Self {
            pal,
            mode: false,
            period_index: 0,
            timer: 0,
            shift: 1,
            length: Length::default(),
            envelope: Envelope::default(),
        }
    }

    fn period(&self) -> u16 {
        let t = if self.pal {
            &NOISE_PERIOD_PAL
        } else {
            &NOISE_PERIOD_NTSC
        };
        t[self.period_index as usize]
    }

    pub fn write(&mut self, reg: u16, val: u8) {
        match reg & 3 {
            0 => {
                self.length.set_halt(val & 0x20 != 0);
                self.envelope.write(val);
            }
            1 => {}
            2 => {
                self.mode = val & 0x80 != 0;
                self.period_index = val & 0x0F;
            }
            _ => {
                self.length.load(val);
                self.envelope.restart();
            }
        }
    }

    pub fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.period() - 1;
            let tap = if self.mode { 6 } else { 1 };
            let feedback = (self.shift ^ (self.shift >> tap)) & 1;
            self.shift = (self.shift >> 1) | (feedback << 14);
        } else {
            self.timer -= 1;
        }
    }

    pub fn output(&self) -> u8 {
        if self.shift & 1 != 0 || !self.length.active() {
            0
        } else {
            self.envelope.output()
        }
    }
}

/// Delta modulation channel. Its memory reader doesn't access memory itself:
/// it raises a DMA request that the bus services by stealing CPU cycles.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dmc {
    pal: bool,
    pub irq_enabled: bool,
    pub looping: bool,
    pub rate_index: u8,
    timer: u16,
    pub output_level: u8,
    pub sample_addr: u16,
    pub sample_len: u16,
    pub current_addr: u16,
    pub bytes_remaining: u16,
    buffer: Option<u8>,
    shift: u8,
    bits_remaining: u8,
    silence: bool,
    pub irq: bool,
    /// The reader wants a byte (bus should start a DMA).
    pub(crate) dma_pending: bool,
    /// CPU cycles until a DMA triggered by enabling the channel starts.
    start_delay: u8,
}

impl Dmc {
    pub fn new(pal: bool) -> Self {
        Self {
            pal,
            irq_enabled: false,
            looping: false,
            rate_index: 0,
            timer: 0,
            output_level: 0,
            sample_addr: 0xC000,
            sample_len: 1,
            current_addr: 0xC000,
            bytes_remaining: 0,
            buffer: None,
            shift: 0,
            bits_remaining: 8,
            silence: true,
            irq: false,
            dma_pending: false,
            start_delay: 0,
        }
    }

    fn period(&self) -> u16 {
        let t = if self.pal {
            &DMC_PERIOD_PAL
        } else {
            &DMC_PERIOD_NTSC
        };
        t[self.rate_index as usize]
    }

    pub fn write(&mut self, reg: u16, val: u8) {
        match reg & 3 {
            0 => {
                self.irq_enabled = val & 0x80 != 0;
                self.looping = val & 0x40 != 0;
                self.rate_index = val & 0x0F;
                if !self.irq_enabled {
                    self.irq = false;
                }
            }
            1 => self.output_level = val & 0x7F,
            2 => self.sample_addr = 0xC000 | ((val as u16) << 6),
            _ => self.sample_len = ((val as u16) << 4) | 1,
        }
    }

    /// `$4015` bit 4. `odd_cycle` picks the 2- or 3-cycle start delay.
    pub fn set_enabled(&mut self, on: bool, odd_cycle: bool) {
        if !on {
            self.bytes_remaining = 0;
            self.dma_pending = false;
            self.start_delay = 0;
        } else if self.bytes_remaining == 0 {
            self.restart();
            if self.buffer.is_none() {
                self.start_delay = if odd_cycle { 3 } else { 2 };
            }
        }
    }

    fn restart(&mut self) {
        self.current_addr = self.sample_addr;
        self.bytes_remaining = self.sample_len;
    }

    pub fn clock_timer(&mut self) {
        if self.start_delay > 0 {
            self.start_delay -= 1;
            if self.start_delay == 0 && self.buffer.is_none() && self.bytes_remaining > 0 {
                self.dma_pending = true;
            }
        }
        if self.timer == 0 {
            self.timer = self.period() - 1;
            self.clock_output();
        } else {
            self.timer -= 1;
        }
    }

    fn clock_output(&mut self) {
        if !self.silence {
            if self.shift & 1 != 0 {
                if self.output_level <= 125 {
                    self.output_level += 2;
                }
            } else if self.output_level >= 2 {
                self.output_level -= 2;
            }
        }
        self.shift >>= 1;
        self.bits_remaining -= 1;
        if self.bits_remaining == 0 {
            self.bits_remaining = 8;
            match self.buffer.take() {
                Some(b) => {
                    self.silence = false;
                    self.shift = b;
                    if self.bytes_remaining > 0 {
                        self.dma_pending = true;
                    }
                }
                None => self.silence = true,
            }
        }
    }

    pub fn dma_address(&self) -> u16 {
        self.current_addr
    }

    /// The bus finished the DMA read.
    pub fn dma_complete(&mut self, val: u8) {
        if self.bytes_remaining == 0 {
            return;
        }
        self.buffer = Some(val);
        self.current_addr = if self.current_addr == 0xFFFF {
            0x8000
        } else {
            self.current_addr + 1
        };
        self.bytes_remaining -= 1;
        if self.bytes_remaining == 0 {
            if self.looping {
                self.restart();
            } else if self.irq_enabled {
                self.irq = true;
            }
        }
    }

    pub fn output(&self) -> u8 {
        self.output_level
    }
}
