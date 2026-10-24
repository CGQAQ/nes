//! The 6502 opcode matrix, shared by the disassembler, the tracer and the
//! debugger. Execution itself lives in `cpu/mod.rs` and does not consult this
//! table, so the table only has to describe instructions, not run them.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// No operand (`CLC`, `RTS`, ...).
    Imp,
    /// Operates on the accumulator (`ASL A`).
    Acc,
    /// `#$nn`
    Imm,
    /// `$nn`
    Zp,
    /// `$nn,X`
    Zpx,
    /// `$nn,Y`
    Zpy,
    /// `$nnnn`
    Abs,
    /// `$nnnn,X`
    Abx,
    /// `$nnnn,Y`
    Aby,
    /// `($nnnn)` — only `JMP`.
    Ind,
    /// `($nn,X)`
    Izx,
    /// `($nn),Y`
    Izy,
    /// Branch target.
    Rel,
}

impl Mode {
    /// Total instruction length in bytes, including the opcode.
    #[allow(clippy::len_without_is_empty)]
    pub const fn len(self) -> u16 {
        match self {
            Mode::Imp | Mode::Acc => 1,
            Mode::Imm | Mode::Zp | Mode::Zpx | Mode::Zpy | Mode::Izx | Mode::Izy | Mode::Rel => 2,
            Mode::Abs | Mode::Abx | Mode::Aby | Mode::Ind => 3,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Opcode {
    pub mnemonic: &'static str,
    pub mode: Mode,
    /// Not part of the documented instruction set.
    pub illegal: bool,
}

impl Opcode {
    #[allow(clippy::len_without_is_empty)]
    pub const fn len(&self) -> u16 {
        self.mode.len()
    }

    /// Instructions that read memory at their effective address (as opposed to
    /// only writing it, or only computing it like `JMP`).
    pub fn reads_memory(&self) -> bool {
        !matches!(
            self.mnemonic,
            "STA" | "STX" | "STY" | "SAX" | "SHA" | "SHX" | "SHY" | "TAS" | "JMP" | "JSR"
        )
    }

    pub fn is_jam(&self) -> bool {
        self.mnemonic == "JAM"
    }
}

const fn op(mnemonic: &'static str, mode: Mode) -> Opcode {
    Opcode {
        mnemonic,
        mode,
        illegal: false,
    }
}

const fn ill(mnemonic: &'static str, mode: Mode) -> Opcode {
    Opcode {
        mnemonic,
        mode,
        illegal: true,
    }
}

use Mode::*;

/// Indexed by opcode byte. Illegal mnemonics follow the names used by
/// nestest.log (`ISB`, `SLO`, ...) so traces can be diffed against it.
pub static OPCODES: [Opcode; 256] = [
    // 0x00
    op("BRK", Imp),
    op("ORA", Izx),
    ill("JAM", Imp),
    ill("SLO", Izx),
    ill("NOP", Zp),
    op("ORA", Zp),
    op("ASL", Zp),
    ill("SLO", Zp),
    op("PHP", Imp),
    op("ORA", Imm),
    op("ASL", Acc),
    ill("ANC", Imm),
    ill("NOP", Abs),
    op("ORA", Abs),
    op("ASL", Abs),
    ill("SLO", Abs),
    // 0x10
    op("BPL", Rel),
    op("ORA", Izy),
    ill("JAM", Imp),
    ill("SLO", Izy),
    ill("NOP", Zpx),
    op("ORA", Zpx),
    op("ASL", Zpx),
    ill("SLO", Zpx),
    op("CLC", Imp),
    op("ORA", Aby),
    ill("NOP", Imp),
    ill("SLO", Aby),
    ill("NOP", Abx),
    op("ORA", Abx),
    op("ASL", Abx),
    ill("SLO", Abx),
    // 0x20
    op("JSR", Abs),
    op("AND", Izx),
    ill("JAM", Imp),
    ill("RLA", Izx),
    op("BIT", Zp),
    op("AND", Zp),
    op("ROL", Zp),
    ill("RLA", Zp),
    op("PLP", Imp),
    op("AND", Imm),
    op("ROL", Acc),
    ill("ANC", Imm),
    op("BIT", Abs),
    op("AND", Abs),
    op("ROL", Abs),
    ill("RLA", Abs),
    // 0x30
    op("BMI", Rel),
    op("AND", Izy),
    ill("JAM", Imp),
    ill("RLA", Izy),
    ill("NOP", Zpx),
    op("AND", Zpx),
    op("ROL", Zpx),
    ill("RLA", Zpx),
    op("SEC", Imp),
    op("AND", Aby),
    ill("NOP", Imp),
    ill("RLA", Aby),
    ill("NOP", Abx),
    op("AND", Abx),
    op("ROL", Abx),
    ill("RLA", Abx),
    // 0x40
    op("RTI", Imp),
    op("EOR", Izx),
    ill("JAM", Imp),
    ill("SRE", Izx),
    ill("NOP", Zp),
    op("EOR", Zp),
    op("LSR", Zp),
    ill("SRE", Zp),
    op("PHA", Imp),
    op("EOR", Imm),
    op("LSR", Acc),
    ill("ALR", Imm),
    op("JMP", Abs),
    op("EOR", Abs),
    op("LSR", Abs),
    ill("SRE", Abs),
    // 0x50
    op("BVC", Rel),
    op("EOR", Izy),
    ill("JAM", Imp),
    ill("SRE", Izy),
    ill("NOP", Zpx),
    op("EOR", Zpx),
    op("LSR", Zpx),
    ill("SRE", Zpx),
    op("CLI", Imp),
    op("EOR", Aby),
    ill("NOP", Imp),
    ill("SRE", Aby),
    ill("NOP", Abx),
    op("EOR", Abx),
    op("LSR", Abx),
    ill("SRE", Abx),
    // 0x60
    op("RTS", Imp),
    op("ADC", Izx),
    ill("JAM", Imp),
    ill("RRA", Izx),
    ill("NOP", Zp),
    op("ADC", Zp),
    op("ROR", Zp),
    ill("RRA", Zp),
    op("PLA", Imp),
    op("ADC", Imm),
    op("ROR", Acc),
    ill("ARR", Imm),
    op("JMP", Ind),
    op("ADC", Abs),
    op("ROR", Abs),
    ill("RRA", Abs),
    // 0x70
    op("BVS", Rel),
    op("ADC", Izy),
    ill("JAM", Imp),
    ill("RRA", Izy),
    ill("NOP", Zpx),
    op("ADC", Zpx),
    op("ROR", Zpx),
    ill("RRA", Zpx),
    op("SEI", Imp),
    op("ADC", Aby),
    ill("NOP", Imp),
    ill("RRA", Aby),
    ill("NOP", Abx),
    op("ADC", Abx),
    op("ROR", Abx),
    ill("RRA", Abx),
    // 0x80
    ill("NOP", Imm),
    op("STA", Izx),
    ill("NOP", Imm),
    ill("SAX", Izx),
    op("STY", Zp),
    op("STA", Zp),
    op("STX", Zp),
    ill("SAX", Zp),
    op("DEY", Imp),
    ill("NOP", Imm),
    op("TXA", Imp),
    ill("ANE", Imm),
    op("STY", Abs),
    op("STA", Abs),
    op("STX", Abs),
    ill("SAX", Abs),
    // 0x90
    op("BCC", Rel),
    op("STA", Izy),
    ill("JAM", Imp),
    ill("SHA", Izy),
    op("STY", Zpx),
    op("STA", Zpx),
    op("STX", Zpy),
    ill("SAX", Zpy),
    op("TYA", Imp),
    op("STA", Aby),
    op("TXS", Imp),
    ill("TAS", Aby),
    ill("SHY", Abx),
    op("STA", Abx),
    ill("SHX", Aby),
    ill("SHA", Aby),
    // 0xA0
    op("LDY", Imm),
    op("LDA", Izx),
    op("LDX", Imm),
    ill("LAX", Izx),
    op("LDY", Zp),
    op("LDA", Zp),
    op("LDX", Zp),
    ill("LAX", Zp),
    op("TAY", Imp),
    op("LDA", Imm),
    op("TAX", Imp),
    ill("LXA", Imm),
    op("LDY", Abs),
    op("LDA", Abs),
    op("LDX", Abs),
    ill("LAX", Abs),
    // 0xB0
    op("BCS", Rel),
    op("LDA", Izy),
    ill("JAM", Imp),
    ill("LAX", Izy),
    op("LDY", Zpx),
    op("LDA", Zpx),
    op("LDX", Zpy),
    ill("LAX", Zpy),
    op("CLV", Imp),
    op("LDA", Aby),
    op("TSX", Imp),
    ill("LAS", Aby),
    op("LDY", Abx),
    op("LDA", Abx),
    op("LDX", Aby),
    ill("LAX", Aby),
    // 0xC0
    op("CPY", Imm),
    op("CMP", Izx),
    ill("NOP", Imm),
    ill("DCP", Izx),
    op("CPY", Zp),
    op("CMP", Zp),
    op("DEC", Zp),
    ill("DCP", Zp),
    op("INY", Imp),
    op("CMP", Imm),
    op("DEX", Imp),
    ill("AXS", Imm),
    op("CPY", Abs),
    op("CMP", Abs),
    op("DEC", Abs),
    ill("DCP", Abs),
    // 0xD0
    op("BNE", Rel),
    op("CMP", Izy),
    ill("JAM", Imp),
    ill("DCP", Izy),
    ill("NOP", Zpx),
    op("CMP", Zpx),
    op("DEC", Zpx),
    ill("DCP", Zpx),
    op("CLD", Imp),
    op("CMP", Aby),
    ill("NOP", Imp),
    ill("DCP", Aby),
    ill("NOP", Abx),
    op("CMP", Abx),
    op("DEC", Abx),
    ill("DCP", Abx),
    // 0xE0
    op("CPX", Imm),
    op("SBC", Izx),
    ill("NOP", Imm),
    ill("ISB", Izx),
    op("CPX", Zp),
    op("SBC", Zp),
    op("INC", Zp),
    ill("ISB", Zp),
    op("INX", Imp),
    op("SBC", Imm),
    op("NOP", Imp),
    ill("SBC", Imm),
    op("CPX", Abs),
    op("SBC", Abs),
    op("INC", Abs),
    ill("ISB", Abs),
    // 0xF0
    op("BEQ", Rel),
    op("SBC", Izy),
    ill("JAM", Imp),
    ill("ISB", Izy),
    ill("NOP", Zpx),
    op("SBC", Zpx),
    op("INC", Zpx),
    ill("ISB", Zpx),
    op("SED", Imp),
    op("SBC", Aby),
    ill("NOP", Imp),
    ill("ISB", Aby),
    ill("NOP", Abx),
    op("SBC", Abx),
    op("INC", Abx),
    ill("ISB", Abx),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_instruction_count() {
        // 151 documented opcodes on the NMOS 6502.
        assert_eq!(OPCODES.iter().filter(|o| !o.illegal).count(), 151);
        assert_eq!(OPCODES.iter().filter(|o| o.is_jam()).count(), 12);
    }
}
