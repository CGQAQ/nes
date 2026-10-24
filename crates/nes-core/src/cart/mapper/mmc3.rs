//! Mapper 4 and relatives: Nintendo MMC3 (TxROM), MMC6 (HKROM), TxSROM (118),
//! TQROM (119), and the IRQ-less Namco 108 / DxROM (206) and 88/95/154.
//!
//! The scanline counter is clocked by rising edges of PPU address line A12,
//! filtered so the short low pulses between sprite pattern fetches don't
//! count: A12 has to stay low for a few CPU cycles first. That's how the real
//! chip works, so IRQ timing follows the PPU's actual fetch pattern
//! (including games that fetch sprites from $0000 and background from $1000).

use super::{Mapper, MapperInit};
use crate::cart::board::{Board, Mirroring};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Variant {
    Mmc3,
    /// MMC3A-style IRQ: no IRQ when the counter is reloaded with 0.
    Mmc3A,
    Mmc6,
    /// 118: nametable select from CHR bank bit 7.
    TxSrom,
    /// 119: CHR bank bit 6 selects 8K of CHR RAM.
    Tqrom,
    /// 206: Namco 108 (no IRQ, no mirroring control, no modes).
    Namco108,
    /// 88: Namco 108 with CHR A16 tied to PPU A12.
    Namco88,
    /// 95: Namco 108 with nametable select from CHR bank bit 5.
    Namco95,
    /// 154: like 88, plus one-screen mirroring select.
    Namco154,
}

/// A12 must have been low for this many CPU cycles before a rising edge
/// clocks the counter. The shortest low gap in normal rendering is the 9-dot
/// stretch from the dot-337 nametable fetch to the next line's first pattern
/// fetch (exactly 3 CPU cycles), which must not clock; real scanline edges
/// follow 60+ dots of low.
const A12_LOW_CYCLES: u8 = 4;

#[derive(Clone, Serialize, Deserialize)]
pub struct Mmc3 {
    board: Board,
    variant: Variant,
    select: u8,
    regs: [u8; 8],
    mirroring: u8,
    ram_protect: u8,
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_line: bool,
    a12: bool,
    a12_low: u8,
    four_screen: bool,
}

impl Mmc3 {
    pub fn new(init: MapperInit, variant: Variant) -> Self {
        let four_screen = init.info.mirroring == Mirroring::FourScreen;
        let mut board = init.board;
        if variant == Variant::Tqrom && board.chr_ram.is_empty() {
            board.chr_ram = vec![0; 0x2000];
        }
        if variant == Variant::Mmc6 {
            // MMC6 has 1K of internal RAM at $7000-$7FFF (mirrored).
            board.prg_ram = vec![0; 0x400];
        }
        let mut m = Self {
            board,
            variant,
            select: 0,
            regs: [0, 2, 4, 5, 6, 7, 0, 1],
            mirroring: 0,
            ram_protect: if variant == Variant::Mmc6 { 0 } else { 0x80 },
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_line: false,
            a12: false,
            a12_low: 0,
            four_screen,
        };
        m.update();
        m
    }

    fn namco(&self) -> bool {
        matches!(
            self.variant,
            Variant::Namco108 | Variant::Namco88 | Variant::Namco95 | Variant::Namco154
        )
    }

