//! Nametables, pattern tables, sprites, palettes and PPU registers.

use super::{mono, upload};
use crate::session::Session;
use egui::{Color32, Rect, Sense, Stroke, Vec2, pos2, vec2};
use nes_core::Palette;
use nes_core::ppu::view;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Nametables,
    Patterns,
    Sprites,
    Palette,
    Registers,
}

#[derive(Default)]
pub struct State {
    tab: Tab,
    pattern_palette: u8,
    drawn_pattern_palette: Option<u8>,
    show_scroll: bool,
    nt_tex: Option<egui::TextureHandle>,
    pat_tex: [Option<egui::TextureHandle>; 2],
    spr_tex: Option<egui::TextureHandle>,
    spr_layer_tex: Option<egui::TextureHandle>,
    pixels: Vec<u16>,
    rgba: Vec<u8>,
    initialized: bool,
}

fn swatch(ui: &mut egui::Ui, palette: &Palette, color: u8, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let [r, g, b, _] = palette.rgba(color as u16);
    ui.painter()
        .rect_filled(rect, 2.0, Color32::from_rgb(r, g, b));
    ui.painter().rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, Color32::from_gray(60)),
        egui::StrokeKind::Inside,
    );
    resp
}

/// Draws a texture scaled to fit the available width, keeping aspect.
fn fit_image(ui: &mut egui::Ui, tex: &egui::TextureHandle, max_scale: f32) -> Rect {
    let size = tex.size_vec2();
    let scale = (ui.available_width() / size.x).clamp(0.5, max_scale);
    let (rect, _) = ui.allocate_exact_size(size * scale, Sense::hover());
    ui.painter().image(
        tex.id(),
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    rect
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session, palette: &Palette, state: &mut State, refresh: bool) {
    if !state.initialized {
        state.show_scroll = true;
        state.initialized = true;
    }
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.tab, Tab::Nametables, "Nametables");
        ui.selectable_value(&mut state.tab, Tab::Patterns, "Pattern tables");
        ui.selectable_value(&mut state.tab, Tab::Sprites, "Sprites");
        ui.selectable_value(&mut state.tab, Tab::Palette, "Palettes");
        ui.selectable_value(&mut state.tab, Tab::Registers, "Registers");
    });
    ui.separator();
    let ctx = ui.ctx().clone();
    let ppu = &s.nes.bus.ppu;
    let cart = &s.nes.bus.cart;
    // Redraw cached images when something changed (or the first time).
    let need = |t: &Option<egui::TextureHandle>| refresh || t.is_none();

    match state.tab {
        Tab::Nametables => {
            if need(&state.nt_tex) {
                view::nametables(ppu, cart, &mut state.pixels);
                palette.convert(&state.pixels, &mut state.rgba);
                upload(&ctx, &mut state.nt_tex, "ppu-nt", [512, 480], &state.rgba);
            }
            ui.checkbox(
                &mut state.show_scroll,
                "Show scroll window (from t, as of last write)",
            );
            if let Some(tex) = &state.nt_tex {
                egui::ScrollArea::both().show(ui, |ui| {
                    let rect = fit_image(ui, tex, 2.0);
                    if state.show_scroll {
                        let k = rect.width() / 512.0;
                        let (sx, sy) = ppu.scroll();
                        // The 256x240 window wraps around the 512x480 space.
                        for (dx, dy) in [(0.0, 0.0), (-512.0, 0.0), (0.0, -480.0), (-512.0, -480.0)]
                        {
                            let min = pos2(sx as f32 + dx, (sy % 480) as f32 + dy);
                            let r = Rect::from_min_size(min, vec2(256.0, 240.0));
                            let clipped = r
                                .intersect(Rect::from_min_size(pos2(0.0, 0.0), vec2(512.0, 480.0)));
                            if clipped.is_positive() {
                                let screen = Rect::from_min_max(
                                    rect.min + clipped.min.to_vec2() * k,
                                    rect.min + clipped.max.to_vec2() * k,
                                );
                                ui.painter().rect_stroke(
                                    screen,
                                    0.0,
                                    Stroke::new(2.0, Color32::from_rgb(255, 60, 200)),
                                    egui::StrokeKind::Inside,
                                );
                            }
                        }
                    }
                    ui.label(mono(format!(
                        "Mirroring: {}   Background pattern table: ${:04X}",
                        {
                            use nes_core::cart::Mapper;
                            cart.board().mirroring.name()
                        },
                        if ppu.ctrl & 0x10 != 0 { 0x1000 } else { 0 }
                    )));
                });
            }
        }
        Tab::Patterns => {
            ui.horizontal(|ui| {
                ui.label("Palette");
                for p in 0..8u8 {
                    let label = if p < 4 {
                        format!("BG{p}")
                    } else {
                        format!("SP{}", p - 4)
                    };
                    ui.selectable_value(&mut state.pattern_palette, p, label);
                }
            });
            ui.horizontal(|ui| {
                for c in 0..4u16 {
                    let addr = 0x3F00 | ((state.pattern_palette as u16) << 2) | c;
                    swatch(
                        ui,
                        palette,
                        ppu.palette_read(if c == 0 { 0x3F00 } else { addr }),
                        16.0,
                    );
                }
            });
            let changed = state.drawn_pattern_palette != Some(state.pattern_palette);
            state.drawn_pattern_palette = Some(state.pattern_palette);
            for t in 0..2usize {
                if need(&state.pat_tex[t]) || changed {
                    view::pattern_table(
                        ppu,
                        cart,
                        t as u16,
                        state.pattern_palette,
                        &mut state.pixels,
                    );
                    palette.convert(&state.pixels, &mut state.rgba);
                    upload(
                        &ctx,
                        &mut state.pat_tex[t],
                        if t == 0 { "pat0" } else { "pat1" },
                        [128, 128],
                        &state.rgba,
                    );
                }
            }
            ui.horizontal(|ui| {
                for (t, tex) in state.pat_tex.iter().enumerate() {
                    if let Some(tex) = tex {
                        ui.vertical(|ui| {
                            ui.label(mono(format!("${:04X}", t * 0x1000)));
                            let w = (ui.available_width() / 2.0 - 8.0).max(128.0);
                            let rect_size = vec2(w, w);
                            let (rect, resp) = ui.allocate_exact_size(rect_size, Sense::hover());
                            ui.painter().image(
                                tex.id(),
                                rect,
                                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                            if let Some(p) = resp.hover_pos() {
                                let rel = (p - rect.min) / rect.width() * 16.0;
                                let tile = rel.y as usize * 16 + rel.x as usize;
                                resp.on_hover_text(format!(
                                    "Tile ${tile:02X} at ${:04X}",
                                    t * 0x1000 + tile * 16
                                ));
                            }
                        });
                    }
                }
            });
        }
        Tab::Sprites => {
            let sprites = view::sprites(ppu);
            if need(&state.spr_layer_tex) {
                let mut layer = Vec::new();
                view::sprite_layer(ppu, cart, &mut layer);
                state.rgba.clear();
                for p in &layer {
                    match p {
                        Some(c) => state.rgba.extend_from_slice(&palette.rgba(*c)),
                        None => state.rgba.extend_from_slice(&[40, 40, 40, 255]),
                    }
                }
                upload(
                    &ctx,
                    &mut state.spr_layer_tex,
                    "spr-layer",
                    [256, 240],
                    &state.rgba,
                );

                // 8x8 grid of sprite thumbnails, 8x16 cells.
                let mut sheet = vec![0u8; 64 * 128 * 4];
                let mut img = Vec::new();
                for sp in &sprites {
                    view::sprite_image(ppu, cart, sp, &mut img);
                    let h = img.len() / 8;
                    let (cx, cy) = ((sp.index % 8) as usize * 8, (sp.index / 8) as usize * 16);
                    for row in 0..h {
                        for col in 0..8 {
                            let o = ((cy + row) * 64 + cx + col) * 4;
                            let px = match img[row * 8 + col] {
                                Some(c) => palette.rgba(c),
                                None => [24, 24, 24, 255],
                            };
                            sheet[o..o + 4].copy_from_slice(&px);
                        }
                    }
                }
                upload(&ctx, &mut state.spr_tex, "spr-sheet", [64, 128], &sheet);
            }
            ui.horizontal_top(|ui| {
                if let Some(tex) = &state.spr_tex {
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(64.0 * 3.0, 128.0 * 3.0), Sense::hover());
                    ui.painter().image(
                        tex.id(),
                        rect,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    if let Some(p) = resp.hover_pos() {
                        let rel = p - rect.min;
                        let i = (rel.y / 48.0) as usize * 8 + (rel.x / 24.0) as usize;
                        if let Some(sp) = sprites.get(i) {
                            resp.on_hover_text(format!(
                                "#{}  x={} y={}  tile ${:02X}  palette {}{}{}{}",
                                sp.index,
                                sp.x,
                                sp.y,
                                sp.tile,
                                sp.palette() - 4,
                                if sp.flip_h() { "  flipH" } else { "" },
                                if sp.flip_v() { "  flipV" } else { "" },
                                if sp.behind_background() {
                                    "  behind BG"
                                } else {
                                    ""
                                },
                            ));
                        }
                    }
                }
                ui.vertical(|ui| {
                    ui.label("Sprite layer");
                    if let Some(tex) = &state.spr_layer_tex {
                        fit_image(ui, tex, 2.0);
                    }
                });
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .show(ui, |ui| {
                    egui::Grid::new("oam")
                        .striped(true)
                        .num_columns(6)
                        .show(ui, |ui| {
                            for h in ["#", "X", "Y", "Tile", "Attr", "Flags"] {
                                ui.strong(h);
                            }
                            ui.end_row();
                            for sp in &sprites {
                                ui.label(mono(sp.index.to_string()));
                                ui.label(mono(sp.x.to_string()));
                                ui.label(mono(sp.y.to_string()));
                                ui.label(mono(format!("${:02X}", sp.tile)));
                                ui.label(mono(format!("${:02X}", sp.attr)));
                                ui.label(mono(format!(
                                    "pal{} {}{}{}",
                                    sp.palette() - 4,
                                    if sp.flip_h() { "H" } else { "-" },
                                    if sp.flip_v() { "V" } else { "-" },
                                    if sp.behind_background() { "B" } else { "-" }
                                )));
                                ui.end_row();
                            }
                        });
                });
        }
        Tab::Palette => {
            for (name, base) in [("Background", 0x3F00u16), ("Sprites", 0x3F10)] {
                ui.label(name);
                ui.horizontal(|ui| {
                    for i in 0..16u16 {
                        let c = ppu.palette_read(base + i);
                        swatch(ui, palette, c, 26.0)
                            .on_hover_text(format!("${:04X} = ${c:02X}", base + i));
                        if i % 4 == 3 {
                            ui.add_space(8.0);
                        }
                    }
                });
            }
            ui.separator();
            ui.label("All 64 colors");
            for row in 0..4u8 {
                ui.horizontal(|ui| {
                    for col in 0..16u8 {
                        let c = row * 16 + col;
                        swatch(ui, palette, c, 20.0).on_hover_text(format!("${c:02X}"));
                    }
                });
            }
        }
        Tab::Registers => {
            let flag = |b: bool| if b { "1" } else { "0" };
            egui::Grid::new("ppuregs")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    let rows: Vec<(&str, String)> = vec![
                        (
                            "PPUCTRL",
                            format!(
                                "${:02X}  NMI={} spr16={} bg=${:04X} spr=${:04X} inc={}",
                                ppu.ctrl,
                                flag(ppu.ctrl & 0x80 != 0),
                                flag(ppu.ctrl & 0x20 != 0),
                                if ppu.ctrl & 0x10 != 0 { 0x1000 } else { 0 },
                                if ppu.ctrl & 0x08 != 0 { 0x1000 } else { 0 },
                                if ppu.ctrl & 0x04 != 0 { 32 } else { 1 }
                            ),
                        ),
                        (
                            "PPUMASK",
                            format!(
                                "${:02X}  bg={} spr={} bgL={} sprL={} gray={} emph={:03b}",
                                ppu.mask,
                                flag(ppu.mask & 0x08 != 0),
                                flag(ppu.mask & 0x10 != 0),
                                flag(ppu.mask & 0x02 != 0),
                                flag(ppu.mask & 0x04 != 0),
                                flag(ppu.mask & 0x01 != 0),
                                ppu.mask >> 5
                            ),
                        ),
                        (
                            "PPUSTATUS",
                            format!(
                                "vblank={} sprite0={} overflow={}",
                                flag(ppu.vblank),
                                flag(ppu.sprite0_hit),
                                flag(ppu.sprite_overflow)
                            ),
                        ),
                        ("OAMADDR", format!("${:02X}", ppu.oam_addr)),
                        (
                            "v",
                            format!(
                                "${:04X}  coarse X={} Y={} NT={} fineY={}",
                                ppu.v,
                                ppu.v & 0x1F,
                                (ppu.v >> 5) & 0x1F,
                                (ppu.v >> 10) & 3,
                                ppu.v >> 12
                            ),
                        ),
                        ("t", format!("${:04X}", ppu.t)),
                        ("fine X", ppu.fine_x.to_string()),
                        ("w", flag(ppu.w).to_string()),
                        ("Scroll (t)", format!("{:?}", ppu.scroll())),
                        ("Scanline, dot", format!("{}, {}", ppu.scanline, ppu.dot)),
                        ("Frame", ppu.frame.to_string()),
                    ];
                    for (k, v) in rows {
                        ui.label(k);
                        ui.label(mono(v));
                        ui.end_row();
                    }
                });
        }
    }
}
