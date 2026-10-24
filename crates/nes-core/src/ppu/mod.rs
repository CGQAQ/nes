//! Dot-accurate 2C02 PPU.
//!
//! The PPU runs one dot per [`Ppu::tick`], performing the same memory fetches
//! as the hardware on the same dots (so mappers that snoop the PPU bus, like
//! MMC2/MMC3/MMC5, see the real access pattern), evaluating sprites with the
//! per-dot state machine (including the sprite-overflow bug), and emitting one
//! pixel per visible dot.

pub mod palette;
pub mod view;

use crate::cart::{Cartridge, Mapper};
use crate::region::Region;
use serde::{Deserialize, Serialize};

pub const WIDTH: usize = 256;
pub const HEIGHT: usize = 240;

/// Frames an unrefreshed open-bus bit takes to decay to 0 (about 0.6 s).
const OPEN_BUS_DECAY_FRAMES: u64 = 36;

mod ctrl {
    pub const NT: u8 = 0x03;
    pub const INC32: u8 = 0x04;
    pub const SPR_TABLE: u8 = 0x08;
    pub const BG_TABLE: u8 = 0x10;
    pub const SPR_16: u8 = 0x20;
    pub const NMI: u8 = 0x80;
}

mod mask {
    pub const GRAY: u8 = 0x01;
    pub const BG_LEFT: u8 = 0x02;
    pub const SPR_LEFT: u8 = 0x04;
    pub const BG: u8 = 0x08;
    pub const SPR: u8 = 0x10;
}

/// Per-dot sprite evaluation state for the next scanline.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SpriteEval {
    /// Pointer into primary OAM (`n * 4 + m`).
    addr: u8,
    /// Next free byte of secondary OAM.
    sec_addr: u8,
    found: u8,
    /// Bytes still to copy (or skip, after overflow) for the current sprite.
    copy_remaining: u8,
    /// All 64 sprites have been looked at.
    done: bool,
    /// The first sprite checked this line was in range.
    sprite0: bool,
    first: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ppu {
    region: Region,

    pub ctrl: u8,
    pub mask: u8,
    pub vblank: bool,
    pub sprite0_hit: bool,
    pub sprite_overflow: bool,
    pub oam_addr: u8,

    /// Loopy registers.
    pub v: u16,
    pub t: u16,
    pub fine_x: u8,
    pub w: bool,
    pending_v: u16,
    v_update_delay: u8,

    read_buffer: u8,
    io_latch: u8,
    io_latch_frame: [u64; 8],

    pub oam: Vec<u8>,
    secondary_oam: [u8; 32],
    oam_latch: u8,
    pub palette: [u8; 32],

    pub scanline: i16,
    pub dot: u16,
    pub frame: u64,
    /// Dots since power-on.
    dots: u64,
    /// `dots` at the last `$2007` read.
    last_data_read: u64,
    /// Master clock this PPU has caught up to.
    master_clock: u64,
    prevent_vblank: bool,
    skip_dot: bool,
    /// Writes to most registers are ignored for about a frame after power-up.
    warmed_up: bool,
    /// Set when VBlank starts; cleared by the owner.
    #[serde(skip)]
    pub frame_complete: bool,

    // Background pipeline.
    nt_latch: u8,
    at_latch: u8,
    pat_lo: u8,
    pat_hi: u8,
    bg_lo: u16,
    bg_hi: u16,
    at_lo: u16,
    at_hi: u16,

    // Sprite units for the line being drawn.
    spr_count: u8,
    spr_zero: bool,
    spr_x: [u8; 8],
    spr_attr: [u8; 8],
    spr_lo: [u8; 8],
    spr_hi: [u8; 8],
    // Results of the evaluation for the next line.
    eval: SpriteEval,
    next_count: u8,
    next_zero: bool,
    fetch_y: u8,
    fetch_tile: u8,

    #[serde(skip, default = "blank_frame")]
    framebuffer: Vec<u16>,
}

fn blank_frame() -> Vec<u16> {
    vec![0; WIDTH * HEIGHT]
}

