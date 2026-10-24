//! Settings and cheats windows.

use crate::input::Input;
use crate::session::Session;
use crate::settings::{PaletteChoice, Settings};
use egui::Key;
use nes_core::Buttons;
use nes_core::cheats::Cheat;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SettingsTab {
    #[default]
    Input,
    Video,
    Audio,
    Emulation,
}

/// What a key-capture is assigning.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Binding {
    Button { port: usize, slot: usize },
    Turbo { port: usize, which: usize },
}

#[derive(Default)]
pub struct SettingsWindow {
    pub open: bool,
    tab: SettingsTab,
    capturing: Option<Binding>,
    palette_error: Option<String>,
    palette_inbox: crate::platform::FileInbox,
}

fn key_label(k: Option<Key>) -> String {
    k.map(|k| k.name().to_string())
        .unwrap_or_else(|| "—".into())
}

impl SettingsWindow {
    /// Returns true if something visual or audible changed.
    pub fn show(&mut self, ctx: &egui::Context, settings: &mut Settings, input: &Input) -> bool {
        let mut changed = false;
        for f in self.palette_inbox.take() {
            match nes_core::Palette::from_pal_file(&f.data) {
                Some(_) => {
                    settings.custom_palette = Some(f.data);
                    settings.palette = PaletteChoice::Custom;
                    self.palette_error = None;
                    changed = true;
                }
                None => {
                    self.palette_error =
                        Some(format!("{} is not a 192- or 1536-byte .pal file", f.name))
                }
            }
        }

        if let Some(binding) = self.capturing {
            let pressed = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key {
                        key, pressed: true, ..
                    } => Some(*key),
                    _ => None,
                })
            });
            if let Some(key) = pressed {
                let value = match key {
                    Key::Escape => None,
                    Key::Delete => Some(None),
                    k => Some(Some(k)),
                };
                if let Some(v) = value {
                    match binding {
                        Binding::Button { port, slot } => settings.keys[port].0[slot] = v,
                        Binding::Turbo { port, which } => settings.turbo_keys[port][which] = v,
                    }
                }
                self.capturing = None;
            }
        }

        let mut open = self.open;
        egui::Window::new("Settings")
            .open(&mut open)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab, SettingsTab::Input, "Input");
                    ui.selectable_value(&mut self.tab, SettingsTab::Video, "Video");
                    ui.selectable_value(&mut self.tab, SettingsTab::Audio, "Audio");
                    ui.selectable_value(&mut self.tab, SettingsTab::Emulation, "Emulation");
                });
                ui.separator();
                match self.tab {
                    SettingsTab::Input => self.input_tab(ui, settings, input),
                    SettingsTab::Video => {
                        changed |= ui
                            .checkbox(
                                &mut settings.crop_overscan,
                                "Hide overscan (top/bottom 8 lines)",
                            )
                            .changed();
                        changed |= ui
                            .checkbox(&mut settings.pixel_aspect, "NTSC pixel aspect ratio (8:7)")
                            .changed();
                        changed |= ui
                            .checkbox(&mut settings.integer_scaling, "Integer scaling")
                            .changed();
                        ui.separator();
                        ui.label("Palette");
                        changed |= ui
                            .radio_value(
                                &mut settings.palette,
                                PaletteChoice::Composite,
                                "Composite (generated from the NTSC signal)",
                            )
                            .changed();
                        ui.horizontal(|ui| {
                            let has_custom = settings.custom_palette.is_some();
                            changed |= ui
                                .add_enabled(
                                    has_custom,
                                    egui::RadioButton::new(
                                        settings.palette == PaletteChoice::Custom,
                                        "Custom .pal",
                                    ),
                                )
                                .clicked()
                                .then(|| settings.palette = PaletteChoice::Custom)
                                .is_some();
                            if ui.button("Load .pal…").clicked() {
                                pick_palette(&self.palette_inbox);
                            }
                        });
                        if let Some(e) = &self.palette_error {
                            ui.colored_label(egui::Color32::from_rgb(255, 120, 120), e);
                        }
                    }
                    SettingsTab::Audio => {
                        changed |= ui.checkbox(&mut settings.muted, "Mute").changed();
                        changed |= ui
                            .add(egui::Slider::new(&mut settings.volume, 0.0..=1.5).text("Volume"))
                            .changed();
                        ui.add(
                            egui::Slider::new(&mut settings.audio_latency_ms, 30..=200)
                                .text("Target latency (ms)"),
                        );
                    }
                    SettingsTab::Emulation => {
                        ui.add(
                            egui::Slider::new(&mut settings.fast_forward_speed, 1.5..=10.0)
                                .text("Fast-forward speed"),
                        );
                        ui.checkbox(&mut settings.rewind_enabled, "Enable rewind");
                        ui.add_enabled(
                            settings.rewind_enabled,
                            egui::Slider::new(&mut settings.rewind_seconds, 5..=120)
                                .text("Rewind history (s)"),
                        );
                        ui.checkbox(
                            &mut settings.allow_opposite_directions,
                            "Allow pressing opposite directions",
                        );
                    }
                }
            });
        self.open = open;
        changed
    }

    fn input_tab(&mut self, ui: &mut egui::Ui, settings: &mut Settings, input: &Input) {
        ui.label("Click a binding, then press a key (Esc cancels, Delete clears).");
        egui::Grid::new("bindings")
            .num_columns(3)
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Button");
                ui.strong("Player 1");
                ui.strong("Player 2");
                ui.end_row();
                for (slot, (name, _)) in Buttons::NAMES.iter().enumerate() {
                    ui.label(*name);
                    for port in 0..2 {
                        let b = Binding::Button { port, slot };
                        let label = if self.capturing == Some(b) {
                            "press a key…".into()
                        } else {
                            key_label(settings.keys[port].0[slot])
                        };
                        if ui.button(label).clicked() {
                            self.capturing = Some(b);
                        }
                    }
                    ui.end_row();
                }
                for (which, name) in ["Turbo A", "Turbo B"].iter().enumerate() {
                    ui.label(*name);
                    for port in 0..2 {
                        let b = Binding::Turbo { port, which };
                        let label = if self.capturing == Some(b) {
                            "press a key…".into()
                        } else {
                            key_label(settings.turbo_keys[port][which])
                        };
                        if ui.button(label).clicked() {
                            self.capturing = Some(b);
                        }
                    }
                    ui.end_row();
                }
            });
        ui.separator();
        ui.checkbox(&mut settings.gamepads_enabled, "Use gamepads");
        if input.gamepad_names.is_empty() {
            ui.weak("No gamepads detected.");
        }
        for (i, name) in input.gamepad_names.iter().enumerate() {
            ui.label(format!("Player {}: {name}", i + 1));
        }
        ui.weak(
            "Gamepad: D-pad/stick, East = A, South = B, North/West = turbo A/B, Start, Select.",
        );
        if ui.button("Reset bindings to defaults").clicked() {
            let d = Settings::default();
            settings.keys = d.keys;
            settings.turbo_keys = d.turbo_keys;
        }
    }
}

