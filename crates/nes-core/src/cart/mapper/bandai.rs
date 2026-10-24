//! Mappers 16, 153, 159: Bandai FCG-1/2 and LZ93D50, with the serial
//! EEPROMs (24C02 on 16, 24C01 on 159) the games save to, or 8K of
//! battery RAM and a 512K outer bank (153).

use super::{Mapper, MapperInit};
use crate::cart::board::{Board, Mirroring};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum EepromState {
    Idle,
    /// 24C02: waiting for the 1010xxxR device byte.
    Device,
    /// 24C02: word address byte.
    WordAddress,
    /// 24C01: combined 7-bit address + R/W byte.
    Address01,
    Write,
    Read,
}

/// A 24C01/24C02 I²C EEPROM, driven by bit-banged SCL/SDA writes.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Eeprom {
    data: Vec<u8>,
    c01: bool,
    state: EepromState,
    bit: u8,
    shift: u8,
    addr: u8,
    /// Currently in the 9th (acknowledge) clock of a byte.
    ack_slot: bool,
    /// The EEPROM (not the CPU) drives this acknowledge.
    ack_by_eeprom: bool,
    scl: bool,
    sda: bool,
    out: bool,
}

impl Eeprom {
    fn new(c01: bool) -> Self {
        Self {
            data: vec![0xFF; if c01 { 128 } else { 256 }],
            c01,
            state: EepromState::Idle,
            bit: 0,
            shift: 0,
            addr: 0,
            ack_slot: false,
            ack_by_eeprom: false,
            scl: false,
            sda: false,
            out: true,
        }
    }

    fn mask(&self) -> u8 {
        (self.data.len() - 1) as u8
    }

    fn write_lines(&mut self, scl: bool, sda: bool) {
        if self.scl && scl {
            if self.sda && !sda {
                // Start (or repeated start).
                self.state = if self.c01 {
                    EepromState::Address01
                } else {
                    EepromState::Device
                };
                self.bit = 0;
                self.ack_slot = false;
            } else if !self.sda && sda {
                self.state = EepromState::Idle;
                self.out = true;
            }
        } else if !self.scl && scl {
            self.rising(sda);
        } else if self.scl && !scl {
            self.falling();
        }
        self.scl = scl;
        self.sda = sda;
    }

    fn rising(&mut self, sda: bool) {
        if self.state == EepromState::Idle {
            return;
        }
        if self.ack_slot {
            self.ack_slot = false;
            self.bit = 0;
            if self.state == EepromState::Read && !self.ack_by_eeprom {
                if sda {
                    // CPU didn't acknowledge: end of read.
                    self.state = EepromState::Idle;
                    return;
                }
                self.addr = self.addr.wrapping_add(1) & self.mask();
                self.shift = self.data[self.addr as usize];
            }
            return;
        }
        if self.state == EepromState::Read {
            self.bit += 1;
            if self.bit == 8 {
                self.ack_slot = true;
                self.ack_by_eeprom = false;
            }
            return;
        }
        self.shift = (self.shift << 1) | sda as u8;
        self.bit += 1;
        if self.bit < 8 {
            return;
        }
        let byte = self.shift;
        self.ack_slot = true;
        self.ack_by_eeprom = true;
        match self.state {
            EepromState::Device => {
                if byte & 0xF0 != 0xA0 {
                    self.state = EepromState::Idle;
                } else if byte & 1 != 0 {
                    self.state = EepromState::Read;
                    self.shift = self.data[self.addr as usize];
                } else {
                    self.state = EepromState::WordAddress;
                }
            }
            EepromState::WordAddress => {
                self.addr = byte & self.mask();
                self.state = EepromState::Write;
            }
            EepromState::Address01 => {
                self.addr = (byte >> 1) & self.mask();
                if byte & 1 != 0 {
                    self.state = EepromState::Read;
                    self.shift = self.data[self.addr as usize];
                } else {
                    self.state = EepromState::Write;
                }
            }
            EepromState::Write => {
                self.data[self.addr as usize] = byte;
                // Writes wrap within a page (8 bytes on 24C02, 4 on 24C01).
                let page: u8 = if self.c01 { 3 } else { 7 };
                self.addr = (self.addr & !page) | (self.addr.wrapping_add(1) & page);
            }
            _ => {}
        }
    }