impl Ppu {
    pub fn new(region: Region) -> Self {
        Self {
            region,
            ctrl: 0,
            mask: 0,
            vblank: false,
            sprite0_hit: false,
            sprite_overflow: false,
            oam_addr: 0,
            v: 0,
            t: 0,
            fine_x: 0,
            w: false,
            pending_v: 0,
            v_update_delay: 0,
            read_buffer: 0,
            io_latch: 0,
            io_latch_frame: [0; 8],
            oam: vec![0; 256],
            secondary_oam: [0xFF; 32],
            oam_latch: 0,
            palette: palette::POWER_UP_PALETTE,
            scanline: 0,
            dot: 0,
            frame: 0,
            dots: 0,
            last_data_read: u64::MAX / 2,
            master_clock: 0,
            prevent_vblank: false,
            skip_dot: false,
            warmed_up: false,
            frame_complete: false,
            nt_latch: 0,
            at_latch: 0,
            pat_lo: 0,
            pat_hi: 0,
            bg_lo: 0,
            bg_hi: 0,
            at_lo: 0,
            at_hi: 0,
            spr_count: 0,
            spr_zero: false,
            spr_x: [0; 8],
            spr_attr: [0; 8],
            spr_lo: [0; 8],
            spr_hi: [0; 8],
            eval: SpriteEval::default(),
            next_count: 0,
            next_zero: false,
            fetch_y: 0xFF,
            fetch_tile: 0xFF,
            framebuffer: blank_frame(),
        }
    }

    /// RESET line (the NES resets the PPU, the Famicom does not).
    pub fn reset(&mut self) {
        self.ctrl = 0;
        self.mask = 0;
        self.w = false;
        self.t = 0;
        self.fine_x = 0;
        self.read_buffer = 0;
        self.warmed_up = false;
        self.scanline = 0;
        self.dot = 0;
    }

    pub fn region(&self) -> Region {
        self.region
    }

    /// Keeps the framebuffer (which is not part of save states) across a load.
    pub(crate) fn take_framebuffer_from(&mut self, other: &mut Ppu) {
        std::mem::swap(&mut self.framebuffer, &mut other.framebuffer);
    }

    /// 256x240 pixels: bits 0-5 are the NES color, bits 6-8 the emphasis bits.
    pub fn framebuffer(&self) -> &[u16] {
        &self.framebuffer
    }

    pub fn nmi_line(&self) -> bool {
        self.vblank && self.ctrl & ctrl::NMI != 0
    }

    pub fn rendering_enabled(&self) -> bool {
        self.mask & (mask::BG | mask::SPR) != 0
    }

    fn rendering_line(&self) -> bool {
        self.scanline < 240
    }

    /// Runs dots until `master` (in master clocks) is reached.
    #[inline]
    pub fn run_until(&mut self, master: u64, cart: &mut Cartridge) {
        let div = self.region.ppu_divider();
        while self.master_clock + div <= master {
            self.tick(cart);
            self.master_clock += div;
        }
    }

    /// Advances one dot.
    pub fn tick(&mut self, cart: &mut Cartridge) {
        self.dots += 1;
        if self.dot >= 340 {
            self.dot = 0;
            self.scanline += 1;
            if self.scanline > self.region.last_scanline() {
                self.scanline = -1;
                self.frame += 1;
                self.warmed_up = true;
            }
        } else {
            self.dot += 1;
        }

        if self.v_update_delay > 0 {
            self.v_update_delay -= 1;
            if self.v_update_delay == 0 {
                self.v = self.pending_v;
                if !(self.rendering_line() && self.rendering_enabled()) {
                    cart.ppu_address(self.v & 0x3FFF);
                }
            }
        }

        if self.rendering_line() {
            self.render_dot(cart);
        } else if self.scanline == self.region.vblank_scanline() && self.dot == 1 {
            if !self.prevent_vblank {
                self.vblank = true;
            }
            self.prevent_vblank = false;
            self.frame_complete = true;
        }
    }

