//! Hex editor over the CPU and PPU address spaces and raw cartridge memory.

use super::{mono, parse_hex};
use crate::session::Session;
use egui::{Color32, RichText, Sense};
use nes_core::Nes;
use nes_core::cart::Mapper;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Region {
    Cpu,
    Ppu,
    Ram,
    Oam,
    Palette,
    PrgRom,
    PrgRam,
    Chr,
}

impl Region {
    const ALL: [Region; 8] = [
        Region::Cpu,
        Region::Ppu,
        Region::Ram,
        Region::Oam,
        Region::Palette,
        Region::PrgRom,
        Region::PrgRam,
        Region::Chr,
    ];

    fn name(self) -> &'static str {
        match self {
            Region::Cpu => "CPU bus",
            Region::Ppu => "PPU bus",
            Region::Ram => "Internal RAM",
            Region::Oam => "OAM",
            Region::Palette => "Palette RAM",
            Region::PrgRom => "PRG ROM",
            Region::PrgRam => "PRG RAM",
            Region::Chr => "CHR",
        }
    }

    fn size(self, nes: &Nes) -> usize {
        let b = nes.bus.cart.board();
        match self {
            Region::Cpu => 0x10000,
            Region::Ppu => 0x4000,
            Region::Ram => 0x800,
            Region::Oam => 0x100,
            Region::Palette => 0x20,
            Region::PrgRom => b.prg_rom.len(),
            Region::PrgRam => b.prg_ram.len(),
            Region::Chr => b.chr_len(),
        }
    }

    fn read(self, nes: &Nes, a: usize) -> u8 {
        let b = nes.bus.cart.board();
        match self {
            Region::Cpu => nes.peek(a as u16),
            Region::Ppu => nes.bus.ppu.peek_vram(a as u16, &nes.bus.cart),
            Region::Ram => nes.bus.ram[a],
            Region::Oam => nes.bus.ppu.oam[a],
            Region::Palette => nes.bus.ppu.palette_read(0x3F00 + a as u16),
            Region::PrgRom => b.prg_rom[a],
            Region::PrgRam => b.prg_ram[a],
            Region::Chr => {
                if b.chr_rom.is_empty() {
                    b.chr_ram[a]
                } else {
                    b.chr_rom[a]
                }
            }
        }
    }

    fn write(self, nes: &mut Nes, a: usize, v: u8) {
        match self {
            Region::Cpu => nes.bus.poke(a as u16, v),
            Region::Ppu => {
                let bus = &mut nes.bus;
                bus.ppu.poke_vram(a as u16, v, &mut bus.cart);
            }
            Region::Ram => nes.bus.ram[a] = v,
            Region::Oam => nes.bus.ppu.oam[a] = v,
            Region::Palette => nes.bus.ppu.palette[a] = v & 0x3F,
            Region::PrgRom => {
                std::sync::Arc::make_mut(&mut nes.bus.cart.board_mut().prg_rom)[a] = v
            }
            Region::PrgRam => nes.bus.cart.board_mut().prg_ram[a] = v,
            Region::Chr => {
                let b = nes.bus.cart.board_mut();
                if b.chr_rom.is_empty() {
                    b.chr_ram[a] = v;
                } else {
                    std::sync::Arc::make_mut(&mut b.chr_rom)[a] = v;
                }
            }
        }
    }
}

pub struct State {
    region: Region,
    goto: String,
    scroll_to: Option<usize>,
    selected: Option<usize>,
    edit: String,
    /// Last seen value and frames since it changed, for highlighting.
    prev: Vec<u8>,
    age: Vec<u8>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            region: Region::Cpu,
            goto: String::new(),
            scroll_to: None,
            selected: None,
            edit: String::new(),
            prev: Vec::new(),
            age: Vec::new(),
        }
    }
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session, state: &mut State) {
    let size = state.region.size(&s.nes);
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("mem-region")
            .selected_text(state.region.name())
            .show_ui(ui, |ui| {
                for r in Region::ALL {
                    if ui
                        .selectable_value(&mut state.region, r, r.name())
                        .changed()
                    {
                        state.selected = None;
                        state.prev.clear();
                    }
                }
            });
        ui.label("Go to");
        let resp = ui.add(egui::TextEdit::singleline(&mut state.goto).desired_width(60.0));
        if resp.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
            && let Some(a) = parse_hex(&state.goto)
        {
            state.scroll_to = Some(a as usize);
            state.selected = Some(a as usize);
        }
        ui.weak(format!("{size} bytes"));
    });
    if let Some(sel) = state.selected.filter(|&a| a < size) {
        ui.horizontal(|ui| {
            ui.label(mono(format!("${sel:04X} =")));
            let cur = state.region.read(&s.nes, sel);
            let resp = ui.add(
                egui::TextEdit::singleline(&mut state.edit)
                    .desired_width(30.0)
                    .hint_text(format!("{cur:02X}")),
            );
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Write").clicked() || enter)
                && !state.edit.is_empty()
                && let Some(v) = parse_hex(&state.edit).filter(|v| *v <= 0xFF)
            {
                state.region.write(&mut s.nes, sel, v as u8);
                state.edit.clear();
                state.selected = Some((sel + 1).min(size - 1));
            }
            ui.weak("(CPU bus writes go to RAM/cart memory only; no register side effects)");
        });
    }
    ui.separator();
    if size == 0 {
        ui.weak("This cartridge has none.");
        return;
    }

    if state.prev.len() != size {
        state.prev = vec![0; size];
        state.age = vec![255; size];
        for a in 0..size {
            state.prev[a] = state.region.read(&s.nes, a);
        }
    }

    let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + 1.0;
    let rows = size.div_ceil(16);
    let mut area = egui::ScrollArea::vertical().auto_shrink(false);
    if let Some(a) = state.scroll_to.take() {
        let spacing = ui.spacing().item_spacing.y;
        area = area.vertical_scroll_offset((a / 16) as f32 * (row_h + spacing));
    }
    let char_w =
        ui.fonts_mut(|f| f.glyph_width(&egui::TextStyle::Monospace.resolve(ui.style()), '0'));
    area.show_rows(ui, row_h, rows, |ui, range| {
        for row in range {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let base = row * 16;
                ui.label(mono(format!("{base:04X}: ")).weak());
                let mut ascii = String::new();
                for col in 0..16 {
                    let a = base + col;
                    if a >= size {
                        break;
                    }
                    let v = state.region.read(&s.nes, a);
                    if v != state.prev[a] {
                        state.prev[a] = v;
                        state.age[a] = 0;
                    } else {
                        state.age[a] = state.age[a].saturating_add(1);
                    }
                    let mut text = RichText::new(format!("{v:02X}")).monospace();
                    if state.selected == Some(a) {
                        text = text
                            .background_color(Color32::from_rgb(60, 90, 160))
                            .color(Color32::WHITE);
                    } else if state.age[a] < 60 {
                        let k = 255 - state.age[a] as u32 * 3;
                        text = text.color(Color32::from_rgb(255, k.min(255) as u8, 80));
                    }
                    let resp = ui.add(egui::Label::new(text).sense(Sense::click()));
                    if resp.clicked() {
                        state.selected = Some(a);
                        state.edit.clear();
                    }
                    ui.add_space(if col == 7 { char_w * 1.5 } else { char_w * 0.6 });
                    ascii.push(if (0x20..0x7F).contains(&v) {
                        v as char
                    } else {
                        '.'
                    });
                }
                ui.add_space(char_w);
                ui.label(mono(ascii).weak());
            });
        }
    });
}
