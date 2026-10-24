//! Mapper 5: Nintendo MMC5 (ExROM).
//!
//! The MMC5 has no connection to the PPU besides the bus, so like the real
//! chip this watches PPU fetches: three identical nametable reads in a row
//! mark the start of a scanline (the PPU does that at dots 337, 339 and the
//! next line's dot 1), and the position of each fetch within the line says
//! whether it's for background or sprites — which matters because 8x16
//! sprites get their own CHR bank set. It also snoops PPUCTRL/PPUMASK.
//!
//! Not emulated: vertical split-screen mode (used by one game).

use super::{Mapper, MapperInit};
use crate::apu::channels::Pulse;
use crate::cart::board::{Board, Mem, Window};
use serde::{Deserialize, Serialize};

/// Fetches per scanline: 128 background, 32 sprite, 8 background, 2 nametable.
const FETCHES_PER_LINE: u16 = 170;

#[derive(Clone, Serialize, Deserialize)]
pub struct Mmc5 {
    board: Board,
    prg_mode: u8,
    chr_mode: u8,
    ram_protect: [u8; 2],
    exram_mode: u8,
    nt_mapping: u8,
    fill_tile: u8,
    fill_attr: u8,
    prg_regs: [u8; 5],
    chr_a: [u16; 8],
    chr_b: [u16; 4],
    chr_upper: u8,
    /// Which CHR set was written last (used outside 8x16 rendering).
    last_set_b: bool,
    exram: Vec<u8>,
    irq_compare: u8,
    irq_enabled: bool,
    irq_pending: bool,
    in_frame: bool,
    scanline: u8,
    multiplicand: u8,
    multiplier: u8,
    // PPU snooping.
    sprites_8x16: bool,
    rendering: bool,
    last_nt_addr: u16,
    nt_matches: u8,
    fetch_index: u16,
    idle_cycles: u8,
    /// Extended-attribute byte for the background tile being fetched.
    ext_attr: u8,
    // Audio.
    pulse: [Pulse; 2],
    pcm: u8,
    audio_divider: u16,
}

impl Mmc5 {
    pub fn new(init: MapperInit) -> Self {
        let mut board = init.board;
        // iNES headers can't describe MMC5's up-to-64K PRG RAM; provide it all.
        if board.prg_ram.len() < 0x10000 && init.info.format != crate::cart::HeaderFormat::Nes2 {
            board.prg_ram.resize(0x10000, 0);
        }
        let mut m = Self {
            board,
            prg_mode: 3,
            chr_mode: 0,
            ram_protect: [0, 0],
            exram_mode: 0,
            nt_mapping: 0,
            fill_tile: 0,
            fill_attr: 0,
            prg_regs: [0, 0, 0, 0, 0xFF],
            chr_a: [0; 8],
            chr_b: [0; 4],
            chr_upper: 0,
            last_set_b: false,
            exram: vec![0; 1024],
            irq_compare: 0,
            irq_enabled: false,
            irq_pending: false,
            in_frame: false,
            scanline: 0,
            multiplicand: 0xFF,
            multiplier: 0xFF,
            sprites_8x16: false,
            rendering: false,
            last_nt_addr: 0,
            nt_matches: 0,
            fetch_index: 0,
            idle_cycles: 0,
            ext_attr: 0,
            pulse: [Pulse::new(true), Pulse::new(true)],
            pcm: 0,
            audio_divider: 0,
        };
        m.update_prg();
        m
    }

    fn ram_writable(&self) -> bool {
        self.ram_protect == [2, 1]
    }

    /// Points an 8K CPU window at ROM or RAM per an MMC5 bank register value.
    fn map8(&mut self, page: usize, value: u8, force_rom: bool) {
        let b = &mut self.board;
        if force_rom || value & 0x80 != 0 {
            b.set_prg_rom_8k(page, (value & 0x7F) as i32);
        } else if !b.prg_ram.is_empty() {
            b.set_prg_ram_8k(page, (value & 0x07) as i32);
        } else {
            b.set_prg_window(page, Window::default());
        }
    }

