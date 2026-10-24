//! The application: menus, hotkeys, emulation pacing, and window layout.

use crate::audio::Audio;
use crate::debugger::DebugWindows;
use crate::input::{self, Input};
use crate::platform::{self, FileInbox, PickedFile, Store};
use crate::session::Session;
use crate::settings::Settings;
use crate::windows::{CheatsWindow, SettingsWindow};
use egui::{Color32, Key, KeyboardShortcut, Modifiers, RichText};
use nes_core::{Buttons, Palette, Region};
use web_time::Instant;

const SETTINGS_KEY: &str = "settings";
const CHEATS_KEY: &str = "cheats";

const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const RESET: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::R);
const POWER: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::R);
const DEBUGGER: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::D);
const NUMBER_KEYS: [Key; 10] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
    Key::Num0,
];

pub struct NesApp {
    pub settings: Settings,
    store: Store,
    audio: Audio,
    input: Input,
    inbox: FileInbox,
    pub session: Option<Session>,
    texture: Option<egui::TextureHandle>,
    rgba: Vec<u8>,
    palette: Palette,
    palette_dirty: bool,
    last_tick: Option<Instant>,
    /// Emulated frames per second, smoothed.
    fps: f32,
    fps_frames: u32,
    fps_since: Option<Instant>,
    /// (message, expiry).
    toast: Option<(String, Instant)>,
    error: Option<String>,
    /// Controller state sent to the console this frame, for the status bar.
    pads: [Buttons; 2],
    fast_forward: bool,
    rewinding: bool,
    speed: f32,
    debug: DebugWindows,
    settings_window: SettingsWindow,
    cheats_window: CheatsWindow,
    show_about: bool,
    save_timer: f64,
    #[cfg(not(target_arch = "wasm32"))]
    capture: Option<Capture>,
}

