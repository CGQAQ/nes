//! Live view of recently executed instructions, with optional logging to a
//! file in nestest.log format.

use crate::session::Session;
use egui::RichText;

pub struct State {
    auto_scroll: bool,
    #[cfg(not(target_arch = "wasm32"))]
    log: Option<Logger>,
    status: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            auto_scroll: true,
            #[cfg(not(target_arch = "wasm32"))]
            log: None,
            status: None,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct Logger {
    out: std::io::BufWriter<std::fs::File>,
    next: u64,
    path: String,
    lines: u64,
}

impl State {
    pub fn logging(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.log.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session, state: &mut State) {
    #[cfg(not(target_arch = "wasm32"))]
    flush_log(s, state);

    ui.horizontal(|ui| {
        ui.checkbox(&mut state.auto_scroll, "Follow");
        if ui.button("Clear").clicked() {
            s.nes.bus.dbg.trace.clear();
        }
        let mut cap = s.nes.bus.dbg.trace.capacity();
        egui::ComboBox::from_id_salt("trace-cap")
            .selected_text(format!("keep {}k", cap / 1000))
            .show_ui(ui, |ui| {
                for c in [10_000usize, 100_000, 1_000_000] {
                    ui.selectable_value(&mut cap, c, format!("{}k", c / 1000));
                }
            });
        if cap != s.nes.bus.dbg.trace.capacity() {
            s.nes.bus.dbg.trace.set_capacity(cap);
        }
        if ui
            .button("Copy visible tail")
            .on_hover_text("Copy the last 2000 lines")
            .clicked()
        {
            let t = &s.nes.bus.dbg.trace;
            let start = t.len().saturating_sub(2000);
            let text: String = (start..t.len())
                .filter_map(|i| t.get(i))
                .map(|e| e.nestest_line() + "\n")
                .collect();
            ui.ctx().copy_text(text);
        }
        #[cfg(not(target_arch = "wasm32"))]
        log_controls(ui, s, state);
    });
    if let Some(msg) = &state.status {
        ui.weak(msg);
    }
    ui.separator();

    let trace = &s.nes.bus.dbg.trace;
    let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::ScrollArea::both()
        .auto_shrink(false)
        .stick_to_bottom(state.auto_scroll)
        .show_rows(ui, row_h, trace.len(), |ui, range| {
            for i in range {
                if let Some(e) = trace.get(i) {
                    let text = if let Some(name) = e.interrupt {
                        RichText::new(format!(
                            "---- {name} ----                                           PPU:{:3},{:3} CYC:{}",
                            e.scanline, e.dot, e.cycle
                        ))
                        .monospace()
                        .color(egui::Color32::from_rgb(255, 170, 60))
                    } else {
                        RichText::new(e.nestest_line()).monospace()
                    };
                    ui.label(text);
                }
            }
        });
}

#[cfg(not(target_arch = "wasm32"))]
fn log_controls(ui: &mut egui::Ui, s: &mut Session, state: &mut State) {
    if let Some(log) = &state.log {
        if ui.button("Stop logging").clicked() {
            state.status = Some(format!("Wrote {} lines to {}", log.lines, log.path));
            state.log = None;
        }
    } else if ui.button("Log to file…").clicked()
        && let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("{}.log", s.id))
            .add_filter("Trace log", &["log", "txt"])
            .save_file()
    {
        match std::fs::File::create(&path) {
            Ok(f) => {
                state.log = Some(Logger {
                    out: std::io::BufWriter::new(f),
                    next: s.nes.bus.dbg.trace.total(),
                    path: path.display().to_string(),
                    lines: 0,
                });
                s.nes.bus.dbg.trace_enabled = true;
                state.status = Some(format!("Logging to {}", path.display()));
            }
            Err(e) => state.status = Some(format!("Couldn't create log: {e}")),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn flush_log(s: &Session, state: &mut State) {
    use std::io::Write;
    let Some(log) = state.log.as_mut() else {
        return;
    };
    let trace = &s.nes.bus.dbg.trace;
    let oldest = trace.total() - trace.len() as u64;
    if log.next < oldest {
        let _ = writeln!(
            log.out,
            "... {} instructions dropped (trace buffer overflowed) ...",
            oldest - log.next
        );
    }
    for e in trace.since(log.next) {
        let _ = writeln!(log.out, "{}", e.nestest_line());
        log.lines += 1;
    }
    log.next = trace.total();
    let _ = log.out.flush();
}