    fn render_dot(&mut self, cart: &mut Cartridge) {
        let dot = self.dot;
        let pre = self.scanline == -1;

        if pre && dot == 1 {
            self.vblank = false;
            self.sprite0_hit = false;
            self.sprite_overflow = false;
        }

        let rendering = self.rendering_enabled();
        if rendering {
            if (2..=257).contains(&dot) || (322..=337).contains(&dot) {
                self.shift_bg();
            }
            if dot & 7 == 1 && ((9..=257).contains(&dot) || dot == 329 || dot == 337) {
                self.reload_bg();
            }
        }

        if !pre && (1..=256).contains(&dot) {
            self.output_pixel((dot - 1) as usize);
        }

        // Odd frames skip the last dot of the pre-render line (NTSC only).
        // Whether rendering is on is sampled a dot before the skip.
        if pre && self.region == Region::Ntsc && self.frame & 1 == 1 {
            if dot == 338 {
                self.skip_dot = rendering;
            } else if dot == 339 && self.skip_dot {
                self.dot = 340;
            }
        }

        if !rendering {
            return;
        }

        // Background fetches.
        if (1..=256).contains(&dot) || (321..=336).contains(&dot) {
            match dot & 7 {
                1 => self.nt_latch = cart.ppu_read(0x2000 | (self.v & 0x0FFF)),
                3 => {
                    let v = self.v;
                    let at = cart
                        .ppu_read(0x23C0 | (v & 0x0C00) | ((v >> 4) & 0x38) | ((v >> 2) & 0x07));
                    let shift = ((v >> 4) & 4) | (v & 2);
                    self.at_latch = (at >> shift) & 3;
                }
                5 => self.pat_lo = cart.ppu_read(self.bg_pattern_addr()),
                7 => self.pat_hi = cart.ppu_read(self.bg_pattern_addr() + 8),
                0 => self.inc_x(),
                _ => {}
            }
        } else if dot == 337 || dot == 339 {
            self.nt_latch = cart.ppu_read(0x2000 | (self.v & 0x0FFF));
        }

        if dot == 256 {
            self.inc_y();
        } else if dot == 257 {
            self.v = (self.v & !0x041F) | (self.t & 0x041F);
        } else if pre && (280..=304).contains(&dot) {
            self.v = (self.v & !0x7BE0) | (self.t & 0x7BE0);
        }

        // Sprites.
        if pre {
            if dot == 1 {
                self.next_count = 0;
                self.next_zero = false;
            }
        } else if (1..=64).contains(&dot) {
            if dot & 1 == 1 {
                self.oam_latch = 0xFF;
            } else {
                self.secondary_oam[((dot >> 1) - 1) as usize] = self.oam_latch;
            }
        } else if (65..=256).contains(&dot) {
            self.evaluate_sprites(dot);
        }

        if (257..=320).contains(&dot) {
            self.oam_addr = 0;
            self.fetch_sprites(cart, dot);
        }
    }

    #[inline]
    fn bg_pattern_addr(&self) -> u16 {
        let table = if self.ctrl & ctrl::BG_TABLE != 0 {
            0x1000
        } else {
            0
        };
        table | ((self.nt_latch as u16) << 4) | ((self.v >> 12) & 7)
    }

    #[inline]
    fn shift_bg(&mut self) {
        self.bg_lo <<= 1;
        self.bg_hi <<= 1;
        self.at_lo <<= 1;
        self.at_hi <<= 1;
    }

    #[inline]
    fn reload_bg(&mut self) {
        self.bg_lo = (self.bg_lo & 0xFF00) | self.pat_lo as u16;
        self.bg_hi = (self.bg_hi & 0xFF00) | self.pat_hi as u16;
        self.at_lo = (self.at_lo & 0xFF00) | if self.at_latch & 1 != 0 { 0xFF } else { 0 };
        self.at_hi = (self.at_hi & 0xFF00) | if self.at_latch & 2 != 0 { 0xFF } else { 0 };
    }

    fn inc_x(&mut self) {
        if self.v & 0x001F == 31 {
            self.v &= !0x001F;
            self.v ^= 0x0400;
        } else {
            self.v += 1;
        }
    }

    fn inc_y(&mut self) {
        if self.v & 0x7000 != 0x7000 {
            self.v += 0x1000;
        } else {
            self.v &= !0x7000;
            let mut y = (self.v & 0x03E0) >> 5;
            if y == 29 {
                y = 0;
                self.v ^= 0x0800;
            } else if y == 31 {
                y = 0;
            } else {
                y += 1;
            }
            self.v = (self.v & !0x03E0) | (y << 5);
        }
    }