    fn update_prg(&mut self) {
        let r = self.prg_regs;
        self.map8(0, r[0] & 0x07, false);
        match self.prg_mode & 3 {
            0 => {
                let base = r[4] & 0x7C;
                for i in 0..4 {
                    self.map8(1 + i, 0x80 | (base + i as u8), true);
                }
            }
            1 => {
                let lo = r[2] & 0xFE;
                self.map8(1, lo, false);
                self.map8(2, lo | 1, false);
                let hi = r[4] & 0x7E;
                self.map8(3, 0x80 | hi, true);
                self.map8(4, 0x80 | hi | 1, true);
            }
            2 => {
                let lo = r[2] & 0xFE;
                self.map8(1, lo, false);
                self.map8(2, lo | 1, false);
                self.map8(3, r[3], false);
                self.map8(4, r[4], true);
            }
            _ => {
                self.map8(1, r[1], false);
                self.map8(2, r[2], false);
                self.map8(3, r[3], false);
                self.map8(4, r[4], true);
            }
        }
        let writable = self.ram_writable();
        for page in 0..4 {
            let w = self.board.prg_window(page);
            if w.mem == Mem::PrgRam {
                self.board.set_prg_writable(page, writable);
            }
        }
    }

    /// Byte offset into CHR memory for a pattern fetch.
    fn chr_offset(&self, addr: u16, sprite_fetch: bool) -> usize {
        let use_b = if self.sprites_8x16 && self.rendering && self.in_frame {
            !sprite_fetch
        } else {
            self.last_set_b
        };
        let a = addr as usize & 0x1FFF;
        let (size, reg) = match self.chr_mode & 3 {
            0 => (0x2000, if use_b { self.chr_b[3] } else { self.chr_a[7] }),
            1 => (
                0x1000,
                if use_b {
                    self.chr_b[3]
                } else {
                    self.chr_a[(a >> 12) * 4 + 3]
                },
            ),
            2 => (
                0x800,
                if use_b {
                    self.chr_b[((a >> 11) & 1) * 2 + 1]
                } else {
                    self.chr_a[(a >> 11) * 2 + 1]
                },
            ),
            _ => (
                0x400,
                if use_b {
                    self.chr_b[(a >> 10) & 3]
                } else {
                    self.chr_a[a >> 10]
                },
            ),
        };
        reg as usize * size + (a & (size - 1))
    }

    fn chr_read(&self, offset: usize) -> u8 {
        let b = &self.board;
        if b.chr_rom.is_empty() {
            let len = b.chr_ram.len().max(1);
            b.chr_ram.get(offset % len).copied().unwrap_or(0)
        } else {
            b.chr_rom[offset % b.chr_rom.len()]
        }
    }

    fn nt_read(&self, addr: u16) -> u8 {
        let slot = ((addr >> 10) & 3) as u8;
        let offset = addr & 0x3FF;
        match (self.nt_mapping >> (slot * 2)) & 3 {
            0 => self.board.read_ciram(0, offset),
            1 => self.board.read_ciram(1, offset),
            2 => {
                if self.exram_mode <= 1 {
                    self.exram[offset as usize]
                } else {
                    0
                }
            }
            _ => {
                if offset < 0x3C0 {
                    self.fill_tile
                } else {
                    let a = self.fill_attr & 3;
                    a | (a << 2) | (a << 4) | (a << 6)
                }
            }
        }
    }

    fn nt_write(&mut self, addr: u16, val: u8) {
        let slot = ((addr >> 10) & 3) as u8;
        let offset = addr & 0x3FF;
        match (self.nt_mapping >> (slot * 2)) & 3 {
            0 => self.board.write_ciram(0, offset, val),
            1 => self.board.write_ciram(1, offset, val),
            2 if self.exram_mode <= 1 => self.exram[offset as usize] = val,
            _ => {}
        }
    }

