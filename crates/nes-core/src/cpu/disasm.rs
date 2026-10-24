//! 6502 disassembler and nestest.log-compatible trace formatting.

use super::Registers;
use super::opcodes::{Mode, OPCODES, Opcode};
use std::fmt::Write;

/// One decoded instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instruction {
    pub addr: u16,
    pub opcode: u8,
    pub bytes: [u8; 3],
    pub len: u16,
}

impl Instruction {
    pub fn info(&self) -> &'static Opcode {
        &OPCODES[self.opcode as usize]
    }

    /// Branch / jump target, when it is statically known.
    pub fn target(&self) -> Option<u16> {
        let info = self.info();
        match (info.mode, info.mnemonic) {
            (Mode::Rel, _) => Some(branch_target(self.addr, self.bytes[1])),
            (Mode::Abs, "JMP" | "JSR") => Some(u16::from_le_bytes([self.bytes[1], self.bytes[2]])),
            _ => None,
        }
    }
}

/// Decodes the instruction at `addr` using a side-effect-free reader.
pub fn decode(addr: u16, peek: impl Fn(u16) -> u8) -> Instruction {
    let opcode = peek(addr);
    let len = OPCODES[opcode as usize].len();
    let mut bytes = [opcode, 0, 0];
    for i in 1..len {
        bytes[i as usize] = peek(addr.wrapping_add(i));
    }
    Instruction {
        addr,
        opcode,
        bytes,
        len,
    }
}

fn branch_target(addr: u16, offset: u8) -> u16 {
    addr.wrapping_add(2)
        .wrapping_add(offset as i8 as i16 as u16)
}

/// Plain operand syntax, e.g. `LDA $0200,X`.
pub fn format(ins: &Instruction) -> String {
    format_with(ins, |_| None)
}

/// Like [`format`], but lets the caller substitute a label for an address
/// operand (`label(addr)` returning `Some("PPUCTRL")` prints `PPUCTRL`).
pub fn format_with(ins: &Instruction, label: impl Fn(u16) -> Option<String>) -> String {
    let info = ins.info();
    let b1 = ins.bytes[1];
    let w = u16::from_le_bytes([ins.bytes[1], ins.bytes[2]]);
    let abs = |a: u16| label(a).unwrap_or_else(|| format!("${a:04X}"));
    let zp = |a: u8| label(a as u16).unwrap_or_else(|| format!("${a:02X}"));
    let m = info.mnemonic;
    match info.mode {
        Mode::Imp => m.to_string(),
        Mode::Acc => format!("{m} A"),
        Mode::Imm => format!("{m} #${b1:02X}"),
        Mode::Zp => format!("{m} {}", zp(b1)),
        Mode::Zpx => format!("{m} {},X", zp(b1)),
        Mode::Zpy => format!("{m} {},Y", zp(b1)),
        Mode::Abs => format!("{m} {}", abs(w)),
        Mode::Abx => format!("{m} {},X", abs(w)),
        Mode::Aby => format!("{m} {},Y", abs(w)),
        Mode::Ind => format!("{m} ({})", abs(w)),
        Mode::Izx => format!("{m} ({},X)", zp(b1)),
        Mode::Izy => format!("{m} ({}),Y", zp(b1)),
        Mode::Rel => format!("{m} {}", abs(branch_target(ins.addr, b1))),
    }
}

/// Operand text annotated with effective addresses and the values currently
/// stored there, in the format of nestest.log (`LDA ($80,X) @ 80 = 0200 = 5A`).
/// `peek` must not have side effects.
pub fn format_annotated(ins: &Instruction, regs: &Registers, peek: impl Fn(u16) -> u8) -> String {
    let info = ins.info();
    let b1 = ins.bytes[1];
    let w = u16::from_le_bytes([ins.bytes[1], ins.bytes[2]]);
    let m = info.mnemonic;
    let peek16_zp = |p: u8| u16::from_le_bytes([peek(p as u16), peek(p.wrapping_add(1) as u16)]);
    match info.mode {
        Mode::Imp => m.to_string(),
        Mode::Acc => format!("{m} A"),
        Mode::Imm => format!("{m} #${b1:02X}"),
        Mode::Zp => format!("{m} ${b1:02X} = {:02X}", peek(b1 as u16)),
        Mode::Zpx | Mode::Zpy => {
            let (idx, name) = if info.mode == Mode::Zpx {
                (regs.x, 'X')
            } else {
                (regs.y, 'Y')
            };
            let ea = b1.wrapping_add(idx);
            format!("{m} ${b1:02X},{name} @ {ea:02X} = {:02X}", peek(ea as u16))
        }
        Mode::Abs => {
            if matches!(m, "JMP" | "JSR") {
                format!("{m} ${w:04X}")
            } else {
                format!("{m} ${w:04X} = {:02X}", peek(w))
            }
        }
        Mode::Abx | Mode::Aby => {
            let (idx, name) = if info.mode == Mode::Abx {
                (regs.x, 'X')
            } else {
                (regs.y, 'Y')
            };
            let ea = w.wrapping_add(idx as u16);
            format!("{m} ${w:04X},{name} @ {ea:04X} = {:02X}", peek(ea))
        }
        Mode::Ind => {
            let lo = peek(w);
            let hi = peek((w & 0xFF00) | (w.wrapping_add(1) & 0x00FF));
            format!("{m} (${w:04X}) = {:04X}", u16::from_le_bytes([lo, hi]))
        }
        Mode::Izx => {
            let p = b1.wrapping_add(regs.x);
            let ea = peek16_zp(p);
            format!("{m} (${b1:02X},X) @ {p:02X} = {ea:04X} = {:02X}", peek(ea))
        }
        Mode::Izy => {
            let base = peek16_zp(b1);
            let ea = base.wrapping_add(regs.y as u16);
            format!(
                "{m} (${b1:02X}),Y = {base:04X} @ {ea:04X} = {:02X}",
                peek(ea)
            )
        }
        Mode::Rel => format!("{m} ${:04X}", branch_target(ins.addr, b1)),
    }
}