    fn output_pixel(&mut self, x: usize) {
        let m = self.mask;
        let mut bg = 0u8;
        let mut bg_pal = 0u8;
        if m & mask::BG != 0 && (x >= 8 || m & mask::BG_LEFT != 0) {
            let bit = 15 - self.fine_x as u16;
            bg = (((self.bg_hi >> bit) & 1) << 1 | ((self.bg_lo >> bit) & 1)) as u8;
            bg_pal = (((self.at_hi >> bit) & 1) << 1 | ((self.at_lo >> bit) & 1)) as u8;
        }

        let mut spr = 0u8;
        let mut spr_attr = 0u8;
        if m & mask::SPR != 0 && (x >= 8 || m & mask::SPR_LEFT != 0) {
            for i in 0..self.spr_count as usize {
                let off = (x as u8).wrapping_sub(self.spr_x[i]);
                if off >= 8 || (x as u16) < self.spr_x[i] as u16 {
                    continue;
                }
                let shift = 7 - off;
                let p = ((self.spr_hi[i] >> shift) & 1) << 1 | ((self.spr_lo[i] >> shift) & 1);
                if p == 0 {
                    continue;
                }
                if i == 0 && self.spr_zero && bg != 0 && x != 255 {
                    self.sprite0_hit = true;
                }
                spr = p;
                spr_attr = self.spr_attr[i];
                break;
            }
        }

        let color = if !self.rendering_enabled() {
            // With rendering off, the backdrop shows the palette entry v
            // points at when v is inside palette RAM.
            if self.v & 0x3F00 == 0x3F00 {
                self.palette_read(self.v)
            } else {
                self.palette_read(0x3F00)
            }
        } else {
            let addr = if spr != 0 && (bg == 0 || spr_attr & 0x20 == 0) {
                0x3F10 | ((spr_attr as u16 & 3) << 2) | spr as u16
            } else if bg != 0 {
                0x3F00 | ((bg_pal as u16) << 2) | bg as u16
            } else {
                0x3F00
            };
            self.palette_read(addr)
        };
        let color = if m & mask::GRAY != 0 {
            color & 0x30
        } else {
            color
        };
        let emphasis = (m as u16 >> 5) << 6;
        let y = self.scanline as usize;
        self.framebuffer[y * WIDTH + x] = color as u16 | emphasis;
    }

    fn sprite_height(&self) -> u8 {
        if self.ctrl & ctrl::SPR_16 != 0 { 16 } else { 8 }
    }

    fn sprite_in_range(&self, y: u8) -> bool {
        let row = (self.scanline as u16).wrapping_sub(y as u16);
        row < self.sprite_height() as u16
    }

    fn evaluate_sprites(&mut self, dot: u16) {
        if dot == 65 {
            self.eval = SpriteEval {
                addr: self.oam_addr,
                first: true,
                ..Default::default()
            };
        }
        if dot & 1 == 1 {
            self.oam_latch = self.oam[self.eval.addr as usize];
            return;
        }
        // Whatever byte was just read is treated as a Y coordinate when the
        // evaluator expects one, even if it isn't (that's the overflow bug).
        let in_range = self.sprite_in_range(self.oam_latch);
        let e = &mut self.eval;
        if e.done {
            // Keeps reading Y of the next sprite and failing to copy it.
            e.addr = e.addr.wrapping_add(4) & 0xFC;
        } else if e.found < 8 {
            self.secondary_oam[(e.sec_addr & 31) as usize] = self.oam_latch;
            if e.copy_remaining > 0 {
                e.copy_remaining -= 1;
                e.sec_addr += 1;
                if e.copy_remaining == 0 {
                    e.found += 1;
                }
                e.addr = e.addr.wrapping_add(1);
                if e.addr == 0 {
                    e.done = true;
                }
            } else {
                let first = e.first;
                e.first = false;
                if in_range {
                    if first {
                        e.sprite0 = true;
                    }
                    e.copy_remaining = 3;
                    e.sec_addr += 1;
                    e.addr = e.addr.wrapping_add(1);
                } else {
                    e.addr = e.addr.wrapping_add(4);
                    if e.addr < 4 {
                        e.done = true;
                    }
                }
            }
        } else if e.copy_remaining > 0 {
            // Reading the rest of the sprite that set the overflow flag.
            e.copy_remaining -= 1;
            e.addr = e.addr.wrapping_add(1);
            if e.copy_remaining == 0 || e.addr == 0 {
                e.done = true;
            }
        } else if in_range {
            self.sprite_overflow = true;
            e.copy_remaining = 3;
            e.addr = e.addr.wrapping_add(1);
        } else {
            // The hardware bug: n and m both increment (m without carry).
            let n = (e.addr & 0xFC).wrapping_add(4);
            let m = e.addr.wrapping_add(1) & 3;
            if n == 0 {
                e.done = true;
            }
            e.addr = n | m;
        }
        if dot == 256 {
            self.next_count = self.eval.found;
            self.next_zero = self.eval.sprite0;
        }
    }

