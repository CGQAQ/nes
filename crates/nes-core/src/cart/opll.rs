//! VRC7 sound: a cut-down Yamaha YM2413 (OPLL) with six two-operator FM
//! channels and its own instrument ROM.
//!
//! This is a behavioral model rather than a gate-level one: phase and
//! envelope generators follow the OPLL's structure and rate scaling, and the
//! waveform math is done in floating point instead of log-sin/exp tables.
//! It produces one sample every 36 CPU cycles (the chip's 49.7 kHz rate).

use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

/// The 15 built-in instruments (patch 0 is the user-defined one), as dumped
/// from a VRC7.
const PATCHES: [[u8; 8]; 16] = [
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    [0x03, 0x21, 0x05, 0x06, 0xE8, 0x81, 0x42, 0x27], // Buzzy bell
    [0x13, 0x41, 0x14, 0x0D, 0xD8, 0xF6, 0x23, 0x12], // Guitar
    [0x11, 0x11, 0x08, 0x08, 0xFA, 0xB2, 0x20, 0x12], // Wurly
    [0x31, 0x61, 0x0C, 0x07, 0xA8, 0x64, 0x61, 0x27], // Flute
    [0x32, 0x21, 0x1E, 0x06, 0xE1, 0x76, 0x01, 0x28], // Clarinet
    [0x02, 0x01, 0x06, 0x00, 0xA3, 0xE2, 0xF4, 0xF4], // Synth
    [0x21, 0x61, 0x1D, 0x07, 0x82, 0x81, 0x11, 0x07], // Trumpet
    [0x23, 0x21, 0x22, 0x17, 0xA2, 0x72, 0x01, 0x17], // Organ
    [0x35, 0x11, 0x25, 0x00, 0x40, 0x73, 0x72, 0x01], // Bells
    [0xB5, 0x01, 0x0F, 0x0F, 0xA8, 0xA5, 0x51, 0x02], // Vibes
    [0x17, 0xC1, 0x24, 0x07, 0xF8, 0xF8, 0x22, 0x12], // Vibraphone
    [0x71, 0x23, 0x11, 0x06, 0x65, 0x74, 0x18, 0x16], // Tutti
    [0x01, 0x02, 0xD3, 0x05, 0xC9, 0x95, 0x03, 0x02], // Fretless
    [0x61, 0x63, 0x0C, 0x00, 0x94, 0xC0, 0x33, 0xF6], // Synth bass
    [0x21, 0x72, 0x0D, 0x00, 0xC1, 0xD5, 0x56, 0x06], // Sweep
];

/// Frequency multiplier ×2, indexed by the 4-bit MULT field.
const MULT_X2: [u32; 16] = [1, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 20, 24, 24, 30, 30];

/// Key-scale-level base attenuation in dB by F-number's top 4 bits.
const KSL_DB: [f32; 16] = [
    0.0, 18.0, 24.0, 27.75, 30.0, 32.25, 33.75, 35.25, 36.0, 37.5, 38.25, 39.0, 39.75, 40.5, 41.25,
    42.0,
];

/// Feedback modulation depth (radians) by the 3-bit FB field.
const FEEDBACK: [f32; 8] = [
    0.0,
    PI / 16.0,
    PI / 8.0,
    PI / 4.0,
    PI / 2.0,
    PI,
    2.0 * PI,
    4.0 * PI,
];

