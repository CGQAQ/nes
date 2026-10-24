//! Debugger UI. The CPU state and disassembly dock on the right, the live
//! instruction trace, hex editor and breakpoint list share a tabbed panel at
//! the bottom, and the PPU/APU/cartridge viewers are floating windows.

mod apu;
mod breakpoints;
mod cart;
mod cpu;
mod disasm;
mod memory;
mod ppu;
mod trace;

use crate::session::Session;
use nes_core::Palette;
use nes_core::debug::BreakOn;
use serde::{Deserialize, Serialize};

pub const STORAGE_KEY: &str = "debugger";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    Into,
    Over,
    Out,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BottomTab {
    #[default]
    Trace,
    Memory,
    Breakpoints,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct DebugWindows {
    /// Show the docked debugger panels.
    pub docked: bool,
    pub bottom_tab: BottomTab,
    pub ppu: bool,
    pub apu: bool,
    pub cart: bool,
    pub break_on: BreakOnSetting,
    pub breakpoints_list: Vec<nes_core::debug::Breakpoint>,
    /// A new frame (or step) happened; views that cache images refresh.
    #[serde(skip)]
    pub frame_needs_refresh: bool,
    #[serde(skip)]
    disasm_state: disasm::State,
    #[serde(skip)]
    trace_state: trace::State,
    #[serde(skip)]
    bp_state: breakpoints::State,
    #[serde(skip)]
    memory_state: memory::State,
    #[serde(skip)]
    ppu_state: ppu::State,
    #[serde(skip)]
    cpu_state: cpu::State,
}

/// Serializable mirror of [`BreakOn`].
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreakOnSetting {
    pub nmi: bool,
    pub irq: bool,
    pub brk: bool,
    pub jam: bool,
}

impl From<BreakOnSetting> for BreakOn {
    fn from(b: BreakOnSetting) -> Self {
        BreakOn {
            nmi: b.nmi,
            irq: b.irq,
            brk: b.brk,
            jam: b.jam,
        }
    }
}

impl Default for DebugWindows {
    fn default() -> Self {
        Self {
            docked: false,
            bottom_tab: BottomTab::Trace,
            ppu: false,
            apu: false,
            cart: false,
            break_on: BreakOnSetting {
                jam: true,
                ..Default::default()
            },
            breakpoints_list: Vec::new(),
            frame_needs_refresh: true,
            disasm_state: Default::default(),
            trace_state: Default::default(),
            bp_state: Default::default(),
            memory_state: Default::default(),
            ppu_state: Default::default(),
            cpu_state: Default::default(),
        }
    }
}

impl DebugWindows {
    /// Applies persisted debugger settings to a freshly loaded game.
    pub fn on_load(&mut self, s: &mut Session) {
        let dbg = &mut s.nes.bus.dbg;
        dbg.break_on = self.break_on.into();
        dbg.breakpoints = self.breakpoints_list.clone();
        dbg.update_breakpoints();
        self.trace_state = Default::default();
        self.disasm_state = Default::default();
        self.frame_needs_refresh = true;
    }

    pub fn step(&mut self, s: &mut Session, kind: StepKind) {
        s.paused = true;
        match kind {
            StepKind::Into => {
                s.nes.step_instruction();
                s.stopped = Some("stepped".into());
            }
            StepKind::Over => {
                s.nes.begin_step_over();
                s.stopped = None;
                s.paused = false;
            }
            StepKind::Out => {
                s.nes.begin_step_out();
                s.stopped = None;
                s.paused = false;
            }
        }
        self.disasm_state.follow_pc = true;
        self.frame_needs_refresh = true;
    }

    pub fn menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Debug", |ui| {
            ui.checkbox(&mut self.docked, "Debugger panels (Ctrl+D)")
                .on_hover_text("CPU, disassembly, instruction trace, memory and breakpoints");
            ui.separator();
            ui.checkbox(&mut self.ppu, "PPU viewer");
            ui.checkbox(&mut self.apu, "APU viewer");
            ui.checkbox(&mut self.cart, "Cartridge");
            ui.separator();
            ui.menu_button("Break on", |ui| {
                ui.checkbox(&mut self.break_on.nmi, "NMI");
                ui.checkbox(&mut self.break_on.irq, "IRQ");
                ui.checkbox(&mut self.break_on.brk, "BRK");
                ui.checkbox(&mut self.break_on.jam, "CPU jam");
            });
        });
    }

    /// Keeps the core's debugger features in sync with what's visible.
    fn sync(&mut self, s: &mut Session) {
        let dbg = &mut s.nes.bus.dbg;
        dbg.break_on = self.break_on.into();
        dbg.trace_enabled =
            (self.docked && self.bottom_tab == BottomTab::Trace) || self.trace_state.logging();
        dbg.cdl_enabled = self.docked;
        // Persist breakpoints across ROM loads and sessions.
        self.breakpoints_list.clone_from(&dbg.breakpoints);
    }

    /// The docked panels; call before the central panel is laid out.
    pub fn show_panels(&mut self, root: &mut egui::Ui, s: &mut Session) {
        self.sync(s);
        if !self.docked {
            return;
        }
        egui::Panel::right("dbg-right")
            .resizable(true)
            .default_size(400.0)
            .show(root, |ui| {
                ui.add_space(4.0);
                if let Some(kind) = cpu::ui(ui, s, &mut self.cpu_state) {
                    self.step(s, kind);
                }
                ui.separator();
                ui.strong("Disassembly");
                disasm::ui(ui, s, &mut self.disasm_state);
            });
        egui::Panel::bottom("dbg-bottom")
            .resizable(true)
            .default_size(260.0)
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut self.bottom_tab,
                        BottomTab::Trace,
                        "Instruction trace",
                    );
                    ui.selectable_value(&mut self.bottom_tab, BottomTab::Memory, "Memory");
                    ui.selectable_value(
                        &mut self.bottom_tab,
                        BottomTab::Breakpoints,
                        "Breakpoints",
                    );
                });
                ui.separator();
                match self.bottom_tab {
                    BottomTab::Trace => trace::ui(ui, s, &mut self.trace_state),
                    BottomTab::Memory => memory::ui(ui, s, &mut self.memory_state),
                    BottomTab::Breakpoints => breakpoints::ui(ui, s, &mut self.bp_state),
                }
            });
    }

    /// Floating viewer windows.
    pub fn show_windows(&mut self, ctx: &egui::Context, s: &mut Session, palette: &Palette) {
        let refresh = std::mem::take(&mut self.frame_needs_refresh) || !s.running();
        let origin = ctx.content_rect().left_top();
        if self.ppu {
            let mut open = true;
            egui::Window::new("PPU viewer")
                .open(&mut open)
                .default_size([540.0, 560.0])
                .default_pos(origin + egui::vec2(20.0, 40.0))
                .show(ctx, |ui| {
                    ppu::ui(ui, s, palette, &mut self.ppu_state, refresh)
                });
            self.ppu = open;
        }
        if self.apu {
            let mut open = true;
            egui::Window::new("APU viewer")
                .open(&mut open)
                .default_size([520.0, 480.0])
                .default_pos(origin + egui::vec2(80.0, 80.0))
                .show(ctx, |ui| apu::ui(ui, s));
            self.apu = open;
        }
        if self.cart {
            let mut open = true;
            egui::Window::new("Cartridge")
                .open(&mut open)
                .default_width(360.0)
                .default_pos(origin + egui::vec2(140.0, 120.0))
                .show(ctx, |ui| cart::ui(ui, s));
            self.cart = open;
        }
    }
}

// ----------------------------------------------------------------------
// Shared helpers.

/// Parses `$1234`, `0x1234` or `1234` as hex.
pub fn parse_hex(s: &str) -> Option<u32> {
    let t = s
        .trim()
        .trim_start_matches('$')
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    u32::from_str_radix(t, 16).ok()
}

/// Uploads (or updates) an RGBA texture.
pub fn upload(
    ctx: &egui::Context,
    slot: &mut Option<egui::TextureHandle>,
    name: &str,
    size: [usize; 2],
    rgba: &[u8],
) {
    let image = egui::ColorImage::from_rgba_unmultiplied(size, rgba);
    match slot {
        Some(t) => t.set(image, egui::TextureOptions::NEAREST),
        None => *slot = Some(ctx.load_texture(name, image, egui::TextureOptions::NEAREST)),
    }
}

pub fn mono(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).monospace()
}