/// `--capture`: save a window screenshot after some frames, then quit
/// (used for automated visual checks and documentation).
#[cfg(not(target_arch = "wasm32"))]
struct Capture {
    path: std::path::PathBuf,
    frames_left: u32,
    requested: bool,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl NesApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, SETTINGS_KEY))
            .unwrap_or_default();
        settings.migrate();
        #[cfg_attr(not(target_arch = "wasm32"), allow(unused_mut))]
        let mut debug: DebugWindows = cc
            .storage
            .and_then(|s| eframe::get_value(s, crate::debugger::STORAGE_KEY))
            .unwrap_or_default();
        let audio = Audio::new();
        audio.set_volume(if settings.muted { 0.0 } else { settings.volume });
        let palette = settings.palette(Region::Ntsc);
        let inbox = FileInbox::default();
        // Web: `?rom=<url>` loads a ROM, `?debug` opens the debugger.
        #[cfg(target_arch = "wasm32")]
        for (key, value) in platform::page_params() {
            match key.as_str() {
                "rom" if !value.is_empty() => platform::fetch_rom(value, &inbox),
                "debug" => debug.docked = true,
                _ => {}
            }
        }
        Self {
            store: Store::new(cc.storage),
            audio,
            input: Input::new(),
            inbox,
            session: None,
            texture: None,
            rgba: Vec::new(),
            palette,
            palette_dirty: false,
            last_tick: None,
            fps: 0.0,
            fps_frames: 0,
            fps_since: None,
            toast: None,
            error: None,
            pads: Default::default(),
            fast_forward: false,
            rewinding: false,
            speed: 1.0,
            debug,
            settings_window: SettingsWindow::default(),
            cheats_window: cc
                .storage
                .and_then(|s| eframe::get_value(s, CHEATS_KEY))
                .unwrap_or_default(),
            show_about: false,
            save_timer: 0.0,
            #[cfg(not(target_arch = "wasm32"))]
            capture: None,
            settings,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_capture(&mut self, path: std::path::PathBuf, frames: u32) {
        self.capture = Some(Capture {
            path,
            frames_left: frames,
            requested: false,
            done: Default::default(),
        });
    }

    /// Forgets remembered window positions and debugger layout.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn reset_layout(&mut self, ctx: &egui::Context) {
        self.debug = DebugWindows::default();
        ctx.memory_mut(|m| *m = Default::default());
    }

    /// Command-line overrides for the debugger layout.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn show_debugger(&mut self, panels: bool, viewers: bool) {
        if panels {
            self.debug.docked = true;
        }
        if viewers {
            self.debug.ppu = true;
            self.debug.apu = true;
            self.debug.cart = true;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn drive_capture(&mut self, ctx: &egui::Context) {
        use std::sync::atomic::Ordering;
        let Some(cap) = self.capture.as_mut() else {
            return;
        };
        if cap.done.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        ctx.request_repaint();
        if cap.frames_left > 0 {
            cap.frames_left -= 1;
            return;
        }
        if !cap.requested {
            cap.requested = true;
            let path = cap.path.clone();
            let done = cap.done.clone();
            ctx.request_screenshot(move |image| {
                let rgba: Vec<u8> = image
                    .pixels
                    .iter()
                    .flat_map(|c| c.to_srgba_unmultiplied())
                    .collect();
                let [w, h] = image.size;
                let result = std::fs::File::create(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|f| {
                        let mut enc =
                            png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
                        enc.set_color(png::ColorType::Rgba);
                        enc.set_depth(png::BitDepth::Eight);
                        enc.write_header()
                            .and_then(|mut wr| wr.write_image_data(&rgba))
                            .map_err(|e| e.to_string())
                    });
                match result {
                    Ok(()) => log::info!("captured {}", path.display()),
                    Err(e) => log::error!("capture failed: {e}"),
                }
                done.store(true, Ordering::SeqCst);
            });
        }
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((
            msg.into(),
            Instant::now() + std::time::Duration::from_secs(3),
        ));
    }

    // ------------------------------------------------------------------
    // ROM loading and saving.

    fn open_file(&mut self, file: PickedFile) {
        self.flush_battery_save();
        let rate = self.audio.sample_rate as f64;
        match Session::load(&file.name, &file.data, self.settings.region, rate) {
            Ok(mut session) => {
                if let Some(save) = self.store.read(&session.save_key()) {
                    session.restore_save_data(&save);
                }
                if let Some(path) = &file.path {
                    self.settings.add_recent(path.clone());
                }
                self.palette = self.settings.palette(session.nes.region());
                self.debug.on_load(&mut session);
                self.cheats_window.apply_to(&mut session);
                self.audio.clear();
                self.toast(format!(
                    "Loaded {} ({})",
                    session.name,
                    session.mapper_name()
                ));
                self.session = Some(session);
                self.error = None;
                self.last_tick = None;
            }
            Err(e) => self.error = Some(format!("Couldn't load {}: {e}", file.name)),
        }
    }

    /// Opens a ROM given on the command line.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_path(&mut self, path: &str) {
        self.open_recent(path.to_string());
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_recent(&mut self, path: String) {
        match std::fs::read(&path) {
            Ok(data) => {
                let name = std::path::Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone());
                self.open_file(PickedFile {
                    name,
                    path: Some(path),
                    data,
                });
            }
            Err(e) => {
                self.error = Some(format!("Couldn't open {path}: {e}"));
                self.settings.recent_roms.retain(|p| p != &path);
            }
        }
    }

    fn flush_battery_save(&mut self) {
        if let Some(s) = self.session.as_mut()
            && let Some(data) = s.dirty_save_data()
        {
            let key = s.save_key();
            self.store.write(&key, &data);
        }
    }

    fn close_rom(&mut self) {
        self.flush_battery_save();
        self.session = None;
        self.texture = None;
        self.audio.clear();
    }

    fn save_state(&mut self, slot: u8) {
        let Some(s) = self.session.as_ref() else {
            return;
        };
        let data = s.nes.save_state();
        let key = s.state_key(slot);
        self.store.write(&key, &data);
        self.settings.state_slot = slot;
        self.toast(format!("Saved state {slot}"));
    }

    fn load_state(&mut self, slot: u8) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        let Some(data) = self.store.read(&s.state_key(slot)) else {
            self.toast(format!("State {slot} is empty"));
            return;
        };
        match s.nes.load_state(&data) {
            Ok(()) => {
                s.clear_rewind();
                s.stopped = None;
                self.settings.state_slot = slot;
                self.toast(format!("Loaded state {slot}"));
            }
            Err(e) => self.error = Some(format!("Couldn't load state {slot}: {e}")),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn screenshot(&mut self) {
        let Some(s) = self.session.as_ref() else {
            return;
        };
        let Some(dir) = self.store.dir() else { return };
        let dir = dir.join("screenshots");
        let _ = std::fs::create_dir_all(&dir);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = dir.join(format!("{}-{stamp}.png", s.id));
        let mut rgba = Vec::new();
        self.palette.convert(s.nes.framebuffer(), &mut rgba);
        let result = std::fs::File::create(&path)
            .map_err(|e| e.to_string())
            .and_then(|f| {
                let mut enc = png::Encoder::new(std::io::BufWriter::new(f), 256, 240);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                enc.write_header()
                    .and_then(|mut w| w.write_image_data(&rgba))
                    .map_err(|e| e.to_string())
            });
        match result {
            Ok(()) => self.toast(format!("Screenshot saved to {}", path.display())),
            Err(e) => self.error = Some(format!("Screenshot failed: {e}")),
        }
    }

    // ------------------------------------------------------------------
    // Input.

    fn handle_hotkeys(&mut self, ctx: &egui::Context) {
        if !input::keyboard_free(ctx) {
            return;
        }
        let mut actions = Vec::new();
        ctx.input_mut(|i| {
            if i.consume_shortcut(&OPEN) {
                actions.push(Action::Open);
            }
            if i.consume_shortcut(&POWER) {
                actions.push(Action::Power);
            }
            if i.consume_shortcut(&RESET) {
                actions.push(Action::Reset);
            }
            if i.consume_shortcut(&DEBUGGER) {
                actions.push(Action::ToggleDebugger);
            }
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                actions.push(Action::TogglePause);
            }
            if i.consume_key(Modifiers::NONE, Key::Period) {
                actions.push(Action::FrameAdvance);
            }
            if i.consume_key(Modifiers::NONE, Key::F12) {
                actions.push(Action::Screenshot);
            }
            if i.consume_key(Modifiers::NONE, Key::F5) {
                actions.push(Action::Continue);
            }
            if i.consume_key(Modifiers::NONE, Key::F10) {
                actions.push(Action::StepOver);
            }
            if i.consume_key(Modifiers::SHIFT, Key::F11) {
                actions.push(Action::StepOut);
            }
            if i.consume_key(Modifiers::NONE, Key::F11) {
                actions.push(Action::StepInto);
            }
            for (n, key) in NUMBER_KEYS.iter().enumerate() {
                let slot = n as u8 + 1;
                if i.consume_key(Modifiers::SHIFT, *key) {
                    actions.push(Action::SaveState(slot));
                } else if i.consume_key(Modifiers::NONE, *key) {
                    actions.push(Action::LoadState(slot));
                }
            }
            self.fast_forward = i.key_down(Key::Tab);
            self.rewinding = i.key_down(Key::Backspace) && self.settings.rewind_enabled;
        });
        for a in actions {
            self.perform(a);
        }
    }

    fn perform(&mut self, action: Action) {
        match action {
            Action::Open => platform::pick_rom(&self.inbox),
            Action::ToggleDebugger => self.debug.docked = !self.debug.docked,
            Action::Reset => {
                if let Some(s) = self.session.as_mut() {
                    s.nes.reset();
                    s.stopped = None;
                    self.toast("Reset");
                }
            }
            Action::Power => {
                self.flush_battery_save();
                if let Some(s) = self.session.as_mut() {
                    s.nes.power_cycle();
                    s.stopped = None;
                    s.clear_rewind();
                    self.cheats_window.apply_to(s);
                    self.toast("Power cycled");
                }
            }
            Action::TogglePause => {
                if let Some(s) = self.session.as_mut() {
                    if s.stopped.is_some() {
                        s.stopped = None;
                        s.paused = false;
                        s.nes.resume();
                    } else {
                        s.paused = !s.paused;
                    }
                }
            }
            Action::FrameAdvance => {
                if let Some(s) = self.session.as_mut() {
                    s.paused = true;
                    s.stopped = None;
                    s.nes.resume();
                    let rewind = self
                        .settings
                        .rewind_enabled
                        .then_some(self.settings.rewind_seconds);
                    s.run_frame(rewind);
                    s.nes.bus.apu.clear_samples();
                    self.debug.frame_needs_refresh = true;
                }
            }
            Action::Screenshot => {
                #[cfg(not(target_arch = "wasm32"))]
                self.screenshot();
            }
            Action::SaveState(slot) => self.save_state(slot),
            Action::LoadState(slot) => self.load_state(slot),
            Action::Continue => {
                if let Some(s) = self.session.as_mut() {
                    s.stopped = None;
                    s.paused = false;
                    s.nes.resume();
                }
            }
            Action::StepInto | Action::StepOver | Action::StepOut => {
                if let Some(s) = self.session.as_mut() {
                    self.debug.step(s, action_to_step(action));
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Emulation.

    fn emulate(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let dt = self
            .last_tick
            .map(|t| (now - t).as_secs_f64())
            .unwrap_or(0.0)
            .min(0.1);
        self.last_tick = Some(now);
        let Some(session) = self.session.as_mut() else {
            return;
        };

        let keyboard = input::keyboard_free(ctx);
        let frame = session.nes.frame();
        let pads = self.input.poll(ctx, &self.settings, keyboard, frame);
        self.pads = pads;
        // Any input means a user gesture happened; browsers need one before
        // audio may start.
        if pads.iter().any(|p| p.0 != 0) || ctx.input(|i| !i.events.is_empty()) {
            self.audio.start();
        }
        session.nes.set_buttons(0, pads[0]);
        session.nes.set_buttons(1, pads[1]);

        let mut produced = 0u32;
        if self.rewinding && session.running() {
            if session.rewind_step() {
                produced += 1;
            }
            session.time_debt = 0.0;
            self.audio.clear();
        } else if session.running() {
            let speed = if self.fast_forward {
                self.settings.fast_forward_speed
            } else {
                self.speed
            };
            session.time_debt += dt * speed as f64;
            let frame_time = session.frame_duration();
            let max_frames = (8.0 * speed.max(1.0)) as u32;
            let rewind = self
                .settings
                .rewind_enabled
                .then_some(self.settings.rewind_seconds);
            while session.time_debt >= frame_time && produced < max_frames {
                session.time_debt -= frame_time;
                let complete = session.run_frame(rewind);
                if complete {
                    produced += 1;
                }
                if !session.running() {
                    break;
                }
            }
            if produced == max_frames {
                // Can't keep up; drop the backlog rather than spiral.
                session.time_debt = 0.0;
            }
        } else {
            session.time_debt = 0.0;
        }

        // Audio: only at normal speed; otherwise drop samples.
        let mut samples = Vec::new();
        session.nes.drain_audio(&mut samples);
        if produced > 0 && !self.fast_forward && self.speed == 1.0 && !self.rewinding {
            self.audio.push(&samples);
        }
        let target =
            (self.audio.sample_rate as usize * self.settings.audio_latency_ms as usize) / 1000;
        let nudge = self.audio.rate_nudge(target);
        session
            .nes
            .bus
            .apu
            .set_sample_rate(self.audio.sample_rate as f64, nudge);

        if produced > 0 {
            self.fps_frames += produced;
            self.debug.frame_needs_refresh = true;
            self.update_texture(ctx);
        }
        let since = *self.fps_since.get_or_insert(now);
        let elapsed = (now - since).as_secs_f32();
        if elapsed >= 0.5 {
            self.fps = self.fps_frames as f32 / elapsed;
            self.fps_frames = 0;
            self.fps_since = Some(now);
        }

        // Periodically persist battery RAM so a crash doesn't lose progress.
        self.save_timer += dt;
        if self.save_timer > 10.0 {
            self.save_timer = 0.0;
            self.flush_battery_save();
        }
    }

    fn update_texture(&mut self, ctx: &egui::Context) {
        let Some(s) = self.session.as_ref() else {
            return;
        };
        if self.palette_dirty {
            self.palette = self.settings.palette(s.nes.region());
            self.palette_dirty = false;
        }
        self.palette.convert(s.nes.framebuffer(), &mut self.rgba);
        let image = egui::ColorImage::from_rgba_unmultiplied([256, 240], &self.rgba);
        match &mut self.texture {
            Some(t) => t.set(image, egui::TextureOptions::NEAREST),
            None => {
                self.texture =
                    Some(ctx.load_texture("nes-frame", image, egui::TextureOptions::NEAREST))
            }
        }
    }

    // ------------------------------------------------------------------
    // UI.

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui
                    .add(
                        egui::Button::new("Open ROM…")
                            .shortcut_text(ui.ctx().format_shortcut(&OPEN)),
                    )
                    .clicked()
                {
                    self.perform(Action::Open);
                }
                #[cfg(not(target_arch = "wasm32"))]
                ui.menu_button("Recent", |ui| {
                    if self.settings.recent_roms.is_empty() {
                        ui.weak("(none)");
                    }
                    let mut open = None;
                    for path in &self.settings.recent_roms {
                        let label = std::path::Path::new(path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.clone());
                        if ui.button(label).on_hover_text(path).clicked() {
                            open = Some(path.clone());
                        }
                    }
                    if let Some(p) = open {
                        self.open_recent(p);
                    }
                });
                let loaded = self.session.is_some();
                if ui
                    .add_enabled(loaded, egui::Button::new("Close ROM"))
                    .clicked()
                {
                    self.close_rom();
                }
                ui.separator();
                ui.add_enabled_ui(loaded, |ui| {
                    ui.menu_button("Save state", |ui| {
                        for slot in 1..=10u8 {
                            let label = format!("Slot {slot}  (Shift+{})", slot % 10);
                            if ui.button(label).clicked() {
                                self.save_state(slot);
                            }
                        }
                    });
                    ui.menu_button("Load state", |ui| {
                        for slot in 1..=10u8 {
                            let exists = self
                                .session
                                .as_ref()
                                .is_some_and(|s| self.store.exists(&s.state_key(slot)));
                            let label = format!("Slot {slot}  ({})", slot % 10);
                            if ui.add_enabled(exists, egui::Button::new(label)).clicked() {
                                self.load_state(slot);
                            }
                        }
                    });
                });
                #[cfg(not(target_arch = "wasm32"))]
                {
                    if ui
                        .add_enabled(loaded, egui::Button::new("Screenshot").shortcut_text("F12"))
                        .clicked()
                    {
                        self.screenshot();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            });

            ui.menu_button("Emulation", |ui| {
                let loaded = self.session.is_some();
                ui.add_enabled_ui(loaded, |ui| {
                    let paused = self.session.as_ref().is_some_and(|s| !s.running());
                    let label = if paused { "Resume" } else { "Pause" };
                    if ui
                        .add(egui::Button::new(label).shortcut_text("Esc"))
                        .clicked()
                    {
                        self.perform(Action::TogglePause);
                    }
                    if ui
                        .add(egui::Button::new("Frame advance").shortcut_text("."))
                        .clicked()
                    {
                        self.perform(Action::FrameAdvance);
                    }
                    if ui
                        .add(
                            egui::Button::new("Reset")
                                .shortcut_text(ui.ctx().format_shortcut(&RESET)),
                        )
                        .clicked()
                    {
                        self.perform(Action::Reset);
                    }
                    if ui
                        .add(
                            egui::Button::new("Power cycle")
                                .shortcut_text(ui.ctx().format_shortcut(&POWER)),
                        )
                        .clicked()
                    {
                        self.perform(Action::Power);
                    }
                });
                ui.separator();
                ui.menu_button("Speed", |ui| {
                    for s in [0.25f32, 0.5, 1.0, 1.5, 2.0, 3.0] {
                        if ui
                            .radio(self.speed == s, format!("{}%", (s * 100.0) as u32))
                            .clicked()
                        {
                            self.speed = s;
                        }
                    }
                });
                ui.menu_button("Region", |ui| {
                    let mut changed = false;
                    changed |= ui
                        .radio_value(&mut self.settings.region, None, "Auto (from ROM)")
                        .clicked();
                    for r in Region::ALL {
                        changed |= ui
                            .radio_value(&mut self.settings.region, Some(r), r.name())
                            .clicked();
                    }
                    if changed {
                        self.toast("Region applies the next time a ROM is loaded");
                    }
                });
                ui.checkbox(&mut self.settings.rewind_enabled, "Rewind (hold Backspace)");
            });

            self.debug.menu(ui);

            ui.menu_button("Tools", |ui| {
                if ui.button("Cheats (Game Genie)…").clicked() {
                    self.cheats_window.open = true;
                }
                if ui.button("Settings…").clicked() {
                    self.settings_window.open = true;
                }
            });

            ui.menu_button("Help", |ui| {
                if ui.button("Controls & about").clicked() {
                    self.show_about = true;
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            match &self.session {
                Some(s) => {
                    ui.label(&s.name);
                    ui.separator();
                    ui.label(format!("{} · {}", s.mapper_name(), s.nes.region().name()));
                    ui.separator();
                    if let Some(reason) = &s.stopped {
                        ui.label(
                            RichText::new(format!("Stopped: {reason}"))
                                .color(Color32::from_rgb(255, 170, 60)),
                        );
                    } else if s.paused {
                        ui.label(RichText::new("Paused").color(Color32::from_rgb(255, 170, 60)));
                    } else if self.rewinding {
                        ui.label(format!("Rewinding ({:.1}s left)", s.rewind_len_seconds()));
                    } else if self.fast_forward {
                        ui.label(format!("Fast-forward, {:.0} fps", self.fps));
                    } else {
                        ui.label(format!("{:.1} fps", self.fps));
                    }
                    for (port, pad) in self.pads.iter().enumerate() {
                        if pad.0 != 0 {
                            let names: Vec<&str> = Buttons::NAMES
                                .iter()
                                .filter(|(_, bit)| pad.pressed(*bit))
                                .map(|(name, _)| *name)
                                .collect();
                            ui.separator();
                            ui.label(format!("P{} {}", port + 1, names.join(" ")));
                        }
                    }
                }
                None => {
                    ui.label("No ROM loaded");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(e) = &self.audio.error {
                    ui.label(RichText::new("Audio off").color(Color32::GRAY))
                        .on_hover_text(format!("Audio unavailable: {e}"));
                } else if !self.audio.is_active() {
                    ui.label(RichText::new("Click or press a key to enable audio").weak());
                }
                if let Some((msg, until)) = &self.toast
                    && Instant::now() < *until
                {
                    ui.label(RichText::new(msg).italics());
                }
            });
        });
    }

    fn game_view(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
        let Some(tex) = &self.texture else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Drop a .nes file here, or File → Open ROM… (Ctrl+O)")
                        .size(18.0)
                        .color(Color32::GRAY),
                );
            });
            return;
        };
        let lines = if self.settings.crop_overscan {
            224.0
        } else {
            240.0
        };
        let aspect = if self.settings.pixel_aspect {
            8.0 / 7.0
        } else {
            1.0
        };
        let base = egui::vec2(256.0 * aspect, lines);
        let avail = rect.size();
        let mut scale = (avail.x / base.x).min(avail.y / base.y);
        if self.settings.integer_scaling && scale >= 1.0 {
            scale = scale.floor();
        }
        let size = base * scale;
        let img_rect = egui::Rect::from_center_size(rect.center(), size);
        let v0 = if self.settings.crop_overscan {
            8.0 / 240.0
        } else {
            0.0
        };
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, v0), egui::pos2(1.0, 1.0 - v0));
        ui.painter().image(tex.id(), img_rect, uv, Color32::WHITE);
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        egui::Window::new("Controls & about")
            .open(&mut self.show_about)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("nes");
                ui.label("A cycle-accurate NES emulator and debugger.");
                ui.add_space(6.0);
                egui::Grid::new("controls").striped(true).show(ui, |ui| {
                    for (k, v) in [
                        ("W / A / S / D", "D-pad"),
                        ("K / J", "A / B"),
                        ("I / U", "Turbo A / Turbo B"),
                        ("O / L", "Start / Select"),
                        ("Esc", "Pause / resume"),
                        (".", "Frame advance"),
                        ("Tab (hold)", "Fast forward"),
                        ("Backspace (hold)", "Rewind"),
                        ("1-0 / Shift+1-0", "Load / save state slot"),
                        ("Ctrl+R / Ctrl+Shift+R", "Reset / power cycle"),
                        (
                            "F5 / F10 / F11 / Shift+F11",
                            "Continue / step over / into / out",
                        ),
                        ("F12", "Screenshot"),
                    ] {
                        ui.monospace(k);
                        ui.label(v);
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
                ui.label(format!("Saves are stored in {}", self.store.location()));
            });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Open,
    ToggleDebugger,
    Reset,
    Power,
    TogglePause,
    FrameAdvance,
    Screenshot,
    SaveState(u8),
    LoadState(u8),
    Continue,
    StepInto,
    StepOver,
    StepOut,
}