    fn update(&mut self) {
        let prg_mode = !self.namco() && self.select & 0x40 != 0;
        let chr_invert = !self.namco() && self.select & 0x80 != 0;
        let r = self.regs;
        let b = &mut self.board;

        if prg_mode {
            b.set_prg_rom_8k(1, -2);
            b.set_prg_rom_8k(3, r[6] as i32);
        } else {
            b.set_prg_rom_8k(1, r[6] as i32);
            b.set_prg_rom_8k(3, -2);
        }
        b.set_prg_rom_8k(2, r[7] as i32);
        b.set_prg_rom_8k(4, -1);

        // 1K CHR bank numbers for the eight windows, before inversion.
        let mut banks = [
            r[0] & 0xFE,
            r[0] | 1,
            r[1] & 0xFE,
            r[1] | 1,
            r[2],
            r[3],
            r[4],
            r[5],
        ];
        if matches!(self.variant, Variant::Namco88 | Variant::Namco154) {
            for (i, bank) in banks.iter_mut().enumerate() {
                *bank = if i < 4 { *bank & 0x3F } else { *bank | 0x40 };
            }
        }
        for (slot, &bank) in banks.iter().enumerate() {
            let window = if chr_invert { slot ^ 4 } else { slot };
            if self.variant == Variant::Tqrom && bank & 0x40 != 0 {
                b.set_chr_ram_1k(window, (bank & 0x07) as i32);
            } else {
                let bank = if self.variant == Variant::Tqrom {
                    bank & 0x3F
                } else {
                    bank
                };
                b.set_chr_1k(window, bank as i32);
            }
        }

        match self.variant {
            Variant::TxSrom => {
                // Each 1K CHR bank's bit 7 picks the CIRAM page for the
                // matching nametable.
                let nt = if chr_invert {
                    [r[2], r[3], r[4], r[5]]
                } else {
                    [r[0], r[0], r[1], r[1]]
                };
                for (i, v) in nt.iter().enumerate() {
                    b.set_nt_ciram(i, (*v >> 7) as u32);
                }
            }
            Variant::Namco95 => {
                b.set_nt_ciram(0, ((r[0] >> 5) & 1) as u32);
                b.set_nt_ciram(1, ((r[0] >> 5) & 1) as u32);
                b.set_nt_ciram(2, ((r[1] >> 5) & 1) as u32);
                b.set_nt_ciram(3, ((r[1] >> 5) & 1) as u32);
            }
            Variant::Namco108 | Variant::Namco88 | Variant::Namco154 => {}
            _ if self.four_screen => b.set_mirroring(Mirroring::FourScreen),
            _ => b.set_mirroring(if self.mirroring & 1 != 0 {
                Mirroring::Horizontal
            } else {
                Mirroring::Vertical
            }),
        }

        if self.variant != Variant::Mmc6 && !b.prg_ram.is_empty() {
            if self.ram_protect & 0x80 != 0 {
                b.set_prg_ram_8k(0, 0);
                b.set_prg_writable(0, self.ram_protect & 0x40 == 0);
            } else {
                b.unmap_prg(0);
            }
        }
    }

    fn clock_irq(&mut self) {
        let was = self.irq_counter;
        if self.irq_counter == 0 || self.irq_reload {
            self.irq_counter = self.irq_latch;
        } else {
            self.irq_counter -= 1;
        }
        let fire = if self.variant == Variant::Mmc3A {
            self.irq_counter == 0 && (was != 0 || self.irq_reload)
        } else {
            self.irq_counter == 0
        };
        self.irq_reload = false;
        if fire && self.irq_enabled {
            self.irq_line = true;
        }
    }

    fn watch_a12(&mut self, addr: u16) {
        let a12 = addr & 0x1000 != 0;
        if a12 && !self.a12 && self.a12_low >= A12_LOW_CYCLES {
            self.clock_irq();
        }
        if a12 {
            self.a12_low = 0;
        }
        self.a12 = a12;
    }

    fn has_irq(&self) -> bool {
        !self.namco()
    }

    /// MMC6 internal RAM: $7000-$7FFF, 1K mirrored, two 512-byte halves
    /// with separate read/write enables, all gated by $8000 bit 5.
    fn mmc6_ram_access(&self, addr: u16) -> Option<(usize, bool, bool)> {
        if !(0x7000..0x8000).contains(&addr) || self.select & 0x20 == 0 {
            return None;
        }
        let i = (addr & 0x3FF) as usize;
        let high = i >= 0x200;
        let (read, write) = if high {
            (self.ram_protect & 0x80 != 0, self.ram_protect & 0x40 != 0)
        } else {
            (self.ram_protect & 0x20 != 0, self.ram_protect & 0x10 != 0)
        };
        Some((i, read, write))
    }
}

