//! Cartridge memory and the bank-switching windows every mapper is built on.
//!
//! The CPU side is split into five 8 KiB windows (`$6000`, `$8000`, `$A000`,
//! `$C000`, `$E000`), the PPU pattern space into eight 1 KiB windows, and the
//! nametable space into four 1 KiB windows. Mappers only decide *which* bank
//! each window shows; reads and writes all go through here, which also lets
//! the debugger translate CPU addresses back to ROM offsets.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mirroring {
    /// `$2000 = $2400`, `$2800 = $2C00`: nametables stacked vertically, as
    /// used by vertically scrolling games.
    #[default]
    Horizontal,
    /// `$2000 = $2800`, `$2400 = $2C00`.
    Vertical,
    /// All four nametables show the first 1 KiB of CIRAM.
    SingleScreenA,
    /// All four nametables show the second 1 KiB of CIRAM.
    SingleScreenB,
    /// Four distinct nametables (extra 2 KiB of VRAM on the cartridge).
    FourScreen,
}

impl Mirroring {
    pub fn name(self) -> &'static str {
        match self {
            Mirroring::Horizontal => "Horizontal",
            Mirroring::Vertical => "Vertical",
            Mirroring::SingleScreenA => "Single A",
            Mirroring::SingleScreenB => "Single B",
            Mirroring::FourScreen => "Four-screen",
        }
    }
}

/// Which memory a window is backed by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mem {
    /// Nothing responds: CPU reads return open bus.
    #[default]
    None,
    PrgRom,
    PrgRam,
    ChrRom,
    ChrRam,
    /// The console's 2 KiB nametable RAM (plus 2 KiB more for four-screen).
    Ciram,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    pub mem: Mem,
    pub offset: u32,
    pub writable: bool,
}

impl Window {
    const fn new(mem: Mem, offset: u32, writable: bool) -> Self {
        Self {
            mem,
            offset,
            writable,
        }
    }
}

const PRG_WINDOW: usize = 0x2000;
const CHR_WINDOW: usize = 0x400;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Board {
    #[serde(skip)]
    pub prg_rom: Arc<Vec<u8>>,
    #[serde(skip)]
    pub chr_rom: Arc<Vec<u8>>,
    pub chr_ram: Vec<u8>,
    /// Work RAM and battery-backed save RAM, in that order.
    pub prg_ram: Vec<u8>,
    pub ciram: Vec<u8>,
    pub battery: bool,
    pub mirroring: Mirroring,
    prg: [Window; 5],
    chr: [Window; 8],
    nt: [Window; 4],
}

fn bank_offset(bank: i32, bank_size: usize, mem_size: usize) -> u32 {
    let count = (mem_size / bank_size).max(1) as i32;
    (bank.rem_euclid(count) as usize * bank_size) as u32
}

impl Board {
    pub fn new(
        prg_rom: Vec<u8>,
        chr_rom: Vec<u8>,
        chr_ram_size: usize,
        prg_ram_size: usize,
        mirroring: Mirroring,
        battery: bool,
    ) -> Self {
        let mut board = Board {
            prg_rom: Arc::new(prg_rom),
            chr_rom: Arc::new(chr_rom),
            chr_ram: vec![0; chr_ram_size],
            prg_ram: vec![0; prg_ram_size],
            ciram: vec![0; 0x1000],
            battery,
            mirroring,
            ..Default::default()
        };
        // Common power-on layout: RAM at $6000, first 16K at $8000, last 16K
        // at $C000, first 8K of CHR. Mappers override what they need.
        if prg_ram_size > 0 {
            board.set_prg_ram_8k(0, 0);
        }
        board.set_prg_rom_16k(0x8000, 0);
        board.set_prg_rom_16k(0xC000, -1);
        board.set_chr_8k(0);
        board.set_mirroring(mirroring);
        board
    }