/// A full nestest.log line, minus the trailing `PPU:`/`CYC:` columns which the
/// caller appends (they depend on the rest of the system).
pub fn nestest_prefix(ins: &Instruction, regs: &Registers, peek: impl Fn(u16) -> u8) -> String {
    let mut line = String::with_capacity(96);
    let _ = write!(line, "{:04X}  ", ins.addr);
    let mut bytes = String::new();
    for i in 0..ins.len as usize {
        let _ = write!(bytes, "{:02X} ", ins.bytes[i]);
    }
    let _ = write!(line, "{bytes:<9}");
    line.push(if ins.info().illegal { '*' } else { ' ' });
    let text = format_annotated(ins, regs, peek);
    let _ = write!(
        line,
        "{text:<32}A:{:02X} X:{:02X} Y:{:02X} P:{:02X} SP:{:02X}",
        regs.a, regs.x, regs.y, regs.p, regs.sp
    );
    line
}

/// Names of the memory-mapped hardware registers, for disassembly labels.
pub fn io_register_name(addr: u16) -> Option<&'static str> {
    Some(match addr {
        0x2000 => "PPUCTRL",
        0x2001 => "PPUMASK",
        0x2002 => "PPUSTATUS",
        0x2003 => "OAMADDR",
        0x2004 => "OAMDATA",
        0x2005 => "PPUSCROLL",
        0x2006 => "PPUADDR",
        0x2007 => "PPUDATA",
        0x4000 => "SQ1_VOL",
        0x4001 => "SQ1_SWEEP",
        0x4002 => "SQ1_LO",
        0x4003 => "SQ1_HI",
        0x4004 => "SQ2_VOL",
        0x4005 => "SQ2_SWEEP",
        0x4006 => "SQ2_LO",
        0x4007 => "SQ2_HI",
        0x4008 => "TRI_LINEAR",
        0x400A => "TRI_LO",
        0x400B => "TRI_HI",
        0x400C => "NOISE_VOL",
        0x400E => "NOISE_LO",
        0x400F => "NOISE_HI",
        0x4010 => "DMC_FREQ",
        0x4011 => "DMC_RAW",
        0x4012 => "DMC_START",
        0x4013 => "DMC_LEN",
        0x4014 => "OAMDMA",
        0x4015 => "SND_CHN",
        0x4016 => "JOY1",
        0x4017 => "JOY2",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(bytes: &[u8]) -> impl Fn(u16) -> u8 + '_ {
        move |a| bytes.get(a as usize).copied().unwrap_or(0)
    }

    #[test]
    fn formats_modes() {
        let m = [0x4C, 0xF5, 0xC5];
        assert_eq!(format(&decode(0, mem(&m))), "JMP $C5F5");
        let m = [0xB1, 0x89];
        assert_eq!(format(&decode(0, mem(&m))), "LDA ($89),Y");
        let m = [0xD0, 0xFE];
        assert_eq!(
            format(&decode(0x8000, |a| [0xD0, 0xFE][(a - 0x8000) as usize])),
            "BNE $8000"
        );
        let _ = m;
    }

    #[test]
    fn nestest_line_layout() {
        let m = [0x4C, 0xF5, 0xC5];
        let ins = decode(0, mem(&m));
        let ins = Instruction {
            addr: 0xC000,
            ..ins
        };
        let regs = Registers {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFD,
            pc: 0xC000,
            p: 0x24,
        };
        assert_eq!(
            nestest_prefix(&ins, &regs, |_| 0),
            "C000  4C F5 C5  JMP $C5F5                       A:00 X:00 Y:00 P:24 SP:FD"
        );
    }
}