const SAMPLE_RATE: f32 = 49_716.0;
/// Envelope attenuation units: 0.375 dB, 0-127 (48 dB) like the chip.
const EG_MAX: f32 = 127.0;
const CHANNEL_GAIN: f32 = 0.06;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum EgState {
    Attack,
    Decay,
    Sustain,
    Release,
    #[default]
    Off,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Operator {
    phase: u32,
    eg: f32,
    state: EgState,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Channel {
    fnum: u16,
    block: u8,
    key: bool,
    sustain: bool,
    instrument: u8,
    volume: u8,
    ops: [Operator; 2],
    feedback: [f32; 2],
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Opll {
    custom: [u8; 8],
    address: u8,
    channels: [Channel; 6],
    lfo_am: f32,
    lfo_pm: f32,
    divider: u8,
    output: f32,
}

/// Per-sample envelope change (in EG units) for an effective rate 0-63:
/// one step every 2^(13 - rate/4) samples, with (rate & 3) adding 25% steps.
fn eg_step(rate: u32) -> f32 {
    if rate < 4 {
        return 0.0;
    }
    let base = (rate >> 2) as i32 - 13;
    (1.0 + (rate & 3) as f32 / 4.0) * 2f32.powi(base)
}

impl Opll {
    pub fn write_address(&mut self, v: u8) {
        self.address = v;
    }

    pub fn write_data(&mut self, v: u8) {
        let a = self.address;
        match a {
            0x00..=0x07 => self.custom[a as usize] = v,
            0x10..=0x15 => {
                self.channels[(a - 0x10) as usize].fnum =
                    (self.channels[(a - 0x10) as usize].fnum & 0x100) | v as u16
            }
            0x20..=0x25 => {
                let ch = &mut self.channels[(a - 0x20) as usize];
                ch.fnum = (ch.fnum & 0xFF) | ((v as u16 & 1) << 8);
                ch.block = (v >> 1) & 7;
                ch.sustain = v & 0x20 != 0;
                let key = v & 0x10 != 0;
                if key && !ch.key {
                    for op in &mut ch.ops {
                        op.state = EgState::Attack;
                        op.phase = 0;
                    }
                } else if !key && ch.key {
                    for op in &mut ch.ops {
                        if op.state != EgState::Off {
                            op.state = EgState::Release;
                        }
                    }
                }
                ch.key = key;
            }
            0x30..=0x35 => {
                let ch = &mut self.channels[(a - 0x30) as usize];
                ch.instrument = v >> 4;
                ch.volume = v & 0x0F;
            }
            _ => {}
        }
    }

    pub fn reset(&mut self) {
        *self = Opll::default();
    }

    fn patch(&self, n: u8) -> [u8; 8] {
        if n == 0 {
            self.custom
        } else {
            PATCHES[n as usize]
        }
    }

    /// Called every CPU cycle.
    pub fn clock(&mut self) {
        self.divider += 1;
        if self.divider < 36 {
            return;
        }
        self.divider = 0;
        self.lfo_am = (self.lfo_am + 3.7 / SAMPLE_RATE) % 1.0;
        self.lfo_pm = (self.lfo_pm + 6.4 / SAMPLE_RATE) % 1.0;
        let am = (1.0 - (2.0 * self.lfo_am - 1.0).abs()) * 12.8; // up to 4.8 dB
        let pm = (2.0 * PI * self.lfo_pm).sin() * 0.004; // about ±7 cents

        let mut out = 0.0;
        for c in 0..6 {
            let patch = self.patch(self.channels[c].instrument);
            out += Self::channel_sample(&mut self.channels[c], &patch, am, pm);
        }
        self.output = out * CHANNEL_GAIN;
    }

    pub fn output(&self) -> f32 {
        self.output
    }

    fn channel_sample(ch: &mut Channel, p: &[u8; 8], am: f32, pm: f32) -> f32 {
        let key_code = ((ch.block as u32) << 1) | (ch.fnum as u32 >> 8);
        let mut level = [0.0f32; 2];
        for (i, op) in ch.ops.iter_mut().enumerate() {
            let flags = p[i];
            let ksr = flags & 0x10 != 0;
            let percussive = flags & 0x20 == 0;
            let (ar, dr) = (p[4 + i] >> 4, p[4 + i] & 0x0F);
            let (sl, rr) = (p[6 + i] >> 4, p[6 + i] & 0x0F);
            let rks = if ksr { key_code } else { key_code >> 2 };
            let rate = |r: u8| {
                if r == 0 {
                    0
                } else {
                    (4 * r as u32 + rks).min(63)
                }
            };

            // Phase.
            let vib = if flags & 0x40 != 0 { 1.0 + pm } else { 1.0 };
            let inc = ((ch.fnum as u32 * MULT_X2[(flags & 0x0F) as usize]) << ch.block) >> 1;
            op.phase = op.phase.wrapping_add((inc as f32 * vib) as u32) & 0x7FFFF;

            // Envelope.
            match op.state {
                EgState::Attack => {
                    let r = rate(ar);
                    if r >= 60 {
                        op.eg = 0.0;
                    } else {
                        let steps = eg_step(r);
                        op.eg -= (op.eg / 8.0 + 1.0) * steps;
                    }
                    if op.eg <= 0.0 {
                        op.eg = 0.0;
                        op.state = EgState::Decay;
                    }
                }
                EgState::Decay => {
                    op.eg += eg_step(rate(dr));
                    let sustain_level = sl as f32 * 8.0;
                    if op.eg >= sustain_level {
                        op.eg = sustain_level;
                        op.state = EgState::Sustain;
                    }
                }
                EgState::Sustain => {
                    // Percussive instruments keep decaying at the release rate.
                    if percussive {
                        op.eg += eg_step(rate(rr));
                    }
                }
                EgState::Release => {
                    let r = if ch.sustain {
                        5
                    } else if percussive {
                        rr
                    } else {
                        7
                    };
                    op.eg += eg_step(rate(r));
                }
                EgState::Off => op.eg = EG_MAX,
            }
            if op.eg >= EG_MAX && matches!(op.state, EgState::Release | EgState::Sustain) {
                op.eg = EG_MAX;
                if op.state == EgState::Release {
                    op.state = EgState::Off;
                }
            }

            // Total attenuation in dB.
            let ksl_mode = if i == 0 { p[2] >> 6 } else { p[3] >> 6 };
            let ksl = if ksl_mode == 0 {
                0.0
            } else {
                let base =
                    (KSL_DB[(ch.fnum >> 5) as usize & 15] - 6.0 * (7 - ch.block) as f32).max(0.0);
                base / (1 << (3 - ksl_mode)) as f32
            };
            let tl = if i == 0 {
                (p[2] & 0x3F) as f32 * 0.75
            } else {
                ch.volume as f32 * 3.0
            };
            let am_db = if flags & 0x80 != 0 { am * 0.375 } else { 0.0 };
            let db = op.eg * 0.375 + ksl + tl + am_db;
            level[i] = if db >= 48.0 {
                0.0
            } else {
                10f32.powf(-db / 20.0)
            };
        }

        let wave = |phase: f32, rectified: bool| {
            let s = phase.sin();
            if rectified && s < 0.0 { 0.0 } else { s }
        };
        let to_rad = |phase: u32| phase as f32 / 524_288.0 * 2.0 * PI;

        // Modulator, with self-feedback from its last two outputs.
        let fb = FEEDBACK[(p[3] & 7) as usize] * (ch.feedback[0] + ch.feedback[1]) / 2.0;
        let m = wave(to_rad(ch.ops[0].phase) + fb, p[3] & 0x08 != 0) * level[0];
        ch.feedback = [ch.feedback[1], m];

        // Carrier, phase-modulated by the modulator.
        wave(to_rad(ch.ops[1].phase) + m * 4.0 * PI, p[3] & 0x10 != 0) * level[1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_note_makes_sound_and_release_silences_it() {
        let mut o = Opll::default();
        o.write_address(0x30);
        o.write_data(0x30); // flute, full volume
        o.write_address(0x10);
        o.write_data(0xAC);
        o.write_address(0x20);
        o.write_data(0x10 | (4 << 1)); // key on, block 4
        let mut peak = 0.0f32;
        for _ in 0..36 * 2000 {
            o.clock();
            peak = peak.max(o.output().abs());
        }
        assert!(peak > 0.01, "peak {peak}");
        o.write_address(0x20);
        o.write_data(4 << 1); // key off
        for _ in 0..36 * 50_000 {
            o.clock();
        }
        assert!(o.output().abs() < 0.001);
    }
}