    /// Re-attaches ROM contents after deserializing a save state.
    pub fn attach_rom(&mut self, prg_rom: Arc<Vec<u8>>, chr_rom: Arc<Vec<u8>>) {
        self.prg_rom = prg_rom;
        self.chr_rom = chr_rom;
    }

    pub fn prg_rom_len(&self) -> usize {
        self.prg_rom.len()
    }

    pub fn chr_len(&self) -> usize {
        if self.chr_rom.is_empty() {
            self.chr_ram.len()
        } else {
            self.chr_rom.len()
        }
    }

    pub fn has_chr_ram(&self) -> bool {
        !self.chr_ram.is_empty()
    }

    // ------------------------------------------------------------------
    // CPU side.

    /// `page` 0 is `$6000`, 1 is `$8000` ... 4 is `$E000`.
    pub fn set_prg_rom_8k(&mut self, page: usize, bank: i32) {
        let off = bank_offset(bank, PRG_WINDOW, self.prg_rom.len());
        self.prg[page] = Window::new(Mem::PrgRom, off, false);
    }

    /// `addr` is `$8000` or `$C000`.
    pub fn set_prg_rom_16k(&mut self, addr: u16, bank: i32) {
        let page = ((addr - 0x6000) >> 13) as usize;
        let off = bank_offset(bank, 0x4000, self.prg_rom.len());
        self.prg[page] = Window::new(Mem::PrgRom, off, false);
        self.prg[page + 1] = Window::new(Mem::PrgRom, off + PRG_WINDOW as u32, false);
    }

    pub fn set_prg_rom_32k(&mut self, bank: i32) {
        let off = bank_offset(bank, 0x8000, self.prg_rom.len());
        for i in 0..4 {
            self.prg[1 + i] = Window::new(Mem::PrgRom, off + (i * PRG_WINDOW) as u32, false);
        }
    }

    pub fn set_prg_ram_8k(&mut self, page: usize, bank: i32) {
        if self.prg_ram.is_empty() {
            self.prg[page] = Window::default();
            return;
        }
        let off = bank_offset(bank, PRG_WINDOW.min(self.prg_ram.len()), self.prg_ram.len());
        self.prg[page] = Window::new(Mem::PrgRam, off, true);
    }

    pub fn unmap_prg(&mut self, page: usize) {
        self.prg[page] = Window::default();
    }

    pub fn set_prg_writable(&mut self, page: usize, writable: bool) {
        self.prg[page].writable = writable;
    }

    pub fn prg_window(&self, page: usize) -> Window {
        self.prg[page]
    }

    pub fn set_prg_window(&mut self, page: usize, window: Window) {
        self.prg[page] = window;
    }

    #[inline]
    pub fn read_prg(&self, addr: u16) -> Option<u8> {
        if addr < 0x6000 {
            return None;
        }
        let w = self.prg[((addr - 0x6000) >> 13) as usize];
        let i = w.offset as usize + (addr as usize & (PRG_WINDOW - 1));
        match w.mem {
            Mem::PrgRom => self.prg_rom.get(i).copied(),
            Mem::PrgRam => self.prg_ram.get(i % self.prg_ram.len().max(1)).copied(),
            _ => None,
        }
    }

    #[inline]
    pub fn write_prg(&mut self, addr: u16, val: u8) {
        if addr < 0x6000 {
            return;
        }
        let w = self.prg[((addr - 0x6000) >> 13) as usize];
        if w.mem == Mem::PrgRam && w.writable {
            let len = self.prg_ram.len();
            let i = (w.offset as usize + (addr as usize & (PRG_WINDOW - 1))) % len;
            self.prg_ram[i] = val;
        }
    }

    /// Translates a CPU address to an offset into PRG ROM (for the debugger).
    pub fn prg_rom_offset(&self, addr: u16) -> Option<usize> {
        if addr < 0x6000 {
            return None;
        }
        let w = self.prg[((addr - 0x6000) >> 13) as usize];
        (w.mem == Mem::PrgRom).then(|| w.offset as usize + (addr as usize & (PRG_WINDOW - 1)))
    }

