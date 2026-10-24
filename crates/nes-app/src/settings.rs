//! Persisted user settings.

use egui::Key;
use nes_core::{Buttons, Region};
use serde::{Deserialize, Serialize};

/// A keyboard binding for each NES button, in `Buttons::NAMES` order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PadKeys(pub [Option<Key>; 8]);

impl PadKeys {
    /// WASD for the D-pad; J/K are B/A (B on the left, as on the pad), L is
    /// Select and O Start. Turbo B/A sit above on U/I.
    pub fn player1() -> Self {
        PadKeys([
            Some(Key::K),
            Some(Key::J),
            Some(Key::L),
            Some(Key::O),
            Some(Key::W),
            Some(Key::S),
            Some(Key::A),
            Some(Key::D),
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

    /// Draw the console around the picture.
    pub console_frame: bool,
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
    /// Which default key layout these settings were saved with; see
    /// [`Settings::migrate`].
    #[serde(default)]
    pub bindings_version: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            keys: [PadKeys::player1(), PadKeys::none()],
            turbo_keys: [[Some(Key::I), Some(Key::U)], [None, None]],
            gamepads_enabled: true,
            allow_opposite_directions: false,
            volume: 0.8,
            muted: false,
            audio_latency_ms: 60,
            console_frame: true,
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
            bindings_version: BINDINGS_VERSION,
        }
    }
}

/// Bumped when the default key layout changes.
const BINDINGS_VERSION: u32 = 1;

impl Settings {
    /// Brings settings saved by an older version up to date. Player 1 keys
    /// still on the old default layout (arrows, X/Z, Enter/Space, turbo S/A)
    /// move to the current one; customized bindings are left alone.
    pub fn migrate(&mut self) {
        if self.bindings_version < 1 {
            use Key::*;
            let old = PadKeys(
                [
                    X, Z, Space, Enter, ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
                ]
                .map(Some),
            );
            if self.keys[0] == old && self.turbo_keys[0] == [Some(S), Some(A)] {
                let d = Settings::default();
                self.keys[0] = d.keys[0].clone();
                self.turbo_keys[0] = d.turbo_keys[0];
            }
        }
        self.bindings_version = BINDINGS_VERSION;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn saved_by_old_version() -> Settings {
        use Key::*;
        Settings {
            keys: [
                PadKeys(
                    [
                        X, Z, Space, Enter, ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
                    ]
                    .map(Some),
                ),
                PadKeys::none(),
            ],
            turbo_keys: [[Some(S), Some(A)], [None, None]],
            bindings_version: 0,
            ..Settings::default()
        }
    }

    #[test]
    fn old_default_keys_move_to_the_new_layout() {
        let mut s = saved_by_old_version();
        s.migrate();
        assert_eq!(s.keys, Settings::default().keys);
        assert_eq!(s.turbo_keys, Settings::default().turbo_keys);
    }

    #[test]
    fn customized_keys_are_kept() {
        let mut s = saved_by_old_version();
        s.keys[0].0[0] = Some(Key::C);
        let keys = s.keys.clone();
        s.migrate();
        assert_eq!(s.keys, keys);

        // Once migrated, choosing the old layout again sticks.
        let mut s = saved_by_old_version();
        s.bindings_version = BINDINGS_VERSION;
        let keys = s.keys.clone();
        s.migrate();
        assert_eq!(s.keys, keys);
    }

    #[test]
    fn missing_version_reads_as_old() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.bindings_version, 0);
    }
}
