//! Persisted user settings.

use egui::Key;
use nes_core::{Buttons, Region};
use serde::{Deserialize, Serialize};

/// A keyboard binding for each NES button, in `Buttons::NAMES` order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PadKeys(pub [Option<Key>; 8]);

impl PadKeys {
    pub fn player1() -> Self {
        PadKeys([
            Some(Key::X),
            Some(Key::Z),
            Some(Key::Space),
            Some(Key::Enter),
            Some(Key::ArrowUp),
            Some(Key::ArrowDown),
            Some(Key::ArrowLeft),
            Some(Key::ArrowRight),
        ])
    }

    pub fn none() -> Self {
        PadKeys([None; 8])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaletteChoice {
    /// Generated from the composite signal (default).
    Composite,
    /// Loaded from a `.pal` file (kept in `custom_palette`).
    Custom,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub keys: [PadKeys; 2],
    /// Second keyboard binding for turbo A / turbo B, per player.
    pub turbo_keys: [[Option<Key>; 2]; 2],
    pub gamepads_enabled: bool,
    pub allow_opposite_directions: bool,

    pub volume: f32,
    pub muted: bool,
    /// Target audio latency in milliseconds.
    pub audio_latency_ms: u32,

    pub integer_scaling: bool,
    pub crop_overscan: bool,
    /// Show pixels with the NTSC 8:7 pixel aspect ratio.
    pub pixel_aspect: bool,
    pub palette: PaletteChoice,
    pub custom_palette: Option<Vec<u8>>,

    /// Force a region instead of the ROM header's.
    pub region: Option<Region>,
    pub fast_forward_speed: f32,
    pub rewind_enabled: bool,
    pub rewind_seconds: u32,

    pub recent_roms: Vec<String>,
    pub state_slot: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            keys: [PadKeys::player1(), PadKeys::none()],
            turbo_keys: [[Some(Key::S), Some(Key::A)], [None, None]],
            gamepads_enabled: true,
            allow_opposite_directions: false,
            volume: 0.8,
            muted: false,
            audio_latency_ms: 60,
            integer_scaling: false,
            crop_overscan: true,
            pixel_aspect: true,
            palette: PaletteChoice::Composite,
            custom_palette: None,
            region: None,
            fast_forward_speed: 4.0,
            rewind_enabled: true,
            rewind_seconds: 30,
            recent_roms: Vec::new(),
            state_slot: 1,
        }
    }
}

impl Settings {
    pub fn add_recent(&mut self, path: String) {
        self.recent_roms.retain(|p| p != &path);
        self.recent_roms.insert(0, path);
        self.recent_roms.truncate(10);
    }

    pub fn palette(&self, region: Region) -> nes_core::Palette {
        match (&self.palette, &self.custom_palette) {
            (PaletteChoice::Custom, Some(data)) => {
                nes_core::Palette::from_pal_file(data).unwrap_or_default()
            }
            _ if region == Region::Pal => nes_core::Palette::pal(),
            _ => nes_core::Palette::ntsc(),
        }
    }
}

/// Button bit for each `PadKeys` slot.
pub const BUTTON_ORDER: [u8; 8] = [
    Buttons::A,
    Buttons::B,
    Buttons::SELECT,
    Buttons::START,
    Buttons::UP,
    Buttons::DOWN,
    Buttons::LEFT,
    Buttons::RIGHT,
];
