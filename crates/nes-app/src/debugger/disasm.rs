//! Disassembly around PC, with execution breakpoints in the gutter.

use super::{mono, parse_hex};
use crate::session::Session;
use egui::{Color32, RichText, Sense};
use nes_core::Nes;
use nes_core::cpu::disasm::{self, Instruction};
use nes_core::debug::cdl;

pub struct State {
    pub follow_pc: bool,
    view_start: u16,
    goto: String,
    shown: (u16, u16),
    scroll_accum: f32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            follow_pc: true,
            view_start: 0,
            goto: String::new(),
            shown: (0, 0),
            scroll_accum: 0.0,
        }
    }
}

/// CDL flags for a CPU address, when it maps to PRG ROM.
fn cdl_flags(nes: &Nes, addr: u16) -> u8 {
    use nes_core::cart::Mapper;
    nes.bus
        .cart
        .board()
        .prg_rom_offset(addr)
        .and_then(|o| nes.bus.dbg.cdl.get(o).copied())
        .unwrap_or(0)
}

/// Known data (read but never executed) is shown as bytes, not decoded.
fn decode_at(nes: &Nes, addr: u16) -> Instruction {
    let flags = cdl_flags(nes, addr);
    if flags & cdl::DATA != 0 && flags & cdl::CODE == 0 {
        let b = nes.peek(addr);
        return Instruction {
            addr,
            opcode: b,
            bytes: [b, 0, 0],
            len: 1,
        };
    }
    disasm::decode(addr, |a| nes.peek(a))
}

fn is_data(nes: &Nes, ins: &Instruction) -> bool {
    let f = cdl_flags(nes, ins.addr);
    f & cdl::DATA != 0 && f & cdl::CODE == 0
}

/// Finds an address about `n` instructions before `target` whose decoding
/// lines up with `target` (6502 code can't be decoded backwards directly).
fn start_before(nes: &Nes, target: u16, n: u16) -> u16 {
    let mut best = target;
    for back in (1..=n * 3).rev() {
        let start = target.wrapping_sub(back);
        let mut a = start;
        let mut count = 0;
        while a.wrapping_sub(start) < back {
            a = a.wrapping_add(decode_at(nes, a).len);
            count += 1;
        }
        if a == target && count >= n.min(back) {
            // Prefer starts the code/data logger knows are instructions.
            if cdl_flags(nes, start) & cdl::CODE != 0 || best == target {
                best = start;
                if cdl_flags(nes, start) & cdl::CODE != 0 {
                    break;
                }
            }
        }
    }
    best
}

fn label_for(nes: &Nes, addr: u16) -> Option<String> {
    if let Some(name) = disasm::io_register_name(addr) {
        return Some(name.to_string());
    }
    let vec = |v: u16| u16::from_le_bytes([nes.peek(v), nes.peek(v + 1)]);
    if addr == vec(0xFFFA) {
        return Some("NMI".into());
    }
    if addr == vec(0xFFFC) {
        return Some("RESET".into());
    }
    if addr == vec(0xFFFE) {
        return Some("IRQ".into());
    }
    None
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session, state: &mut State) {
    let pc = s.nes.cpu.regs.pc;
    ui.horizontal(|ui| {
        ui.checkbox(&mut state.follow_pc, "Follow PC");
        ui.label("Go to");
        let resp = ui.add(
            egui::TextEdit::singleline(&mut state.goto)
                .desired_width(60.0)
                .hint_text("$C000"),
        );
        if resp.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
            && let Some(a) = parse_hex(&state.goto)
        {
            state.view_start = start_before(&s.nes, a as u16, 4);
            state.follow_pc = false;
        }
        for (name, v) in [("NMI", 0xFFFAu16), ("RESET", 0xFFFC), ("IRQ", 0xFFFE)] {
            if ui.small_button(name).clicked() {
                let a = u16::from_le_bytes([s.nes.peek(v), s.nes.peek(v + 1)]);
                state.view_start = start_before(&s.nes, a, 4);
                state.follow_pc = false;
            }
        }
    });
    let (code, data) = s.nes.bus.dbg.cdl_coverage();
    ui.weak(format!(
        "Code/data logger: {:.1}% code, {:.1}% data of PRG ROM",
        code * 100.0,
        data * 100.0
    ));
    ui.separator();

    let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
    let rows = ((ui.available_height() / row_h).floor() as usize).max(4);

    if state.follow_pc {
        let (lo, hi) = state.shown;
        let visible = if lo <= hi {
            pc >= lo && pc < hi
        } else {
            pc >= lo || pc < hi
        };
        // Re-center when PC leaves the view or nears its bottom.
        let near_end = pc.wrapping_sub(lo) as usize > hi.wrapping_sub(lo) as usize * 3 / 4;
        if !visible || near_end || lo == hi {
            state.view_start = start_before(&s.nes, pc, (rows / 3) as u16);
        }
    }

    // Mouse wheel scrolls by instructions.
    let area = ui.available_rect_before_wrap();
    if ui.rect_contains_pointer(area) {
        state.scroll_accum += ui.input(|i| i.smooth_scroll_delta.y);
        while state.scroll_accum.abs() >= row_h {
            state.follow_pc = false;
            if state.scroll_accum > 0.0 {
                state.view_start = start_before(&s.nes, state.view_start, 1);
                state.scroll_accum -= row_h;
            } else {
                state.view_start = state
                    .view_start
                    .wrapping_add(decode_at(&s.nes, state.view_start).len);
                state.scroll_accum += row_h;
            }
        }
    }

    let mut addr = state.view_start;
    let mut toggle = None;
    for _ in 0..rows {
        let ins = decode_at(&s.nes, addr);
        let has_bp = s.nes.bus.dbg.has_exec_breakpoint(addr);
        let row = ui.horizontal(|ui| {
            let (gutter, painter) = ui.allocate_painter(egui::vec2(16.0, row_h), Sense::click());
            let c = gutter.rect.center();
            if has_bp {
                painter.circle_filled(c, 5.0, Color32::from_rgb(220, 50, 50));
            } else if gutter.hovered() {
                painter.circle_stroke(
                    c,
                    5.0,
                    egui::Stroke::new(1.0, Color32::from_rgb(220, 50, 50)),
                );
            }
            if gutter.clicked() {
                toggle = Some(addr);
            }
            let is_pc = addr == pc;
            let marker = if is_pc { ">" } else { " " };
            let bytes: String = (0..ins.len as usize)
                .map(|i| format!("{:02X} ", ins.bytes[i]))
                .collect();
            let text = if is_data(&s.nes, &ins) {
                format!(".db ${:02X}", ins.bytes[0])
            } else {
                disasm::format_with(&ins, |a| label_for(&s.nes, a))
            };
            let mut line =
                RichText::new(format!("{marker}{addr:04X}  {bytes:<9} {text}")).monospace();
            if is_pc {
                line = line
                    .background_color(Color32::from_rgba_unmultiplied(255, 220, 0, 40))
                    .strong();
            } else if ins.info().illegal && !is_data(&s.nes, &ins) {
                line = line.color(Color32::from_rgb(200, 140, 255));
            }
            ui.label(line);
            if let Some(l) = label_for(&s.nes, addr).filter(|l| l.len() <= 5) {
                ui.label(mono(format!("; {l}")).weak());
            }
        });
        row.response.on_hover_text(format!("${addr:04X}"));
        addr = addr.wrapping_add(ins.len);
    }
    state.shown = (state.view_start, addr);
    if let Some(a) = toggle {
        s.nes.bus.dbg.toggle_exec_breakpoint(a);
    }
}
