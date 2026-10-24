//! 2A03 audio processing unit.
//!
//! Clocked once per CPU cycle. The mixed output (with any cartridge expansion
//! audio) is box-filtered down to the host sample rate and then run through
//! the same analog filter chain as the NES's output stage.

pub mod channels;

use crate::region::Region;
use channels::{Dmc, Noise, Pulse, Triangle};
use serde::{Deserialize, Serialize};

/// Channel order used by mute flags and the oscilloscope.
pub const CHANNEL_NAMES: [&str; 5] = ["Pulse 1", "Pulse 2", "Triangle", "Noise", "DMC"];
pub const SCOPE_LEN: usize = 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FrameCounter {
    five_step: bool,
    irq_inhibit: bool,
    cycle: u32,
    step: usize,
    pending: Option<u8>,
    pending_delay: u8,
}

/// First-order filters as on the NES mainboard: two high-passes (90 Hz and
/// 440 Hz) and a 14 kHz low-pass.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct OutputFilter {
    hp1_a: f32,
    hp2_a: f32,
    lp_b: f32,
    hp1_prev_in: f32,
    hp1_out: f32,
    hp2_prev_in: f32,
    hp2_out: f32,
    lp_out: f32,
}

impl OutputFilter {
    fn new(sample_rate: f64) -> Self {
        let dt = 1.0 / sample_rate;
        let hp = |f: f64| {
            let rc = 1.0 / (2.0 * std::f64::consts::PI * f);
            (rc / (rc + dt)) as f32
        };
        let rc_lp = 1.0 / (2.0 * std::f64::consts::PI * 14_000.0);
        Self {
            hp1_a: hp(90.0),
            hp2_a: hp(440.0),
            lp_b: (dt / (rc_lp + dt)) as f32,
            ..Default::default()
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        self.hp1_out = self.hp1_a * (self.hp1_out + x - self.hp1_prev_in);
        self.hp1_prev_in = x;
        let y = self.hp1_out;
        self.hp2_out = self.hp2_a * (self.hp2_out + y - self.hp2_prev_in);
        self.hp2_prev_in = y;
        let z = self.hp2_out;
        self.lp_out += self.lp_b * (z - self.lp_out);
        self.lp_out
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Apu {
    region: Region,
    pub pulse1: Pulse,
    pub pulse2: Pulse,
    pub triangle: Triangle,
    pub noise: Noise,
    pub dmc: Dmc,
    frame: FrameCounter,
    pub frame_irq: bool,
    cycle: u64,
    last_4017: u8,

    /// Per-channel mute switches (debugging aid; not part of emulation).
    #[serde(skip)]
    pub mute: [bool; 5],

    sample_rate: f64,
    cycles_per_sample: f64,
    sample_phase: f64,
    acc: f32,
    acc_count: u32,
    filter: OutputFilter,
    #[serde(skip)]
    samples: Vec<f32>,
    #[serde(skip, default = "empty_scope")]
    scope: Vec<[u8; 5]>,
    #[serde(skip)]
    scope_pos: usize,
}

fn empty_scope() -> Vec<[u8; 5]> {
    vec![[0; 5]; SCOPE_LEN]
}

const NTSC_STEPS: [[u32; 6]; 2] = [
    [7457, 14913, 22371, 29828, 29829, 29830],
    [7457, 14913, 22371, 29829, 37281, 37282],
];
const PAL_STEPS: [[u32; 6]; 2] = [
    [8313, 16627, 24939, 33252, 33253, 33254],
    [8313, 16627, 24939, 33253, 41565, 41566],
];

/// Non-linear DAC lookup tables (nesdev "APU Mixer", lookup-table form).
const PULSE_TABLE: [f32; 31] = {
    let mut t = [0.0; 31];
    let mut n = 1;
    while n < 31 {
        t[n] = 95.52 / (8128.0 / n as f32 + 100.0);
        n += 1;
    }
    t
};

const TND_TABLE: [f32; 203] = {
    let mut t = [0.0; 203];
    let mut n = 1;
    while n < 203 {
        t[n] = 163.67 / (24329.0 / n as f32 + 100.0);
        n += 1;
    }
    t
};

impl Apu {
    pub fn new(region: Region, sample_rate: f64) -> Self {
        let pal = region == Region::Pal;
        let mut apu = Self {
            region,
            pulse1: Pulse::new(false),
            pulse2: Pulse::new(true),
            triangle: Triangle::default(),
            noise: Noise::new(pal),
            dmc: Dmc::new(pal),
            frame: FrameCounter {
                five_step: false,
                irq_inhibit: false,
                cycle: 0,
                step: 0,
                pending: None,
                pending_delay: 0,
            },
            frame_irq: false,
            cycle: 0,
            last_4017: 0,
            mute: [false; 5],
            sample_rate,
            cycles_per_sample: region.cpu_clock_hz() / sample_rate,
            sample_phase: 0.0,
            acc: 0.0,
            acc_count: 0,
            filter: OutputFilter::new(sample_rate),
            samples: Vec::with_capacity(2048),
            scope: empty_scope(),
            scope_pos: 0,
        };
        // Power-up behaves as if $4017 were written with 0 just before the
        // reset vector is fetched.
        apu.write_frame_counter(0);
        apu
    }

    pub fn reset(&mut self) {
        self.write_status(0);
        self.frame_irq = false;
        self.dmc.irq = false;
        // RESET re-applies the last sequencer mode (IRQ inhibit is cleared),
        // with the same delay as a $4017 write.
        self.frame.irq_inhibit = false;
        self.frame.pending = Some(self.last_4017 & 0x80);
        self.frame.pending_delay = 3;
    }

    /// Changes the output rate; `nudge` (e.g. 1.002) lets frontends trim the
    /// rate slightly to keep their audio buffer level stable.
    pub fn set_sample_rate(&mut self, rate: f64, nudge: f64) {
        if (rate - self.sample_rate).abs() > f64::EPSILON {
            self.sample_rate = rate;
            self.filter = OutputFilter::new(rate);
        }
        self.cycles_per_sample = self.region.cpu_clock_hz() / (rate * nudge);
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn drain_samples(&mut self, out: &mut Vec<f32>) {
        out.append(&mut self.samples);
    }

    pub fn clear_samples(&mut self) {
        self.samples.clear();
    }

    /// Last [`SCOPE_LEN`] output samples of each channel (raw 0-15 / 0-127
    /// levels), oldest first.
    pub fn scope(&self) -> impl Iterator<Item = [u8; 5]> + '_ {
        let (a, b) = self.scope.split_at(self.scope_pos);
        b.iter().chain(a.iter()).copied()
    }

    pub fn irq_line(&self) -> bool {
        self.frame_irq || self.dmc.irq
    }

    pub(crate) fn cycle_is_odd(&self) -> bool {
        self.cycle & 1 == 1
    }

    // ------------------------------------------------------------------

    fn quarter_frame(&mut self) {
        self.pulse1.envelope.clock();
        self.pulse2.envelope.clock();
        self.noise.envelope.clock();
        self.triangle.clock_linear();
    }

    fn half_frame(&mut self) {
        self.pulse1.length.clock();
        self.pulse2.length.clock();
        self.triangle.length.clock();
        self.noise.length.clock();
        self.pulse1.clock_sweep();
        self.pulse2.clock_sweep();
    }

    fn clock_frame_counter(&mut self) {
        self.frame.cycle += 1;
        let steps = if self.region == Region::Pal {
            &PAL_STEPS
        } else {
            &NTSC_STEPS
        };
        let mode = self.frame.five_step as usize;
        if self.frame.cycle == steps[mode][self.frame.step] {
            let step = self.frame.step;
            if self.frame.five_step {
                match step {
                    0 | 2 => self.quarter_frame(),
                    1 | 4 => {
                        self.quarter_frame();
                        self.half_frame();
                    }
                    _ => {}
                }
            } else {
                match step {
                    0 | 2 => self.quarter_frame(),
                    1 => {
                        self.quarter_frame();
                        self.half_frame();
                    }
                    3 | 5 => self.set_frame_irq(),
                    4 => {
                        self.quarter_frame();
                        self.half_frame();
                        self.set_frame_irq();
                    }
                    _ => {}
                }
            }
            self.frame.step += 1;
            if self.frame.step == 6 {
                self.frame.step = 0;
                self.frame.cycle = 0;
            }
        }

        // A $4017 write restarts the sequence after its 3-4 cycle delay.
        if let Some(val) = self.frame.pending {
            self.frame.pending_delay -= 1;
            if self.frame.pending_delay == 0 {
                self.frame.pending = None;
                self.frame.five_step = val & 0x80 != 0;
                self.frame.cycle = 0;
                self.frame.step = 0;
                if self.frame.five_step {
                    self.quarter_frame();
                    self.half_frame();
                }
            }
        }
    }

    fn set_frame_irq(&mut self) {
        if !self.frame.irq_inhibit {
            self.frame_irq = true;
        }
    }

    /// One CPU cycle. `expansion` is the cartridge's audio output this cycle.
    pub fn tick(&mut self, expansion: f32) {
        self.clock_frame_counter();
        self.pulse1.clock_timer();
        self.pulse2.clock_timer();
        self.triangle.clock_timer();
        self.noise.clock_timer();
        self.dmc.clock_timer();

        self.pulse1.length.commit();
        self.pulse2.length.commit();
        self.triangle.length.commit();
        self.noise.length.commit();
        self.cycle += 1;

        let levels = [
            self.pulse1.output(),
            self.pulse2.output(),
            self.triangle.output(),
            self.noise.output(),
            self.dmc.output(),
        ];
        let m = |i: usize| if self.mute[i] { 0 } else { levels[i] };
        let tnd = 3 * m(2) as usize + 2 * m(3) as usize + m(4) as usize;
        let sample = PULSE_TABLE[(m(0) + m(1)) as usize] + TND_TABLE[tnd] + expansion;
        self.acc += sample;
        self.acc_count += 1;
        self.sample_phase += 1.0;
        if self.sample_phase >= self.cycles_per_sample {
            self.sample_phase -= self.cycles_per_sample;
            let avg = self.acc / self.acc_count as f32;
            self.acc = 0.0;
            self.acc_count = 0;
            let out = self.filter.process(avg);
            self.samples.push(out);
            self.scope[self.scope_pos] = levels;
            self.scope_pos = (self.scope_pos + 1) % SCOPE_LEN;
        }
    }

    // ------------------------------------------------------------------
    // Registers.

    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x4000..=0x4003 => self.pulse1.write(addr, val),
            0x4004..=0x4007 => self.pulse2.write(addr, val),
            0x4008..=0x400B => self.triangle.write(addr, val),
            0x400C..=0x400F => self.noise.write(addr, val),
            0x4010..=0x4013 => self.dmc.write(addr, val),
            0x4015 => self.write_status(val),
            0x4017 => self.write_frame_counter(val),
            _ => {}
        }
    }

    fn write_status(&mut self, val: u8) {
        self.pulse1.length.set_enabled(val & 0x01 != 0);
        self.pulse2.length.set_enabled(val & 0x02 != 0);
        self.triangle.length.set_enabled(val & 0x04 != 0);
        self.noise.length.set_enabled(val & 0x08 != 0);
        let odd = self.cycle_is_odd();
        self.dmc.set_enabled(val & 0x10 != 0, odd);
        self.dmc.irq = false;
    }

    fn write_frame_counter(&mut self, val: u8) {
        self.last_4017 = val;
        self.frame.irq_inhibit = val & 0x40 != 0;
        if self.frame.irq_inhibit {
            self.frame_irq = false;
        }
        // The new mode takes effect 3 or 4 CPU cycles later, depending on
        // whether the write lands on an APU cycle boundary.
        self.frame.pending = Some(val);
        self.frame.pending_delay = if self.cycle_is_odd() { 4 } else { 3 };
    }

    /// `$4015` read. Returns bits 0-4, 6 and 7; bit 5 is open bus.
    pub fn read_status(&mut self) -> u8 {
        let v = self.peek_status();
        self.frame_irq = false;
        v
    }

    pub fn peek_status(&self) -> u8 {
        (self.pulse1.length.active() as u8)
            | (self.pulse2.length.active() as u8) << 1
            | (self.triangle.length.active() as u8) << 2
            | (self.noise.length.active() as u8) << 3
            | ((self.dmc.bytes_remaining > 0) as u8) << 4
            | (self.frame_irq as u8) << 6
            | (self.dmc.irq as u8) << 7
    }

    /// Frame counter state for the debugger: (five-step mode, cycle, IRQ inhibit).
    pub fn frame_counter_state(&self) -> (bool, u32, bool) {
        (
            self.frame.five_step,
            self.frame.cycle,
            self.frame.irq_inhibit,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_irq_fires_in_four_step_mode() {
        let mut apu = Apu::new(Region::Ntsc, 48_000.0);
        let mut cycles = 0;
        while !apu.frame_irq {
            apu.tick(0.0);
            cycles += 1;
            assert!(cycles < 40_000);
        }
        // 3-4 cycles of $4017 write delay plus 29828.
        assert!((29_830..=29_833).contains(&cycles), "{cycles}");
    }

    #[test]
    fn produces_samples_at_the_requested_rate() {
        let mut apu = Apu::new(Region::Ntsc, 48_000.0);
        for _ in 0..1_789_773 {
            apu.tick(0.0);
        }
        let mut out = Vec::new();
        apu.drain_samples(&mut out);
        assert!((47_990..=48_010).contains(&out.len()), "{}", out.len());
    }

    #[test]
    fn length_counter_silences_pulse() {
        let mut apu = Apu::new(Region::Ntsc, 48_000.0);
        apu.write(0x4015, 0x01);
        apu.write(0x4000, 0x3F); // constant volume 15, halt off... (bit5 = halt)
        apu.write(0x4000, 0x1F); // constant volume, length counter enabled
        apu.write(0x4002, 0x80);
        apu.write(0x4003, 0x08); // length index 1 -> 254
        apu.tick(0.0);
        assert!(apu.peek_status() & 1 != 0);
    }
}