    fn falling(&mut self) {
        self.out = match self.state {
            EepromState::Idle => true,
            _ if self.ack_slot => !self.ack_by_eeprom,
            EepromState::Read => (self.shift >> (7 - self.bit.min(7))) & 1 != 0,
            _ => true,
        };
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Bandai {
    board: Board,
    mapper: u16,
    /// Registers decoded at $6000-$7FFF (FCG) and/or $8000-$FFFF (LZ93D50).
    low_regs: bool,
    high_regs: bool,
    latched_irq: bool,
    chr: [u8; 8],
    prg: u8,
    irq_enabled: bool,
    irq_counter: u16,
    irq_latch: u16,
    irq_pending: bool,
    eeprom: Option<Eeprom>,
    eeprom_read_enable: bool,
    ram_enabled: bool,
}

impl Bandai {
    pub fn new(init: MapperInit) -> Self {
        let mapper = init.info.mapper;
        let sub = init.info.submapper;
        let (low_regs, high_regs) = match (mapper, sub) {
            (16, 4) => (true, false),
            (16, 5) | (153, _) | (159, _) => (false, true),
            _ => (true, true),
        };
        let eeprom = match mapper {
            159 => Some(Eeprom::new(true)),
            16 => Some(Eeprom::new(false)),
            _ => None,
        };
        let mut m = Self {
            board: init.board,
            mapper,
            low_regs,
            high_regs,
            latched_irq: high_regs,
            chr: [0; 8],
            prg: 0,
            irq_enabled: false,
            irq_counter: 0,
            irq_latch: 0,
            irq_pending: false,
            eeprom,
            eeprom_read_enable: false,
            ram_enabled: false,
        };
        if mapper == 16 || mapper == 159 {
            // Saves live in the EEPROM, not PRG RAM.
            m.board.prg_ram.clear();
            m.board.unmap_prg(0);
        }
        m.update();
        m
    }

    fn update(&mut self) {
        let b = &mut self.board;
        if self.mapper == 153 {
            let outer = ((self.chr.iter().fold(0, |a, c| a | c) & 1) as i32) << 4;
            b.set_prg_rom_16k(0x8000, outer | (self.prg & 0x0F) as i32);
            b.set_prg_rom_16k(0xC000, outer | 0x0F);
            b.set_chr_8k(0);
            if self.ram_enabled {
                b.set_prg_ram_8k(0, 0);
            } else {
                b.unmap_prg(0);
            }
        } else {
            b.set_prg_rom_16k(0x8000, (self.prg & 0x0F) as i32);
            b.set_prg_rom_16k(0xC000, -1);
            for (i, &bank) in self.chr.iter().enumerate() {
                b.set_chr_1k(i, bank as i32);
            }
        }
    }

    fn write_reg(&mut self, reg: u16, val: u8) {
        match reg {
            0..=7 => self.chr[reg as usize] = val,
            8 => self.prg = val,
            9 => self.board.set_mirroring(match val & 3 {
                0 => Mirroring::Vertical,
                1 => Mirroring::Horizontal,
                2 => Mirroring::SingleScreenA,
                _ => Mirroring::SingleScreenB,
            }),
            0xA => {
                self.irq_enabled = val & 1 != 0;
                if self.latched_irq {
                    self.irq_counter = self.irq_latch;
                }
                self.irq_pending = false;
            }
            0xB => {
                if self.latched_irq {
                    self.irq_latch = (self.irq_latch & 0xFF00) | val as u16;
                } else {
                    self.irq_counter = (self.irq_counter & 0xFF00) | val as u16;
                }
            }
            0xC => {
                if self.latched_irq {
                    self.irq_latch = (self.irq_latch & 0x00FF) | ((val as u16) << 8);
                } else {
                    self.irq_counter = (self.irq_counter & 0x00FF) | ((val as u16) << 8);
                }
            }
            0xD => {
                if let Some(e) = self.eeprom.as_mut() {
                    self.eeprom_read_enable = val & 0x80 != 0;
                    e.write_lines(val & 0x20 != 0, val & 0x40 != 0);
                } else {
                    self.ram_enabled = val & 0x20 != 0;
                }
            }
            _ => {}
        }
        self.update();
    }
}

impl Mapper for Bandai {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        match self.mapper {
            153 => "Bandai LZ93D50 (SRAM)",
            159 => "Bandai LZ93D50 (24C01)",
            _ => "Bandai FCG / LZ93D50",
        }
    }

    fn cpu_read(&mut self, addr: u16) -> Option<u8> {
        self.cpu_peek(addr)
    }

    fn cpu_peek(&self, addr: u16) -> Option<u8> {
        if (0x6000..0x8000).contains(&addr)
            && let Some(e) = &self.eeprom
        {
            let bit = if self.eeprom_read_enable && e.out {
                0x10
            } else {
                0
            };
            return Some(bit);
        }
        self.board.read_prg(addr)
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        let reg = addr & 0x0F;
        let decoded = if addr >= 0x8000 {
            self.high_regs
        } else {
            addr >= 0x6000 && self.low_regs
        };
        if decoded {
            self.write_reg(reg, val);
        } else {
            self.board.write_prg(addr, val);
        }
    }

    fn cpu_clock(&mut self) {
        if self.irq_enabled {
            if self.irq_counter == 0 {
                self.irq_pending = true;
            }
            self.irq_counter = self.irq_counter.wrapping_sub(1);
        }
    }

    fn irq(&self) -> bool {
        self.irq_pending
    }

    fn save_data(&self) -> Option<Vec<u8>> {
        match &self.eeprom {
            Some(e) => Some(e.data.clone()),
            None => self.board.battery.then(|| self.board.prg_ram.clone()),
        }
    }

    fn load_save_data(&mut self, data: &[u8]) {
        let target = match self.eeprom.as_mut() {
            Some(e) => &mut e.data,
            None => &mut self.board.prg_ram,
        };
        let n = target.len().min(data.len());
        target[..n].copy_from_slice(&data[..n]);
    }

    fn debug_state(&self) -> Vec<(&'static str, String)> {
        vec![
            ("PRG", format!("{:02X}", self.prg)),
            (
                "CHR",
                self.chr
                    .iter()
                    .map(|c| format!("{c:02X}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            (
                "IRQ",
                format!(
                    "counter {} {} {}",
                    self.irq_counter, self.irq_enabled, self.irq_pending
                ),
            ),
            (
                "EEPROM",
                self.eeprom
                    .as_ref()
                    .map(|e| format!("{:?} addr {:02X}", e.state, e.addr))
                    .unwrap_or("none".into()),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bit-bangs one byte (MSB first), returning whether it was acknowledged.
    fn send(e: &mut Eeprom, byte: u8) -> bool {
        for i in (0..8).rev() {
            let b = (byte >> i) & 1 != 0;
            e.write_lines(false, b);
            e.write_lines(true, b);
        }
        e.write_lines(false, true);
        e.write_lines(true, true);
        let ack = !e.out;
        e.write_lines(false, true);
        ack
    }

    fn recv(e: &mut Eeprom, ack: bool) -> u8 {
        let mut v = 0;
        for _ in 0..8 {
            e.write_lines(false, true);
            e.write_lines(true, true);
            v = (v << 1) | e.out as u8;
        }
        e.write_lines(false, !ack);
        e.write_lines(true, !ack);
        e.write_lines(false, true);
        v
    }

    fn start(e: &mut Eeprom) {
        e.write_lines(false, true);
        e.write_lines(true, true);
        e.write_lines(true, false);
    }

    fn stop(e: &mut Eeprom) {
        e.write_lines(false, false);
        e.write_lines(true, false);
        e.write_lines(true, true);
    }

    #[test]
    fn eeprom_24c02_write_then_random_read() {
        let mut e = Eeprom::new(false);
        start(&mut e);
        assert!(send(&mut e, 0xA0));
        assert!(send(&mut e, 0x10));
        assert!(send(&mut e, 0x42));
        assert!(send(&mut e, 0x43));
        stop(&mut e);
        assert_eq!(&e.data[0x10..0x12], &[0x42, 0x43]);

        start(&mut e);
        send(&mut e, 0xA0);
        send(&mut e, 0x10);
        start(&mut e);
        send(&mut e, 0xA1);
        let a = recv(&mut e, true);
        let b = recv(&mut e, false);
        stop(&mut e);
        assert_eq!((a, b), (0x42, 0x43));
    }
}
