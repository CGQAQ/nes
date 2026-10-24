//! Registers, flags, timing, and run control.

use super::{StepKind, mono};
use crate::session::Session;
use egui::{Color32, RichText};
use nes_core::cpu::CpuBus;
use nes_core::cpu::flags;

#[derive(Default)]
pub struct State {
    run_to_scanline: i16,
}

fn hex_u8(ui: &mut egui::Ui, v: &mut u8, enabled: bool) {
    ui.add_enabled(
        enabled,
        egui::DragValue::new(v)
            .hexadecimal(2, false, true)
            .prefix("$")
            .speed(0.2),
    );
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session, state: &mut State) -> Option<StepKind> {
    let mut step = None;
    let stopped = !s.running();

    ui.horizontal_wrapped(|ui| {
        if stopped {
            if ui.button("Continue").on_hover_text("F5").clicked() {
                s.stopped = None;
                s.paused = false;
                s.nes.resume();
            }
        } else if ui.button("Pause").on_hover_text("Esc").clicked() {
            s.paused = true;
        }
        if ui
            .button("Step into")
            .on_hover_text("F11: one instruction")
            .clicked()
        {
            step = Some(StepKind::Into);
        }
        if ui
            .button("Step over")
            .on_hover_text("F10: run through a JSR")
            .clicked()
        {
            step = Some(StepKind::Over);
        }
        if ui
            .button("Step out")
            .on_hover_text("Shift+F11: run until RTS/RTI")
            .clicked()
        {
            step = Some(StepKind::Out);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Run to scanline");
        ui.add(egui::DragValue::new(&mut state.run_to_scanline).range(-1..=310));
        if ui.button("Go").clicked() {
            s.nes.begin_run_to_scanline(state.run_to_scanline);
            s.stopped = None;
            s.paused = false;
        }
    });
    match &s.stopped {
        Some(r) => {
            ui.label(RichText::new(format!("Stopped: {r}")).color(Color32::from_rgb(255, 170, 60)))
        }
        None if s.paused => {
            ui.label(RichText::new("Paused").color(Color32::from_rgb(255, 170, 60)))
        }
        None => ui.label(RichText::new("Running").color(Color32::from_rgb(120, 200, 120))),
    };
    ui.separator();

    let editable = stopped;
    let regs = &mut s.nes.cpu.regs;
    egui::Grid::new("regs")
        .num_columns(4)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            ui.label("A");
            hex_u8(ui, &mut regs.a, editable);
            ui.label("X");
            hex_u8(ui, &mut regs.x, editable);
            ui.end_row();
            ui.label("Y");
            hex_u8(ui, &mut regs.y, editable);
            ui.label("SP");
            hex_u8(ui, &mut regs.sp, editable);
            ui.end_row();
            ui.label("PC");
            ui.add_enabled(
                editable,
                egui::DragValue::new(&mut regs.pc)
                    .hexadecimal(4, false, true)
                    .prefix("$"),
            );
            ui.label("P");
            ui.label(mono(format!("${:02X}", regs.p)));
            ui.end_row();
        });

    ui.horizontal(|ui| {
        for (name, bit) in [
            ("N", flags::N),
            ("V", flags::V),
            ("D", flags::D),
            ("I", flags::I),
            ("Z", flags::Z),
            ("C", flags::C),
        ] {
            let mut on = regs.p & bit != 0;
            if ui
                .add_enabled(editable, egui::Checkbox::new(&mut on, name))
                .changed()
            {
                if on {
                    regs.p |= bit;
                } else {
                    regs.p &= !bit;
                }
            }
        }
    });
    ui.separator();

    let nes = &s.nes;
    let ppu = &nes.bus.ppu;
    egui::Grid::new("timing")
        .num_columns(2)
        .spacing([12.0, 2.0])
        .show(ui, |ui| {
            ui.label("CPU cycle");
            ui.label(mono(nes.cycles().to_string()));
            ui.end_row();
            ui.label("Frame");
            ui.label(mono(ppu.frame.to_string()));
            ui.end_row();
            ui.label("Scanline, dot");
            ui.label(mono(format!("{}, {}", ppu.scanline, ppu.dot)));
            ui.end_row();
            ui.label("NMI / IRQ");
            ui.label(mono(format!(
                "{} / {}",
                if nes.bus.nmi_line() { "asserted" } else { "-" },
                if nes.bus.irq_line() { "asserted" } else { "-" },
            )));
            ui.end_row();
            if nes.cpu.is_jammed() {
                ui.label("State");
                ui.label(RichText::new("JAMMED (reset to recover)").color(Color32::RED));
                ui.end_row();
            }
        });
    ui.separator();

    ui.label("Stack");
    let sp = nes.cpu.regs.sp;
    ui.horizontal_wrapped(|ui| {
        for i in 1..=12u16 {
            let addr = 0x0100 | ((sp as u16 + i) & 0xFF);
            if addr <= 0x0100 + sp as u16 {
                break;
            }
            ui.label(mono(format!("{:02X}", nes.peek(addr))))
                .on_hover_text(format!("${addr:04X}"));
        }
        if sp == 0xFF {
            ui.weak("(empty)");
        }
    });
    step
}