    // ------------------------------------------------------------------
    // PPU pattern tables ($0000-$1FFF).

    fn chr_mem(&self) -> (Mem, usize) {
        if self.chr_rom.is_empty() {
            (Mem::ChrRam, self.chr_ram.len())
        } else {
            (Mem::ChrRom, self.chr_rom.len())
        }
    }

    /// Maps `count` consecutive 1 KiB windows starting at `slot` to a bank of
    /// `count` KiB, from CHR ROM if the board has any, else CHR RAM.
    fn set_chr(&mut self, slot: usize, count: usize, bank: i32) {
        let (mem, len) = self.chr_mem();
        self.set_chr_from(slot, count, mem, bank, len);
    }

    fn set_chr_from(&mut self, slot: usize, count: usize, mem: Mem, bank: i32, len: usize) {
        let off = bank_offset(bank, CHR_WINDOW * count, len);
        let writable = mem == Mem::ChrRam || mem == Mem::Ciram;
        for i in 0..count {
            self.chr[slot + i] = Window::new(mem, off + (i * CHR_WINDOW) as u32, writable);
        }
    }

    pub fn set_chr_1k(&mut self, slot: usize, bank: i32) {
        self.set_chr(slot, 1, bank);
    }
    pub fn set_chr_2k(&mut self, slot: usize, bank: i32) {
        self.set_chr(slot, 2, bank);
    }
    pub fn set_chr_4k(&mut self, slot: usize, bank: i32) {
        self.set_chr(slot, 4, bank);
    }
    pub fn set_chr_8k(&mut self, bank: i32) {
        self.set_chr(0, 8, bank);
    }

    /// Explicitly maps a 1 KiB CHR window to RAM (boards with both ROM and RAM).
    pub fn set_chr_ram_1k(&mut self, slot: usize, bank: i32) {
        let len = self.chr_ram.len();
        self.set_chr_from(slot, 1, Mem::ChrRam, bank, len);
    }

    /// Maps a 1 KiB CHR window to one of the CIRAM pages (Namco 163, ...).
    pub fn set_chr_ciram_1k(&mut self, slot: usize, page: i32) {
        self.set_chr_from(slot, 1, Mem::Ciram, page, 0x800);
    }

    pub fn chr_window(&self, slot: usize) -> Window {
        self.chr[slot]
    }

    // ------------------------------------------------------------------
    // Nametables ($2000-$2FFF, mirrored to $3EFF).

    pub fn set_mirroring(&mut self, m: Mirroring) {
        self.mirroring = m;
        let pages: [u32; 4] = match m {
            Mirroring::Horizontal => [0, 0, 1, 1],
            Mirroring::Vertical => [0, 1, 0, 1],
            Mirroring::SingleScreenA => [0, 0, 0, 0],
            Mirroring::SingleScreenB => [1, 1, 1, 1],
            Mirroring::FourScreen => [0, 1, 2, 3],
        };
        for (i, p) in pages.into_iter().enumerate() {
            self.nt[i] = Window::new(Mem::Ciram, p * 0x400, true);
        }
    }

    /// Points one nametable at a CIRAM page (0..=3).
    pub fn set_nt_ciram(&mut self, slot: usize, page: u32) {
        self.nt[slot] = Window::new(Mem::Ciram, (page & 3) * 0x400, true);
    }

    /// Points one nametable at a 1 KiB bank of CHR ROM (Sunsoft-4, Namco 163).
    pub fn set_nt_chr_rom(&mut self, slot: usize, bank: i32) {
        let len = self.chr_rom.len();
        self.nt[slot] = Window::new(Mem::ChrRom, bank_offset(bank, CHR_WINDOW, len), false);
    }

    pub fn nt_window(&self, slot: usize) -> Window {
        self.nt[slot]
    }

    // ------------------------------------------------------------------

