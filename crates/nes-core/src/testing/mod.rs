//! Helpers for running accuracy test ROMs headlessly.
//!
//! Most of blargg's newer test ROMs report through cartridge RAM: `$6000`
//! holds a status byte (`$80` running, `$81` "press reset", otherwise the
//! result code with 0 meaning pass), `$6001-$6003` hold the signature
//! `DE B0 61`, and `$6004` onward a NUL-terminated message.

pub mod suite;

use crate::{Nes, StopReason};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Passed(String),
    Failed {
        code: u8,
        message: String,
    },
    /// Still running when the frame budget ran out.
    Timeout(String),
    /// The ROM never wrote the `$6000` signature.
    NoProtocol,
}

impl Outcome {
    pub fn passed(&self) -> bool {
        matches!(self, Outcome::Passed(_))
    }
}

fn has_signature(nes: &Nes) -> bool {
    nes.peek(0x6001) == 0xDE && nes.peek(0x6002) == 0xB0 && nes.peek(0x6003) == 0x61
}

/// The message at `$6004`.
pub fn message(nes: &Nes) -> String {
    let mut s = Vec::new();
    for a in 0x6004..0x7000u16 {
        let b = nes.peek(a);
        if b == 0 {
            break;
        }
        s.push(b);
    }
    String::from_utf8_lossy(&s).trim().to_string()
}

/// Runs a `$6000`-protocol test ROM for at most `max_frames` frames.
pub fn run_blargg(nes: &mut Nes, max_frames: u32) -> Outcome {
    let mut reset_at: Option<u32> = None;
    // After pressing RESET, the status stays $81 until the test reaches its
    // new phase; don't press again in the meantime.
    let mut reset_pressed = false;
    let mut seen_signature = false;
    for frame in 0..max_frames {
        while nes.run_frame() != StopReason::FrameComplete {}
        if !has_signature(nes) {
            if seen_signature || frame < 120 {
                continue;
            }
            return Outcome::NoProtocol;
        }
        seen_signature = true;
        let status = nes.peek(0x6000);
        log::trace!("frame {frame}: status ${status:02X}");
        if status != 0x81 {
            reset_pressed = false;
        }
        match status {
            0x80 => {}
            0x81 if !reset_pressed => {
                // The test wants RESET pressed; wait a few frames like a human would.
                match reset_at {
                    None => reset_at = Some(frame + 6),
                    Some(at) if frame >= at => {
                        reset_at = None;
                        reset_pressed = true;
                        nes.reset();
                    }
                    _ => {}
                }
            }
            0x81 => {}
            0 => return Outcome::Passed(message(nes)),
            code => {
                return Outcome::Failed {
                    code,
                    message: message(nes),
                };
            }
        }
    }
    if seen_signature {
        Outcome::Timeout(message(nes))
    } else {
        Outcome::NoProtocol
    }
}

/// Text of the first nametable, one line per tile row, for test ROMs that
/// only report on screen. Their fonts map ASCII codes straight to tiles.
pub fn screen_text(nes: &Nes) -> String {
    use crate::cart::Mapper;
    let mut out = String::new();
    for row in 0..30u16 {
        let line: String = (0..32u16)
            .map(|col| {
                let t = nes.bus.cart.ppu_peek(0x2000 + row * 32 + col);
                if (0x20..0x7F).contains(&t) {
                    t as char
                } else {
                    ' '
                }
            })
            .collect();
        let line = line.trim_end();
        if !line.is_empty() {
            out.push_str(line.trim_start());
            out.push('\n');
        }
    }
    out
}

/// FNV-1a hash of the framebuffer, for screenshot regression tests.
pub fn frame_hash(nes: &Nes) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &p in nes.framebuffer() {
        for b in p.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}
