//! Debugger views of PPU memory. Everything is read with side-effect-free
//! peeks, so drawing these never disturbs emulation (or mapper state).
//! Output pixels are NES color indices (0-63), like the framebuffer.

use super::Ppu;
use crate::cart::{Cartridge, Mapper};

/// Color index of `pixel` (0-3) in palette `pal` (0-3 background, 4-7 sprites).
fn color(ppu: &Ppu, pal: u8, pixel: u8) -> u16 {
    let addr = if pixel == 0 {
        0x3F00
    } else {
        0x3F00 | ((pal as u16) << 2) | pixel as u16
    };
    ppu.palette_read(addr) as u16
}

fn tile_row(cart: &Cartridge, table: u16, tile: u16, row: u16) -> (u8, u8) {
    let addr = table | (tile << 4) | row;
    (cart.ppu_peek(addr), cart.ppu_peek(addr + 8))
}

/// One 128x128 pattern table (`table` 0 or 1) drawn with palette `pal`.
pub fn pattern_table(ppu: &Ppu, cart: &Cartridge, table: u16, pal: u8, out: &mut Vec<u16>) {
    out.clear();
    out.resize(128 * 128, 0);
    let base = table * 0x1000;
    for tile in 0..256u16 {
        let (tx, ty) = ((tile % 16) as usize * 8, (tile / 16) as usize * 8);
        for row in 0..8u16 {
            let (lo, hi) = tile_row(cart, base, tile, row);
            for col in 0..8 {
                let p = ((lo >> (7 - col)) & 1) | (((hi >> (7 - col)) & 1) << 1);
                out[(ty + row as usize) * 128 + tx + col] = color(ppu, pal, p);
            }
        }
    }
}

/// All four nametables as a 512x480 image, as the background would draw them.
pub fn nametables(ppu: &Ppu, cart: &Cartridge, out: &mut Vec<u16>) {
    out.clear();
    out.resize(512 * 480, 0);
    let bg_table = if ppu.ctrl & 0x10 != 0 { 0x1000 } else { 0 };
    for nt in 0..4u16 {
        let base = 0x2000 + nt * 0x400;
        let (ox, oy) = ((nt & 1) as usize * 256, (nt >> 1) as usize * 240);
        for ty in 0..30u16 {
            for tx in 0..32u16 {
                let tile = cart.ppu_peek(base + ty * 32 + tx) as u16;
                let attr = cart.ppu_peek(base + 0x3C0 + (ty / 4) * 8 + tx / 4);
                let shift = ((ty & 2) << 1) | (tx & 2);
                let pal = (attr >> shift) & 3;
                for row in 0..8u16 {
                    let (lo, hi) = tile_row(cart, bg_table, tile, row);
                    let y = oy + (ty * 8 + row) as usize;
                    for col in 0..8 {
                        let p = ((lo >> (7 - col)) & 1) | (((hi >> (7 - col)) & 1) << 1);
                        out[y * 512 + ox + (tx * 8) as usize + col] = color(ppu, pal, p);
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SpriteInfo {
    pub index: u8,
    pub y: u8,
    pub tile: u8,
    pub attr: u8,
    pub x: u8,
}

impl SpriteInfo {
    pub fn palette(&self) -> u8 {
        (self.attr & 3) + 4
    }
    pub fn behind_background(&self) -> bool {
        self.attr & 0x20 != 0
    }
    pub fn flip_h(&self) -> bool {
        self.attr & 0x40 != 0
    }
    pub fn flip_v(&self) -> bool {
        self.attr & 0x80 != 0
    }
}

pub fn sprites(ppu: &Ppu) -> Vec<SpriteInfo> {
    (0..64)
        .map(|i| {
            let o = &ppu.oam[i * 4..i * 4 + 4];
            SpriteInfo {
                index: i as u8,
                y: o[0],
                tile: o[1],
                attr: o[2],
                x: o[3],
            }
        })
        .collect()
}

/// One sprite as an 8x8 or 8x16 image. Transparent pixels are `None`.
pub fn sprite_image(ppu: &Ppu, cart: &Cartridge, s: &SpriteInfo, out: &mut Vec<Option<u16>>) {
    let tall = ppu.ctrl & 0x20 != 0;
    let h = if tall { 16 } else { 8 };
    out.clear();
    out.resize(8 * h, None);
    for row in 0..h as u16 {
        let src_row = if s.flip_v() { h as u16 - 1 - row } else { row };
        let (table, tile) = if tall {
            (
                ((s.tile & 1) as u16) << 12,
                ((s.tile & 0xFE) as u16) + src_row / 8,
            )
        } else {
            (if ppu.ctrl & 0x08 != 0 { 0x1000 } else { 0 }, s.tile as u16)
        };
        let (lo, hi) = tile_row(cart, table, tile, src_row & 7);
        for col in 0..8 {
            let bit = if s.flip_h() { col } else { 7 - col };
            let p = ((lo >> bit) & 1) | (((hi >> bit) & 1) << 1);
            if p != 0 {
                out[row as usize * 8 + col] = Some(color(ppu, s.palette(), p));
            }
        }
    }
}

/// Sprite layer composited at screen positions (256x240, `None` = empty).
pub fn sprite_layer(ppu: &Ppu, cart: &Cartridge, out: &mut Vec<Option<u16>>) {
    out.clear();
    out.resize(256 * 240, None);
    let mut img = Vec::new();
    // Draw back to front so lower OAM indices win, like the hardware.
    for s in sprites(ppu).iter().rev() {
        sprite_image(ppu, cart, s, &mut img);
        let h = img.len() / 8;
        for row in 0..h {
            let y = s.y as usize + 1 + row;
            if y >= 240 {
                continue;
            }
            for col in 0..8 {
                let x = s.x as usize + col;
                if x < 256
                    && let Some(c) = img[row * 8 + col]
                {
                    out[y * 256 + x] = Some(c);
                }
            }
        }
    }
}