fn action_to_step(a: Action) -> crate::debugger::StepKind {
    use crate::debugger::StepKind;
    match a {
        Action::StepOver => StepKind::Over,
        Action::StepOut => StepKind::Out,
        _ => StepKind::Into,
    }
}

impl eframe::App for NesApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Files dropped on the window or picked from the dialog.
        let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            platform::read_dropped(f, &self.inbox);
        }
        for file in self.inbox.take() {
            self.open_file(file);
        }

        input::release_widget_focus(&ctx);
        self.handle_hotkeys(&ctx);
        self.emulate(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));

        if let Some(s) = self.session.as_mut() {
            self.debug.show_panels(ui, s);
            self.debug.show_windows(&ctx, s, &self.palette);
        }
        if self
            .settings_window
            .show(&ctx, &mut self.settings, &self.input)
        {
            self.palette_dirty = true;
            self.audio.set_volume(if self.settings.muted {
                0.0
            } else {
                self.settings.volume
            });
        }
        self.cheats_window.show(&ctx, self.session.as_mut());
        self.about_window(&ctx);

        if let Some(err) = self.error.clone() {
            egui::Window::new("Error")
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    ui.label(err);
                    if ui.button("OK").clicked() {
                        self.error = None;
                    }
                });
        }

        egui::CentralPanel::no_frame().show(ui, |ui| self.game_view(ui));

        if self.session.as_ref().is_some_and(|s| s.running()) || self.rewinding {
            ctx.request_repaint();
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.drive_capture(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.flush_battery_save();
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
        eframe::set_value(storage, crate::debugger::STORAGE_KEY, &self.debug);
        eframe::set_value(storage, CHEATS_KEY, &self.cheats_window);
        self.store.persist(storage);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.flush_battery_save();
    }

    #[cfg(target_arch = "wasm32")]
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(&mut *self)
    }
}
