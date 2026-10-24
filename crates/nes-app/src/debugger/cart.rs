//! Header info, mapper registers and the current bank layout.

use super::mono;
use crate::session::Session;
use nes_core::cart::Mapper;
use nes_core::cart::board::{Mem, Window};

fn window_text(w: Window) -> String {
    match w.mem {
        Mem::None => "open bus".into(),
        Mem::PrgRom => format!("PRG ROM ${:06X}", w.offset),
        Mem::PrgRam => format!(
            "PRG RAM ${:05X}{}",
            w.offset,
            if w.writable { "" } else { " (ro)" }
        ),
        Mem::ChrRom => format!("CHR ROM ${:05X}", w.offset),
        Mem::ChrRam => format!("CHR RAM ${:05X}", w.offset),
        Mem::Ciram => format!("CIRAM ${:03X}", w.offset),
    }
}

pub fn ui(ui: &mut egui::Ui, s: &mut Session) {
    let info = s.nes.rom_info().clone();
    let cart = &s.nes.bus.cart;
    egui::Grid::new("cart-info")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            let rows = [
                ("File", s.name.clone()),
                ("Header", format!("{:?}", info.format)),
                (
                    "Mapper",
                    format!(
                        "{} ({}), submapper {}",
                        info.mapper,
                        cart.name(),
                        info.submapper
                    ),
                ),
                ("PRG ROM", format!("{} KiB", info.prg_rom_size / 1024)),
                (
                    "CHR",
                    if info.chr_rom_size > 0 {
                        format!("{} KiB ROM", info.chr_rom_size / 1024)
                    } else {
                        format!("{} KiB RAM", cart.board().chr_ram.len() / 1024)
                    },
                ),
                (
                    "PRG RAM",
                    format!(
                        "{} KiB{}",
                        cart.board().prg_ram.len() / 1024,
                        if info.battery { " (battery)" } else { "" }
                    ),
                ),
                ("Mirroring", cart.board().mirroring.name().to_string()),
                ("Region", s.nes.region().name().to_string()),
                (
                    "CRC32",
                    format!("{:08X} (PRG {:08X})", info.crc32, info.prg_crc32),
                ),
            ];
            for (k, v) in rows {
                ui.label(k);
                ui.label(mono(v));
                ui.end_row();
            }
        });
    let regs = cart.debug_state();
    if !regs.is_empty() {
        ui.separator();
        ui.strong("Mapper registers");
        egui::Grid::new("mapper-regs")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                for (k, v) in regs {
                    ui.label(k);
                    ui.label(mono(v));
                    ui.end_row();
                }
            });
    }
    ui.separator();
    ui.strong("Bank layout");
    let b = cart.board();
    egui::Grid::new("banks")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            for (i, base) in [0x6000u16, 0x8000, 0xA000, 0xC000, 0xE000]
                .iter()
                .enumerate()
            {
                ui.label(mono(format!("${base:04X}")));
                ui.label(mono(window_text(b.prg_window(i))));
                ui.end_row();
            }
            for i in 0..8 {
                ui.label(mono(format!("PPU ${:04X}", i * 0x400)));
                ui.label(mono(window_text(b.chr_window(i))));
                ui.end_row();
            }
            for i in 0..4 {
                ui.label(mono(format!("PPU ${:04X}", 0x2000 + i * 0x400)));
                ui.label(mono(window_text(b.nt_window(i))));
                ui.end_row();
            }
        });
}