fn pick_palette(inbox: &crate::platform::FileInbox) {
    let dialog = rfd::AsyncFileDialog::new().add_filter("Palette", &["pal"]);
    let inbox = inbox.clone();
    crate::platform::spawn_task(async move {
        if let Some(h) = dialog.pick_file().await {
            let data = h.read().await;
            inbox.deliver(crate::platform::PickedFile {
                name: h.file_name(),
                path: None,
                data,
            });
        }
    });
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CheatsWindow {
    #[serde(skip)]
    pub open: bool,
    #[serde(skip)]
    code: String,
    #[serde(skip)]
    error: Option<String>,
    /// Cheats per game id, remembered across sessions.
    saved: HashMap<String, Vec<Cheat>>,
}

impl CheatsWindow {
    pub fn apply_to(&self, s: &mut Session) {
        s.nes.bus.cheats.list = self.saved.get(&s.id).cloned().unwrap_or_default();
        s.nes.bus.cheats.refresh();
    }

    pub fn show(&mut self, ctx: &egui::Context, session: Option<&mut Session>) {
        let mut open = self.open;
        egui::Window::new("Cheats")
            .open(&mut open)
            .default_width(340.0)
            .show(ctx, |ui| {
                let Some(s) = session else {
                    ui.weak("Load a ROM first.");
                    return;
                };
                ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.code)
                            .desired_width(150.0)
                            .hint_text("SXIOPO or 8123:45"),
                    );
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    if ui.button("Add").clicked() || enter {
                        match Cheat::parse(&self.code) {
                            Ok(c) => {
                                s.nes.bus.cheats.add(c);
                                self.code.clear();
                                self.error = None;
                            }
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                });
                if let Some(e) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(255, 120, 120), e);
                }
                ui.separator();
                let cheats = &mut s.nes.bus.cheats;
                let mut remove = None;
                let mut changed = false;
                if cheats.list.is_empty() {
                    ui.weak("No cheats for this game.");
                }
                for (i, c) in cheats.list.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut c.enabled, "").changed();
                        ui.monospace(&c.code);
                        let cmp = c
                            .compare
                            .map(|v| format!(" if ${v:02X}"))
                            .unwrap_or_default();
                        ui.weak(format!("${:04X} = ${:02X}{cmp}", c.addr, c.value));
                        if ui.small_button("x").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    cheats.remove(i);
                } else if changed {
                    cheats.refresh();
                }
                self.saved.insert(s.id.clone(), cheats.list.clone());
            });
        self.open = open;
    }
}