    fn fetch_sprites(&mut self, cart: &mut Cartridge, dot: u16) {
        let slot = ((dot - 257) / 8) as usize;
        let phase = (dot - 257) % 8;
        let base = slot * 4;
        match phase {
            0 => {
                if slot == 0 {
                    self.spr_count = self.next_count;
                    self.spr_zero = self.next_zero;
                }
                self.fetch_y = self.secondary_oam[base];
                self.fetch_tile = self.secondary_oam[base + 1];
                self.spr_attr[slot] = self.secondary_oam[base + 2];
                self.spr_x[slot] = self.secondary_oam[base + 3];
                self.oam_latch = self.fetch_y;
                cart.ppu_read(0x2000 | (self.v & 0x0FFF));
            }
            2 => {
                self.oam_latch = self.spr_attr[slot];
                cart.ppu_read(0x2000 | (self.v & 0x0FFF));
            }
            4 | 6 => {
                let addr = self.sprite_pattern_addr(slot) + if phase == 6 { 8 } else { 0 };
                let mut data = cart.ppu_read(addr);
                if slot >= self.spr_count as usize {
                    data = 0;
                } else if self.spr_attr[slot] & 0x40 != 0 {
                    data = data.reverse_bits();
                }
                if phase == 4 {
                    self.spr_lo[slot] = data;
                } else {
                    self.spr_hi[slot] = data;
                }
                self.oam_latch = self.spr_x[slot];
            }
            _ => {}
        }
    }

    fn sprite_pattern_addr(&self, slot: usize) -> u16 {
        let (tile, y) = if slot < self.spr_count as usize {
            (self.fetch_tile, self.fetch_y)
        } else {
            (0xFF, 0xFF)
        };
        let attr = if slot < self.spr_count as usize {
            self.spr_attr[slot]
        } else {
            0
        };
        let mut row = (self.scanline as u16).wrapping_sub(y as u16) as u8;
        if self.ctrl & ctrl::SPR_16 != 0 {
            row &= 15;
            if attr & 0x80 != 0 {
                row = 15 - row;
            }
            let table = (tile as u16 & 1) << 12;
            let tile = (tile & 0xFE) as u16 + if row >= 8 { 1 } else { 0 };
            table | (tile << 4) | (row as u16 & 7)
        } else {
            row &= 7;
            if attr & 0x80 != 0 {
                row = 7 - row;
            }
            let table = if self.ctrl & ctrl::SPR_TABLE != 0 {
                0x1000
            } else {
                0
            };
            table | ((tile as u16) << 4) | row as u16
        }
    }

    // ------------------------------------------------------------------
    // Palette RAM.

    #[inline]
    fn palette_index(addr: u16) -> usize {
        let mut a = (addr & 0x1F) as usize;
        if a & 0x13 == 0x10 {
            a &= !0x10;
        }
        a
    }

    #[inline]
    pub fn palette_read(&self, addr: u16) -> u8 {
        self.palette[Self::palette_index(addr)] & 0x3F
    }

    fn palette_write(&mut self, addr: u16, val: u8) {
        self.palette[Self::palette_index(addr)] = val & 0x3F;
    }

    // ------------------------------------------------------------------
    // CPU interface ($2000-$2007).

    fn latch(&self) -> u8 {
        let mut v = self.io_latch;
        for bit in 0..8 {
            if self.frame.saturating_sub(self.io_latch_frame[bit]) > OPEN_BUS_DECAY_FRAMES {
                v &= !(1 << bit);
            }
        }
        v
    }

    fn refresh_latch(&mut self, val: u8, bits: u8) {
        self.io_latch = (self.latch() & !bits) | (val & bits);
        for bit in 0..8 {
            if bits & (1 << bit) != 0 {
                self.io_latch_frame[bit] = self.frame;
            }
        }
    }