    #[inline]
    fn window_read(&self, w: Window, i: usize) -> u8 {
        match w.mem {
            Mem::ChrRom => self
                .chr_rom
                .get(w.offset as usize + i)
                .copied()
                .unwrap_or(0),
            Mem::ChrRam => {
                let len = self.chr_ram.len().max(1);
                self.chr_ram
                    .get((w.offset as usize + i) % len)
                    .copied()
                    .unwrap_or(0)
            }
            Mem::Ciram => self.ciram[(w.offset as usize + i) & 0xFFF],
            Mem::PrgRam => self
                .prg_ram
                .get(w.offset as usize + i)
                .copied()
                .unwrap_or(0),
            Mem::PrgRom => self
                .prg_rom
                .get(w.offset as usize + i)
                .copied()
                .unwrap_or(0),
            Mem::None => 0,
        }
    }

    #[inline]
    fn window_write(&mut self, w: Window, i: usize, val: u8) {
        if !w.writable {
            return;
        }
        match w.mem {
            Mem::ChrRam => {
                let len = self.chr_ram.len();
                if len > 0 {
                    self.chr_ram[(w.offset as usize + i) % len] = val;
                }
            }
            Mem::Ciram => self.ciram[(w.offset as usize + i) & 0xFFF] = val,
            Mem::PrgRam => {
                if let Some(b) = self.prg_ram.get_mut(w.offset as usize + i) {
                    *b = val;
                }
            }
            _ => {}
        }
    }

    /// PPU read of `$0000-$3EFF`.
    #[inline]
    pub fn read_ppu(&self, addr: u16) -> u8 {
        let addr = addr & 0x3FFF;
        if addr < 0x2000 {
            let w = self.chr[(addr >> 10) as usize];
            self.window_read(w, (addr & 0x3FF) as usize)
        } else {
            let w = self.nt[((addr >> 10) & 3) as usize];
            self.window_read(w, (addr & 0x3FF) as usize)
        }
    }

    #[inline]
    pub fn write_ppu(&mut self, addr: u16, val: u8) {
        let addr = addr & 0x3FFF;
        if addr < 0x2000 {
            let w = self.chr[(addr >> 10) as usize];
            self.window_write(w, (addr & 0x3FF) as usize, val);
        } else {
            let w = self.nt[((addr >> 10) & 3) as usize];
            self.window_write(w, (addr & 0x3FF) as usize, val);
        }
    }

    /// Reads CIRAM directly (for mappers that intercept nametable fetches).
    pub fn read_ciram(&self, page: u32, offset: u16) -> u8 {
        self.ciram[((page & 3) * 0x400 + (offset as u32 & 0x3FF)) as usize]
    }

    pub fn write_ciram(&mut self, page: u32, offset: u16, val: u8) {
        self.ciram[((page & 3) * 0x400 + (offset as u32 & 0x3FF)) as usize] = val;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_banks_count_from_the_end() {
        let prg: Vec<u8> = (0..4).flat_map(|b| vec![b as u8; 0x4000]).collect();
        let mut b = Board::new(prg, vec![], 0x2000, 0x2000, Mirroring::Vertical, false);
        assert_eq!(b.read_prg(0x8000), Some(0));
        assert_eq!(b.read_prg(0xC000), Some(3));
        b.set_prg_rom_16k(0x8000, 5); // wraps to bank 1
        assert_eq!(b.read_prg(0x8000), Some(1));
        b.write_prg(0x6000, 0x42);
        assert_eq!(b.read_prg(0x6000), Some(0x42));
    }

    #[test]
    fn mirroring_layouts() {
        let mut b = Board::new(
            vec![0; 0x8000],
            vec![],
            0x2000,
            0,
            Mirroring::Vertical,
            false,
        );
        b.write_ppu(0x2000, 1);
        assert_eq!(b.read_ppu(0x2800), 1);
        assert_eq!(b.read_ppu(0x2400), 0);
        b.set_mirroring(Mirroring::Horizontal);
        assert_eq!(b.read_ppu(0x2400), 1);
        assert_eq!(b.read_ppu(0x3400), 1);
    }
}