impl Mapper for Mmc3 {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        match self.variant {
            Variant::Mmc3 => "MMC3",
            Variant::Mmc3A => "MMC3A",
            Variant::Mmc6 => "MMC6",
            Variant::TxSrom => "TxSROM",
            Variant::Tqrom => "TQROM",
            Variant::Namco108 => "Namco 108",
            Variant::Namco88 => "Namco 108 (88)",
            Variant::Namco95 => "Namco 108 (95)",
            Variant::Namco154 => "Namco 108 (154)",
        }
    }

    fn cpu_read(&mut self, addr: u16) -> Option<u8> {
        self.cpu_peek(addr)
    }

    fn cpu_peek(&self, addr: u16) -> Option<u8> {
        if self.variant == Variant::Mmc6 && (0x6000..0x8000).contains(&addr) {
            let (i, read, _) = self.mmc6_ram_access(addr)?;
            let either = self.ram_protect & 0xA0 != 0;
            // Reads of a disabled half return 0 while the other is enabled.
            return if read {
                Some(self.board.prg_ram[i])
            } else if either {
                Some(0)
            } else {
                None
            };
        }
        self.board.read_prg(addr)
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr < 0x8000 {
            if self.variant == Variant::Mmc6 {
                if let Some((i, read, write)) = self.mmc6_ram_access(addr) {
                    if read && write {
                        self.board.prg_ram[i] = val;
                    }
                }
            } else {
                self.board.write_prg(addr, val);
            }
            return;
        }
        let namco = self.namco();
        match (addr & 0xE001, namco) {
            (0x8000, _) => {
                self.select = val;
                if self.variant == Variant::Namco154 {
                    self.board.set_mirroring(if val & 0x40 != 0 {
                        Mirroring::SingleScreenB
                    } else {
                        Mirroring::SingleScreenA
                    });
                }
            }
            (0x8001, _) => {
                let r = (self.select & 7) as usize;
                self.regs[r] = if namco && r >= 6 { val & 0x0F } else { val };
            }
            (0xA000, false) => self.mirroring = val,
            (0xA001, false) => self.ram_protect = val,
            (0xC000, false) => self.irq_latch = val,
            (0xC001, false) => {
                self.irq_counter = 0;
                self.irq_reload = true;
            }
            (0xE000, false) => {
                self.irq_enabled = false;
                self.irq_line = false;
            }
            (0xE001, false) => self.irq_enabled = true,
            _ => {}
        }
        self.update();
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        if self.has_irq() {
            self.watch_a12(addr);
        }
        self.board.read_ppu(addr)
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if self.has_irq() {
            self.watch_a12(addr);
        }
        self.board.write_ppu(addr, val);
    }

    fn ppu_address(&mut self, addr: u16) {
        if self.has_irq() {
            self.watch_a12(addr);
        }
    }

    fn cpu_clock(&mut self) {
        if !self.a12 {
            self.a12_low = self.a12_low.saturating_add(1);
        }
    }

    fn irq(&self) -> bool {
        self.irq_line
    }

    fn save_data(&self) -> Option<Vec<u8>> {
        let b = &self.board;
        (b.battery || self.variant == Variant::Mmc6).then(|| b.prg_ram.clone())
    }

    fn debug_state(&self) -> Vec<(&'static str, String)> {
        let r = self.regs;
        vec![
            ("Bank select", format!("${:02X}", self.select)),
            (
                "R0-R5 (CHR)",
                format!(
                    "{:02X} {:02X} {:02X} {:02X} {:02X} {:02X}",
                    r[0], r[1], r[2], r[3], r[4], r[5]
                ),
            ),
            ("R6-R7 (PRG)", format!("{:02X} {:02X}", r[6], r[7])),
            (
                "Mirroring",
                if self.mirroring & 1 != 0 {
                    "horizontal"
                } else {
                    "vertical"
                }
                .into(),
            ),
            ("PRG RAM", format!("${:02X}", self.ram_protect)),
            (
                "IRQ latch/counter",
                format!("{} / {}", self.irq_latch, self.irq_counter),
            ),
            (
                "IRQ enabled/pending",
                format!("{} / {}", self.irq_enabled, self.irq_line),
            ),
        ]
    }
}