    fn increment_v(&mut self, cart: &mut Cartridge) {
        if self.rendering_line() && self.rendering_enabled() {
            // During rendering, $2007 access bumps both scroll counters.
            self.inc_x();
            self.inc_y();
        } else {
            let step = if self.ctrl & ctrl::INC32 != 0 { 32 } else { 1 };
            self.v = self.v.wrapping_add(step) & 0x7FFF;
            cart.ppu_address(self.v & 0x3FFF);
        }
    }

    pub fn cpu_read(&mut self, reg: u16, cart: &mut Cartridge) -> u8 {
        match reg & 7 {
            2 => {
                let status = (self.vblank as u8) << 7
                    | (self.sprite0_hit as u8) << 6
                    | (self.sprite_overflow as u8) << 5;
                let r = status | (self.latch() & 0x1F);
                // Reading one dot before VBlank would be set suppresses it
                // (and its NMI) for the whole frame.
                if self.scanline == self.region.vblank_scanline() && self.dot == 0 {
                    self.prevent_vblank = true;
                }
                self.vblank = false;
                self.w = false;
                self.refresh_latch(r, 0xE0);
                r
            }
            4 => {
                let r = if self.rendering_line() && self.rendering_enabled() {
                    self.oam_latch
                } else {
                    let v = self.oam[self.oam_addr as usize];
                    if self.oam_addr & 3 == 2 { v & 0xE3 } else { v }
                };
                self.refresh_latch(r, 0xFF);
                r
            }
            7 => {
                // A read on the cycle right after another (DMA halt repeats,
                // indexed dummy reads) gets the previous value still on the
                // data bus, but the buffer refill and increment still happen.
                let back_to_back = self.dots.wrapping_sub(self.last_data_read) < 6;
                self.last_data_read = self.dots;
                if back_to_back {
                    let stale = self.latch();
                    let addr = self.v & 0x3FFF;
                    self.read_buffer =
                        cart.ppu_read(if addr >= 0x3F00 { addr - 0x1000 } else { addr });
                    self.increment_v(cart);
                    return stale;
                }
                let addr = self.v & 0x3FFF;
                let r = if addr >= 0x3F00 {
                    let gray = if self.mask & mask::GRAY != 0 {
                        0x30
                    } else {
                        0x3F
                    };
                    let p = (self.palette_read(addr) & gray) | (self.latch() & 0xC0);
                    self.read_buffer = cart.ppu_read(addr - 0x1000);
                    self.refresh_latch(p, 0x3F);
                    p
                } else {
                    let r = self.read_buffer;
                    self.read_buffer = cart.ppu_read(addr);
                    self.refresh_latch(r, 0xFF);
                    r
                };
                self.increment_v(cart);
                r
            }
            _ => self.latch(),
        }
    }

    /// Register read without side effects (debugger).
    pub fn cpu_peek(&self, reg: u16) -> u8 {
        match reg & 7 {
            2 => {
                (self.vblank as u8) << 7
                    | (self.sprite0_hit as u8) << 6
                    | (self.sprite_overflow as u8) << 5
                    | (self.latch() & 0x1F)
            }
            4 => self.oam[self.oam_addr as usize],
            7 => {
                let addr = self.v & 0x3FFF;
                if addr >= 0x3F00 {
                    self.palette_read(addr)
                } else {
                    self.read_buffer
                }
            }
            _ => self.latch(),
        }
    }

