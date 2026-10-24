//! Per-channel oscilloscopes, state, and mute switches.

use super::mono;
use crate::session::Session;
use egui::{Color32, Sense, Stroke, pos2, vec2};
use nes_core::apu::{CHANNEL_NAMES, channels::Envelope};

const COLORS: [Color32; 5] = [
    Color32::from_rgb(255, 110, 110),
    Color32::from_rgb(255, 190, 90),
    Color32::from_rgb(110, 200, 255),
    Color32::from_rgb(190, 190, 190),
    Color32::from_rgb(170, 130, 255),
];

fn note_name(freq: f64) -> String {
    if !(20.0..20_000.0).contains(&freq) {
        return String::new();
    }
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let n = (12.0 * (freq / 440.0).log2()).round() as i32 + 69;
    format!("{}{}", NAMES[n.rem_euclid(12) as usize], n / 12 - 1)
}

fn env(e: &Envelope) -> String {
    if e.constant {
        format!("vol {}", e.volume)
    } else {
        format!("env {} (decay {})", e.volume, e.decay)
    }
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session) {
    let apu = &s.nes.bus.apu;
    let cpu_hz = s.nes.region().cpu_clock_hz();
    let scope: Vec<[u8; 5]> = apu.scope().collect();
    let (five, fc_cycle, inhibit) = apu.frame_counter_state();
    ui.label(mono(format!(
        "Frame counter: {}-step, cycle {fc_cycle}{}   frame IRQ: {}   DMC IRQ: {}",
        if five { 5 } else { 4 },
        if inhibit { ", IRQ inhibited" } else { "" },
        apu.frame_irq,
        apu.dmc.irq
    )));
    ui.separator();

    let info: [String; 5] = [
        {
            let p = &apu.pulse1;
            let f = cpu_hz / (16.0 * (p.period as f64 + 1.0));
            format!(
                "duty {}  period {:4}  {f:7.1} Hz {:3}  {}  len {}",
                p.duty,
                p.period,
                note_name(f),
                env(&p.envelope),
                p.length.counter
            )
        },
        {
            let p = &apu.pulse2;
            let f = cpu_hz / (16.0 * (p.period as f64 + 1.0));
            format!(
                "duty {}  period {:4}  {f:7.1} Hz {:3}  {}  len {}",
                p.duty,
                p.period,
                note_name(f),
                env(&p.envelope),
                p.length.counter
            )
        },
        {
            let t = &apu.triangle;
            let f = cpu_hz / (32.0 * (t.period as f64 + 1.0));
            format!(
                "period {:4}  {f:7.1} Hz {:3}  linear {}  len {}",
                t.period,
                note_name(f),
                t.linear,
                t.length.counter
            )
        },
        {
            let n = &apu.noise;
            format!(
                "period #{}  mode {}  {}  len {}",
                n.period_index,
                if n.mode { "short" } else { "long" },
                env(&n.envelope),
                n.length.counter
            )
        },
        {
            let d = &apu.dmc;
            format!(
                "level {:3}  rate #{}  sample ${:04X}+{}  at ${:04X}, {} left{}",
                d.output_level,
                d.rate_index,
                d.sample_addr,
                d.sample_len,
                d.current_addr,
                d.bytes_remaining,
                if d.looping { "  loop" } else { "" }
            )
        },
    ];

    for ch in 0..5 {
        ui.horizontal(|ui| {
            let apu = &mut s.nes.bus.apu;
            let mut on = !apu.mute[ch];
            if ui
                .checkbox(
                    &mut on,
                    egui::RichText::new(CHANNEL_NAMES[ch])
                        .color(COLORS[ch])
                        .strong(),
                )
                .changed()
            {
                apu.mute[ch] = !on;
            }
            ui.label(mono(&info[ch]).small());
        });
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, Color32::from_gray(18));
        let max = if ch == 4 { 127.0 } else { 15.0 };
        let n = scope.len().max(2);
        let points: Vec<egui::Pos2> = scope
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let x = rect.left() + rect.width() * i as f32 / (n - 1) as f32;
                let y = rect.bottom() - 3.0 - (rect.height() - 6.0) * v[ch] as f32 / max;
                pos2(x, y)
            })
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(1.2, COLORS[ch])));
    }
}
