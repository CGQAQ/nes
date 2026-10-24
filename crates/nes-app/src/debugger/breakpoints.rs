//! Breakpoint list: execute / read / write, single addresses or ranges.

use super::{mono, parse_hex};
use crate::session::Session;
use nes_core::debug::{BreakKind, Breakpoint};

pub struct State {
    addr: String,
    kind: BreakKind,
    error: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            addr: String::new(),
            kind: BreakKind::EXEC,
            error: None,
        }
    }
}

fn parse_range(s: &str) -> Option<(u16, u16)> {
    let (a, b) = match s.split_once('-') {
        Some((a, b)) => (parse_hex(a)?, parse_hex(b)?),
        None => {
            let a = parse_hex(s)?;
            (a, a)
        }
    };
    (a <= 0xFFFF && b <= 0xFFFF).then_some((a as u16, b as u16))
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session, state: &mut State) {
    ui.horizontal(|ui| {
        let resp = ui.add(
            egui::TextEdit::singleline(&mut state.addr)
                .desired_width(110.0)
                .hint_text("$8000 or $2000-$2007"),
        );
        ui.checkbox(&mut state.kind.exec, "Exec");
        ui.checkbox(&mut state.kind.read, "Read");
        ui.checkbox(&mut state.kind.write, "Write");
        let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("Add").clicked() || submit {
            match parse_range(&state.addr) {
                Some((start, end)) if state.kind.exec || state.kind.read || state.kind.write => {
                    s.nes.bus.dbg.breakpoints.push(Breakpoint {
                        start,
                        end,
                        kind: state.kind,
                        enabled: true,
                    });
                    s.nes.bus.dbg.update_breakpoints();
                    state.addr.clear();
                    state.error = None;
                }
                Some(_) => state.error = Some("pick at least one of exec/read/write".into()),
                None => state.error = Some("expected a hex address or range".into()),
            }
        }
    });
    if let Some(e) = &state.error {
        ui.colored_label(egui::Color32::from_rgb(255, 120, 120), e);
    }
    ui.weak("Tip: click the gutter in the disassembly to toggle an exec breakpoint.");
    ui.separator();

    let dbg = &mut s.nes.bus.dbg;
    let mut remove = None;
    let mut changed = false;
    if dbg.breakpoints.is_empty() {
        ui.weak("No breakpoints.");
    }
    egui::Grid::new("bps")
        .striped(true)
        .num_columns(4)
        .show(ui, |ui| {
            for (i, bp) in dbg.breakpoints.iter_mut().enumerate() {
                changed |= ui.checkbox(&mut bp.enabled, "").changed();
                let range = if bp.start == bp.end {
                    format!("${:04X}", bp.start)
                } else {
                    format!("${:04X}-${:04X}", bp.start, bp.end)
                };
                ui.label(mono(range));
                ui.label(mono(bp.kind.label()));
                if ui.small_button("x").on_hover_text("Remove").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = remove {
        dbg.breakpoints.remove(i);
        changed = true;
    }
    if changed {
        dbg.update_breakpoints();
    }
    if !dbg.breakpoints.is_empty() && ui.button("Remove all").clicked() {
        dbg.breakpoints.clear();
        dbg.update_breakpoints();
    }
}