    fn detect_scanline(&mut self, addr: u16) {
        if (0x2000..0x3000).contains(&addr) && addr == self.last_nt_addr {
            self.nt_matches += 1;
            if self.nt_matches == 2 {
                if self.in_frame {
                    self.scanline = self.scanline.wrapping_add(1);
                    if self.scanline == self.irq_compare && self.irq_compare != 0 {
                        self.irq_pending = true;
                    }
                } else {
                    self.in_frame = true;
                    self.scanline = 0;
                }
                // This read is the first fetch of the new line.
                self.fetch_index = 0;
            }
        } else {
            self.nt_matches = 0;
        }
        self.last_nt_addr = addr;
    }

    fn audio_output(&self) -> f32 {
        let p = self.pulse[0].output() as f32 + self.pulse[1].output() as f32;
        // Mixed like the APU's pulses (linear approximation), plus PCM.
        p * 0.0099 + self.pcm as f32 * 0.0016
    }
}

impl Mapper for Mmc5 {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        "MMC5"
    }

    fn cpu_read(&mut self, addr: u16) -> Option<u8> {
        match addr {
            0x5204 => {
                let v = (self.irq_pending as u8) << 7 | (self.in_frame as u8) << 6;
                self.irq_pending = false;
                Some(v)
            }
            _ => self.cpu_peek(addr),
        }
    }

    fn cpu_peek(&self, addr: u16) -> Option<u8> {
        match addr {
            0x5015 => Some(
                self.pulse[0].length.active() as u8 | (self.pulse[1].length.active() as u8) << 1,
            ),
            0x5204 => Some((self.irq_pending as u8) << 7 | (self.in_frame as u8) << 6),
            0x5205 => Some((self.multiplicand as u16 * self.multiplier as u16) as u8),
            0x5206 => Some(((self.multiplicand as u16 * self.multiplier as u16) >> 8) as u8),
            0x5C00..=0x5FFF => {
                if self.exram_mode >= 2 {
                    Some(self.exram[(addr - 0x5C00) as usize])
                } else {
                    None
                }
            }
            0x6000..=0xFFFF => self.board.read_prg(addr),
            _ => None,
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x5000..=0x5003 => self.pulse[0].write(addr, val),
            0x5004..=0x5007 => self.pulse[1].write(addr, val),
            0x5011 => {
                if val != 0 {
                    self.pcm = val;
                }
            }
            0x5015 => {
                self.pulse[0].length.set_enabled(val & 1 != 0);
                self.pulse[1].length.set_enabled(val & 2 != 0);
            }
            0x5100 => {
                self.prg_mode = val & 3;
                self.update_prg();
            }
            0x5101 => self.chr_mode = val & 3,
            0x5102 => {
                self.ram_protect[0] = val & 3;
                self.update_prg();
            }
            0x5103 => {
                self.ram_protect[1] = val & 3;
                self.update_prg();
            }
            0x5104 => self.exram_mode = val & 3,
            0x5105 => self.nt_mapping = val,
            0x5106 => self.fill_tile = val,
            0x5107 => self.fill_attr = val & 3,
            0x5113..=0x5117 => {
                self.prg_regs[(addr - 0x5113) as usize] = val;
                self.update_prg();
            }
            0x5120..=0x5127 => {
                self.chr_a[(addr - 0x5120) as usize] = val as u16 | ((self.chr_upper as u16) << 8);
                self.last_set_b = false;
            }
            0x5128..=0x512B => {
                self.chr_b[(addr - 0x5128) as usize] = val as u16 | ((self.chr_upper as u16) << 8);
                self.last_set_b = true;
            }
            0x5130 => self.chr_upper = val & 3,
            0x5203 => self.irq_compare = val,
            0x5204 => self.irq_enabled = val & 0x80 != 0,
            0x5205 => self.multiplicand = val,
            0x5206 => self.multiplier = val,
            0x5C00..=0x5FFF => {
                let i = (addr - 0x5C00) as usize;
                match self.exram_mode {
                    // As nametable/attribute RAM it's only writable while rendering.
                    0 | 1 => self.exram[i] = if self.in_frame { val } else { 0 },
                    2 => self.exram[i] = val,
                    _ => {}
                }
            }
            0x6000..=0xFFFF => self.board.write_prg(addr, val),
            _ => {}
        }
    }

    fn snoop_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x2000 => self.sprites_8x16 = val & 0x20 != 0,
            0x2001 => {
                self.rendering = val & 0x18 != 0;
                if !self.rendering {
                    self.in_frame = false;
                }
            }
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.idle_cycles = 0;
        self.detect_scanline(addr);
        let index = self.fetch_index;
        self.fetch_index = (self.fetch_index + 1).min(FETCHES_PER_LINE);
        let sprite_fetch = (128..160).contains(&index);
        let ext = self.exram_mode == 1 && self.in_frame && self.rendering && !sprite_fetch;

        if addr >= 0x2000 {
            let offset = addr & 0x3FF;
            if ext && offset < 0x3C0 {
                // Tile fetch: remember this tile's extended attribute.
                self.ext_attr = self.exram[offset as usize];
            } else if ext && offset >= 0x3C0 {
                let pal = self.ext_attr >> 6;
                return pal | (pal << 2) | (pal << 4) | (pal << 6);
            }
            return self.nt_read(addr);
        }
        if ext {
            let bank = (self.ext_attr & 0x3F) as usize | ((self.chr_upper as usize) << 6);
            return self.chr_read(bank * 0x1000 + (addr as usize & 0xFFF));
        }
        self.chr_read(self.chr_offset(addr, sprite_fetch))
    }

    fn ppu_peek(&self, addr: u16) -> u8 {
        if addr >= 0x2000 {
            self.nt_read(addr)
        } else {
            self.chr_read(self.chr_offset(addr, false))
        }
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x2000 {
            self.nt_write(addr, val);
        } else if !self.board.chr_ram.is_empty() {
            let off = self.chr_offset(addr, false) % self.board.chr_ram.len();
            self.board.chr_ram[off] = val;
        }
    }

    fn cpu_clock(&mut self) {
        // The PPU stops fetching during vblank (or with rendering off).
        self.idle_cycles = self.idle_cycles.saturating_add(1);
        if self.idle_cycles >= 3 {
            self.in_frame = false;
        }
        for p in &mut self.pulse {
            p.clock_timer();
            p.length.commit();
        }
        // Envelope and length run at a fixed 240 Hz.
        self.audio_divider += 1;
        if self.audio_divider >= 7457 {
            self.audio_divider = 0;
            for p in &mut self.pulse {
                p.envelope.clock();
                p.length.clock();
            }
        }
    }

    fn irq(&self) -> bool {
        self.irq_pending && self.irq_enabled
    }

    fn expansion_audio(&self) -> f32 {
        self.audio_output()
    }

    fn debug_state(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "PRG mode / regs",
                format!("{} / {:02X?}", self.prg_mode, self.prg_regs),
            ),
            (
                "CHR mode",
                format!(
                    "{} (set {})",
                    self.chr_mode,
                    if self.last_set_b { "B" } else { "A" }
                ),
            ),
            (
                "CHR A",
                self.chr_a
                    .iter()
                    .map(|c| format!("{c:03X}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            (
                "CHR B",
                self.chr_b
                    .iter()
                    .map(|c| format!("{c:03X}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            ("ExRAM mode", self.exram_mode.to_string()),
            ("NT mapping", format!("${:02X}", self.nt_mapping)),
            (
                "Scanline IRQ",
                format!(
                    "line {} compare {} in-frame {} {}",
                    self.scanline, self.irq_compare, self.in_frame, self.irq_pending
                ),
            ),
        ]
    }
}
