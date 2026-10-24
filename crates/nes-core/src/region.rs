//! Console timing variants.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Region {
    #[default]
    Ntsc,
    Pal,
    /// Famiclone timing used in Russia and elsewhere: PAL frame rate with NTSC
    /// CPU/PPU ratio.
    Dendy,
}

impl Region {
    pub const ALL: [Region; 3] = [Region::Ntsc, Region::Pal, Region::Dendy];

    pub fn name(self) -> &'static str {
        match self {
            Region::Ntsc => "NTSC",
            Region::Pal => "PAL",
            Region::Dendy => "Dendy",
        }
    }

    /// Master clock frequency in Hz.
    pub fn master_clock_hz(self) -> f64 {
        match self {
            Region::Ntsc => 21_477_272.0,
            Region::Pal | Region::Dendy => 26_601_712.0,
        }
    }

    /// Master clocks per CPU cycle.
    pub fn cpu_divider(self) -> u64 {
        match self {
            Region::Ntsc => 12,
            Region::Pal => 16,
            Region::Dendy => 15,
        }
    }

    /// Master clocks per PPU dot.
    pub fn ppu_divider(self) -> u64 {
        match self {
            Region::Ntsc => 4,
            Region::Pal | Region::Dendy => 5,
        }
    }

    pub fn cpu_clock_hz(self) -> f64 {
        self.master_clock_hz() / self.cpu_divider() as f64
    }

    /// How a CPU cycle is split around its bus access, in master clocks:
    /// (before a read, after a read). Writes use the mirror image.
    pub(crate) fn cycle_split(self) -> (u64, u64) {
        match self {
            Region::Ntsc => (5, 7),
            Region::Pal => (7, 9),
            Region::Dendy => (6, 9),
        }
    }

    /// Index of the last scanline (the pre-render line wraps around to -1).
    pub fn last_scanline(self) -> i16 {
        match self {
            Region::Ntsc => 260,
            Region::Pal | Region::Dendy => 310,
        }
    }

    /// Scanline on which the VBlank flag is set and NMI fires.
    pub fn vblank_scanline(self) -> i16 {
        match self {
            Region::Ntsc | Region::Pal => 241,
            Region::Dendy => 291,
        }
    }

    pub fn frame_rate(self) -> f64 {
        let dots_per_frame = 341.0 * (self.last_scanline() as f64 + 2.0);
        let dots = match self {
            // Odd frames are one dot shorter while rendering is on.
            Region::Ntsc => dots_per_frame - 0.5,
            _ => dots_per_frame,
        };
        self.master_clock_hz() / self.ppu_divider() as f64 / dots
    }
}