    pub fn cpu_write(&mut self, reg: u16, val: u8, cart: &mut Cartridge) {
        self.refresh_latch(val, 0xFF);
        match reg & 7 {
            0 => {
                if self.warmed_up {
                    self.ctrl = val;
                    self.t = (self.t & !0x0C00) | ((val as u16 & ctrl::NT as u16) << 10);
                }
            }
            1 => {
                if self.warmed_up {
                    self.mask = val;
                }
            }
            3 => self.oam_addr = val,
            4 => {
                if self.rendering_line() && self.rendering_enabled() {
                    // Glitchy increment of the sprite index only.
                    self.oam_addr = self.oam_addr.wrapping_add(4);
                } else {
                    self.oam[self.oam_addr as usize] = val;
                    self.oam_addr = self.oam_addr.wrapping_add(1);
                }
            }
            5 => {
                if self.warmed_up {
                    if !self.w {
                        self.t = (self.t & !0x001F) | (val as u16 >> 3);
                        self.fine_x = val & 7;
                    } else {
                        self.t = (self.t & !0x73E0)
                            | ((val as u16 & 7) << 12)
                            | ((val as u16 & 0xF8) << 2);
                    }
                    self.w = !self.w;
                }
            }
            6 => {
                if self.warmed_up {
                    if !self.w {
                        self.t = (self.t & 0x00FF) | ((val as u16 & 0x3F) << 8);
                    } else {
                        self.t = (self.t & 0xFF00) | val as u16;
                        // v takes the new value a few dots later.
                        self.pending_v = self.t;
                        self.v_update_delay = 3;
                    }
                    self.w = !self.w;
                }
            }
            7 => {
                let addr = self.v & 0x3FFF;
                if addr >= 0x3F00 {
                    self.palette_write(addr, val);
                } else {
                    cart.ppu_write(addr, val);
                }
                self.increment_v(cart);
            }
            // PPUSTATUS is read-only; the write only refreshes the I/O latch.
            _ => {}
        }
    }

    /// OAM DMA writes go through $2004.
    pub fn oam_dma_write(&mut self, val: u8, cart: &mut Cartridge) {
        self.cpu_write(4, val, cart);
    }

    // ------------------------------------------------------------------
    // Debugger views.

    pub fn peek_vram(&self, addr: u16, cart: &Cartridge) -> u8 {
        let addr = addr & 0x3FFF;
        if addr >= 0x3F00 {
            self.palette_read(addr)
        } else {
            cart.ppu_peek(addr)
        }
    }

    pub fn poke_vram(&mut self, addr: u16, val: u8, cart: &mut Cartridge) {
        let addr = addr & 0x3FFF;
        if addr >= 0x3F00 {
            self.palette_write(addr, val)
        } else {
            cart.ppu_write(addr, val)
        }
    }

    /// Scroll position (x, y) in the 512x480 nametable space, as the next
    /// frame will start drawing it.
    pub fn scroll(&self) -> (u16, u16) {
        let t = self.t;
        let x = ((t & 0x1F) << 3) | self.fine_x as u16 | ((t >> 10) & 1) << 8;
        let y = (((t >> 5) & 0x1F) << 3) | ((t >> 12) & 7) | (((t >> 11) & 1) * 240);
        (x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cart::ines::parse;
    use crate::cart::mapper::create;

    fn nrom() -> Cartridge {
        let mut data = vec![b'N', b'E', b'S', 0x1A, 1, 1, 0, 0];
        data.resize(16 + 0x4000 + 0x2000, 0);
        create(parse(&data).unwrap()).unwrap()
    }

    #[test]
    fn frame_has_expected_length() {
        let mut cart = nrom();
        let mut ppu = Ppu::new(Region::Ntsc);
        // Rendering off: every frame is 341 * 262 dots.
        let mut dots = 0u64;
        ppu.tick(&mut cart);
        while !(ppu.scanline == 241 && ppu.dot == 1) {
            ppu.tick(&mut cart);
        }
        ppu.frame_complete = false;
        while !ppu.frame_complete {
            ppu.tick(&mut cart);
            dots += 1;
        }
        assert_eq!(dots, 341 * 262);
    }

    #[test]
    fn scroll_registers() {
        let mut cart = nrom();
        let mut ppu = Ppu::new(Region::Ntsc);
        ppu.warmed_up = true;
        ppu.cpu_write(0, 0x01, &mut cart);
        ppu.cpu_write(5, 0x7D, &mut cart);
        ppu.cpu_write(5, 0x5E, &mut cart);
        assert_eq!(ppu.fine_x, 5);
        assert_eq!(ppu.scroll(), (256 + 0x7D, 0x5E));
    }

    #[test]
    fn palette_mirrors() {
        let mut cart = nrom();
        let mut ppu = Ppu::new(Region::Ntsc);
        ppu.poke_vram(0x3F10, 0x21, &mut cart);
        assert_eq!(ppu.peek_vram(0x3F00, &cart), 0x21);
        ppu.poke_vram(0x3F11, 0x05, &mut cart);
        assert_eq!(ppu.peek_vram(0x3F01, &cart), ppu.palette[1]);
        assert_eq!(ppu.peek_vram(0x3F11, &cart), 0x05);
    }
}
