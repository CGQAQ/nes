//! Cycle-accurate Ricoh 2A03 CPU core (an NMOS 6502 without decimal mode).
//!
//! Every bus access the real chip performs — including dummy reads and the
//! dummy writes of read-modify-write instructions — is issued through
//! [`CpuBus`], one call per CPU cycle. The bus advances the rest of the system
//! on each call, so instruction timing falls out of the access pattern rather
//! than from a cycle table.
//!
//! Interrupt lines are sampled at the end of every cycle; an interrupt is taken
//! after an instruction if it was pending at the end of the instruction's
//! second-to-last cycle, which is how the hardware behaves.

pub mod disasm;
pub mod opcodes;

use serde::{Deserialize, Serialize};

/// The CPU's view of the system bus. Each call is exactly one CPU cycle.
pub trait CpuBus {
    fn read(&mut self, addr: u16) -> u8;
    /// A read cycle whose value the CPU throws away. It must behave exactly
    /// like [`CpuBus::read`] (register side effects included); the separate
    /// entry point only lets debuggers ignore it.
    fn dummy_read(&mut self, addr: u16) {
        self.read(addr);
    }
    fn write(&mut self, addr: u16, val: u8);
    /// Level of the (active-low on hardware) NMI line; `true` means asserted.
    fn nmi_line(&self) -> bool;
    /// Level of the IRQ line; `true` means asserted.
    fn irq_line(&self) -> bool;
}

pub mod flags {
    pub const C: u8 = 0x01;
    pub const Z: u8 = 0x02;
    pub const I: u8 = 0x04;
    pub const D: u8 = 0x08;
    pub const B: u8 = 0x10;
    pub const U: u8 = 0x20;
    pub const V: u8 = 0x40;
    pub const N: u8 = 0x80;
}
use flags::*;

pub const NMI_VECTOR: u16 = 0xFFFA;
pub const RESET_VECTOR: u16 = 0xFFFC;
pub const IRQ_VECTOR: u16 = 0xFFFE;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registers {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    /// Status register. Bit 5 always reads as set and bit 4 (B) as clear: B
    /// only exists in copies of P pushed to the stack.
    pub p: u8,
}

/// What a call to [`Cpu::step`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Instruction,
    Nmi,
    Irq,
    /// The CPU executed (or is stuck on) a JAM opcode and only a reset frees it.
    Jammed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cpu {
    pub regs: Registers,
    /// NMI line level seen at the end of the previous cycle (edge detector).
    nmi_prev_line: bool,
    /// An NMI edge has been detected and not yet serviced.
    need_nmi: bool,
    prev_need_nmi: bool,
    /// IRQ line asserted and I clear, as of the end of the last cycle.
    run_irq: bool,
    prev_run_irq: bool,
    /// Interrupts are checked between instructions, but never directly after
    /// an interrupt sequence: the handler's first instruction always runs.
    interrupt_check: bool,
    jammed: bool,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            regs: Registers {
                a: 0,
                x: 0,
                y: 0,
                sp: 0,
                pc: 0,
                p: I | U,
            },
            nmi_prev_line: false,
            need_nmi: false,
            prev_need_nmi: false,
            run_irq: false,
            prev_run_irq: false,
            interrupt_check: false,
            jammed: false,
        }
    }

    pub fn is_jammed(&self) -> bool {
        self.jammed
    }

    /// An NMI edge is latched and will be serviced (it takes priority over IRQ).
    pub fn nmi_pending(&self) -> bool {
        self.need_nmi
    }

    /// True when the next [`Cpu::step`] will enter an interrupt handler
    /// instead of executing the instruction at PC.
    pub fn interrupt_pending(&self) -> bool {
        !self.jammed && self.interrupt_check && (self.prev_run_irq || self.prev_need_nmi)
    }

    /// Power-up (`power = true`) or the RESET line. Takes 7 bus cycles, like
    /// the hardware's aborted interrupt sequence: S is decremented three times
    /// without writing, then the reset vector is read.
    pub fn reset<B: CpuBus>(&mut self, bus: &mut B, power: bool) {
        if power {
            *self = Cpu::new();
        }
        self.jammed = false;
        self.need_nmi = false;
        self.prev_need_nmi = false;
        self.run_irq = false;
        self.prev_run_irq = false;
        self.interrupt_check = false;
        // The real chip reads from PC here; reading the stack page instead
        // keeps the same timing without triggering register side effects.
        let sp = self.regs.sp;
        self.dummy_read(bus, 0x0100 | sp as u16);
        self.dummy_read(bus, 0x0100 | sp as u16);
        for _ in 0..3 {
            let sp = self.regs.sp;
            self.dummy_read(bus, 0x0100 | sp as u16);
            self.regs.sp = sp.wrapping_sub(1);
        }
        self.regs.p |= I;
        let lo = self.read(bus, RESET_VECTOR);
        let hi = self.read(bus, RESET_VECTOR + 1);
        self.regs.pc = u16::from_le_bytes([lo, hi]);
    }

    /// Executes one instruction, or one interrupt entry sequence.
    pub fn step<B: CpuBus>(&mut self, bus: &mut B) -> Step {
        if self.jammed {
            // The bus keeps running (and the PPU keeps drawing) while stuck.
            self.dummy_read(bus, 0xFFFF);
            return Step::Jammed;
        }
        if self.interrupt_check && (self.prev_run_irq || self.prev_need_nmi) {
            self.interrupt_check = false;
            return self.interrupt(bus);
        }
        self.interrupt_check = true;
        let opcode = self.fetch(bus);
        self.execute(bus, opcode);
        if self.jammed {
            Step::Jammed
        } else {
            Step::Instruction
        }
    }

    // ---------------------------------------------------------------------
    // Bus helpers. Every call here is one CPU cycle.

    #[inline(always)]
    fn read<B: CpuBus>(&mut self, bus: &mut B, addr: u16) -> u8 {
        let v = bus.read(addr);
        self.poll(bus);
        v
    }

    #[inline(always)]
    fn write<B: CpuBus>(&mut self, bus: &mut B, addr: u16, val: u8) {
        bus.write(addr, val);
        self.poll(bus);
    }

    /// End-of-cycle interrupt sampling.
    #[inline(always)]
    fn poll<B: CpuBus>(&mut self, bus: &mut B) {
        self.prev_need_nmi = self.need_nmi;
        let nmi = bus.nmi_line();
        if nmi && !self.nmi_prev_line {
            self.need_nmi = true;
        }
        self.nmi_prev_line = nmi;
        self.prev_run_irq = self.run_irq;
        self.run_irq = bus.irq_line() && self.regs.p & I == 0;
    }

    #[inline(always)]
    fn fetch<B: CpuBus>(&mut self, bus: &mut B) -> u8 {
        let v = self.read(bus, self.regs.pc);
        self.regs.pc = self.regs.pc.wrapping_add(1);
        v
    }

    #[inline(always)]
    fn fetch16<B: CpuBus>(&mut self, bus: &mut B) -> u16 {
        let lo = self.fetch(bus);
        let hi = self.fetch(bus);
        u16::from_le_bytes([lo, hi])
    }

    /// A read whose value the CPU discards (it still has side effects).
    #[inline(always)]
    fn dummy_read<B: CpuBus>(&mut self, bus: &mut B, addr: u16) {
        bus.dummy_read(addr);
        self.poll(bus);
    }

    #[inline(always)]
    fn dummy_read_pc<B: CpuBus>(&mut self, bus: &mut B) {
        self.dummy_read(bus, self.regs.pc);
    }

    #[inline(always)]
    fn push<B: CpuBus>(&mut self, bus: &mut B, val: u8) {
        let sp = self.regs.sp;
        self.write(bus, 0x0100 | sp as u16, val);
        self.regs.sp = sp.wrapping_sub(1);
    }

    #[inline(always)]
    fn pull<B: CpuBus>(&mut self, bus: &mut B) -> u8 {
        self.regs.sp = self.regs.sp.wrapping_add(1);
        self.read(bus, 0x0100 | self.regs.sp as u16)
    }

    /// The dummy stack read every pull instruction does before incrementing S.
    #[inline(always)]
    fn dummy_read_stack<B: CpuBus>(&mut self, bus: &mut B) {
        self.dummy_read(bus, 0x0100 | self.regs.sp as u16);
    }

    // ---------------------------------------------------------------------
    // Effective-address computation, with the exact dummy accesses.

    /// `write` selects the store/RMW variant of the indexed modes, which
    /// always perform the dummy read at the un-carried address.
    #[inline(always)]
    fn ea<B: CpuBus>(&mut self, bus: &mut B, mode: opcodes::Mode, write: bool) -> u16 {
        use opcodes::Mode::*;
        match mode {
            Zp => self.fetch(bus) as u16,
            Zpx | Zpy => {
                let base = self.fetch(bus);
                self.dummy_read(bus, base as u16);
                let idx = if mode == Zpx {
                    self.regs.x
                } else {
                    self.regs.y
                };
                base.wrapping_add(idx) as u16
            }
            Abs => self.fetch16(bus),
            Abx | Aby => {
                let base = self.fetch16(bus);
                let idx = if mode == Abx {
                    self.regs.x
                } else {
                    self.regs.y
                };
                self.indexed(bus, base, idx, write)
            }
            Izx => {
                let ptr = self.fetch(bus);
                self.dummy_read(bus, ptr as u16);
                let ptr = ptr.wrapping_add(self.regs.x);
                let lo = self.read(bus, ptr as u16);
                let hi = self.read(bus, ptr.wrapping_add(1) as u16);
                u16::from_le_bytes([lo, hi])
            }
            Izy => {
                let ptr = self.fetch(bus);
                let lo = self.read(bus, ptr as u16);
                let hi = self.read(bus, ptr.wrapping_add(1) as u16);
                let base = u16::from_le_bytes([lo, hi]);
                self.indexed(bus, base, self.regs.y, write)
            }
            Imp | Acc | Imm | Ind | Rel => unreachable!("mode {mode:?} has no data address"),
        }
    }

    #[inline(always)]
    fn indexed<B: CpuBus>(&mut self, bus: &mut B, base: u16, idx: u8, write: bool) -> u16 {
        let addr = base.wrapping_add(idx as u16);
        if write || (addr ^ base) & 0xFF00 != 0 {
            self.dummy_read(bus, (base & 0xFF00) | (addr & 0x00FF));
        }
        addr
    }

    /// Read instruction: fetch the operand and hand it to `f`.
    #[inline(always)]
    fn rd<B: CpuBus>(&mut self, bus: &mut B, mode: opcodes::Mode, f: fn(&mut Self, u8)) {
        let v = if mode == opcodes::Mode::Imm {
            self.fetch(bus)
        } else {
            let addr = self.ea(bus, mode, false);
            self.read(bus, addr)
        };
        f(self, v);
    }

    #[inline(always)]
    fn wr<B: CpuBus>(&mut self, bus: &mut B, mode: opcodes::Mode, val: u8) {
        let addr = self.ea(bus, mode, true);
        self.write(bus, addr, val);
    }

    /// Read-modify-write: read, write the old value back (the 6502's dummy
    /// write, visible to mappers), then write the result.
    #[inline(always)]
    fn rmw<B: CpuBus>(&mut self, bus: &mut B, mode: opcodes::Mode, f: fn(&mut Self, u8) -> u8) {
        if mode == opcodes::Mode::Acc {
            self.dummy_read_pc(bus);
            self.regs.a = f(self, self.regs.a);
            return;
        }
        let addr = self.ea(bus, mode, true);
        let v = self.read(bus, addr);
        self.write(bus, addr, v);
        let r = f(self, v);
        self.write(bus, addr, r);
    }

    /// Implied-mode instruction: one dummy read of the next byte.
    #[inline(always)]
    fn imp<B: CpuBus>(&mut self, bus: &mut B, f: fn(&mut Self)) {
        self.dummy_read_pc(bus);
        f(self);
    }

    // ---------------------------------------------------------------------
    // Flag helpers and ALU operations.

    #[inline(always)]
    fn set_flag(&mut self, flag: u8, on: bool) {
        if on {
            self.regs.p |= flag;
        } else {
            self.regs.p &= !flag;
        }
    }

    #[inline(always)]
    fn set_zn(&mut self, v: u8) {
        self.set_flag(Z, v == 0);
        self.set_flag(N, v & 0x80 != 0);
    }

    fn carry(&self) -> u8 {
        self.regs.p & C
    }

    fn lda(&mut self, v: u8) {
        self.regs.a = v;
        self.set_zn(v);
    }
    fn ldx(&mut self, v: u8) {
        self.regs.x = v;
        self.set_zn(v);
    }
    fn ldy(&mut self, v: u8) {
        self.regs.y = v;
        self.set_zn(v);
    }
    fn lax(&mut self, v: u8) {
        self.regs.a = v;
        self.regs.x = v;
        self.set_zn(v);
    }
    fn and(&mut self, v: u8) {
        self.regs.a &= v;
        self.set_zn(self.regs.a);
    }
    fn ora(&mut self, v: u8) {
        self.regs.a |= v;
        self.set_zn(self.regs.a);
    }
    fn eor(&mut self, v: u8) {
        self.regs.a ^= v;
        self.set_zn(self.regs.a);
    }
    fn adc(&mut self, v: u8) {
        let a = self.regs.a as u16;
        let sum = a + v as u16 + self.carry() as u16;
        let r = sum as u8;
        self.set_flag(C, sum > 0xFF);
        self.set_flag(V, (!(self.regs.a ^ v) & (self.regs.a ^ r)) & 0x80 != 0);
        self.regs.a = r;
        self.set_zn(r);
    }
    fn sbc(&mut self, v: u8) {
        self.adc(!v);
    }
    fn compare(&mut self, reg: u8, v: u8) {
        self.set_flag(C, reg >= v);
        self.set_zn(reg.wrapping_sub(v));
    }
    fn cmp(&mut self, v: u8) {
        self.compare(self.regs.a, v);
    }
    fn cpx(&mut self, v: u8) {
        self.compare(self.regs.x, v);
    }
    fn cpy(&mut self, v: u8) {
        self.compare(self.regs.y, v);
    }
    fn bit(&mut self, v: u8) {
        self.set_flag(Z, self.regs.a & v == 0);
        self.set_flag(N, v & 0x80 != 0);
        self.set_flag(V, v & 0x40 != 0);
    }
    fn nop_read(&mut self, _v: u8) {}

    fn asl(&mut self, v: u8) -> u8 {
        self.set_flag(C, v & 0x80 != 0);
        let r = v << 1;
        self.set_zn(r);
        r
    }
    fn lsr(&mut self, v: u8) -> u8 {
        self.set_flag(C, v & 0x01 != 0);
        let r = v >> 1;
        self.set_zn(r);
        r
    }
    fn rol(&mut self, v: u8) -> u8 {
        let r = (v << 1) | self.carry();
        self.set_flag(C, v & 0x80 != 0);
        self.set_zn(r);
        r
    }
    fn ror(&mut self, v: u8) -> u8 {
        let r = (v >> 1) | (self.carry() << 7);
        self.set_flag(C, v & 0x01 != 0);
        self.set_zn(r);
        r
    }
    fn inc(&mut self, v: u8) -> u8 {
        let r = v.wrapping_add(1);
        self.set_zn(r);
        r
    }
    fn dec(&mut self, v: u8) -> u8 {
        let r = v.wrapping_sub(1);
        self.set_zn(r);
        r
    }

    // Unofficial combined RMW operations.
    fn slo(&mut self, v: u8) -> u8 {
        let r = self.asl(v);
        self.ora(r);
        r
    }
    fn rla(&mut self, v: u8) -> u8 {
        let r = self.rol(v);
        self.and(r);
        r
    }
    fn sre(&mut self, v: u8) -> u8 {
        let r = self.lsr(v);
        self.eor(r);
        r
    }
    fn rra(&mut self, v: u8) -> u8 {
        let r = self.ror(v);
        self.adc(r);
        r
    }
    fn dcp(&mut self, v: u8) -> u8 {
        let r = v.wrapping_sub(1);
        self.cmp(r);
        r
    }
    fn isb(&mut self, v: u8) -> u8 {
        let r = v.wrapping_add(1);
        self.sbc(r);
        r
    }

    // Unofficial immediate operations.
    fn anc(&mut self, v: u8) {
        self.and(v);
        self.set_flag(C, self.regs.a & 0x80 != 0);
    }
    fn alr(&mut self, v: u8) {
        self.regs.a &= v;
        self.regs.a = self.lsr(self.regs.a);
    }
    fn arr(&mut self, v: u8) {
        let t = self.regs.a & v;
        let r = (t >> 1) | (self.carry() << 7);
        self.regs.a = r;
        self.set_zn(r);
        self.set_flag(C, r & 0x40 != 0);
        self.set_flag(V, ((r >> 6) ^ (r >> 5)) & 1 != 0);
    }
    fn axs(&mut self, v: u8) {
        let t = self.regs.a & self.regs.x;
        self.set_flag(C, t >= v);
        self.regs.x = t.wrapping_sub(v);
        self.set_zn(self.regs.x);
    }
    /// ANE/XAA is analog-unstable; 0xEE is the commonly measured "magic".
    fn ane(&mut self, v: u8) {
        self.regs.a = (self.regs.a | 0xEE) & self.regs.x & v;
        self.set_zn(self.regs.a);
    }
    fn lxa(&mut self, v: u8) {
        // Really `(A | magic) & imm` with a chip-dependent magic; magic = 0xFF
        // (so just `imm`) is what blargg's tests and most software expect.
        let r = v;
        self.regs.a = r;
        self.regs.x = r;
        self.set_zn(r);
    }
    fn las(&mut self, v: u8) {
        let r = v & self.regs.sp;
        self.regs.a = r;
        self.regs.x = r;
        self.regs.sp = r;
        self.set_zn(r);
    }

    /// SHA/SHX/SHY/TAS: store `val & (H + 1)`, where H is the high byte of
    /// the base address. When the index crosses a page the stored value also
    /// replaces the high byte of the target address.
    fn sh_store<B: CpuBus>(&mut self, bus: &mut B, base: u16, idx: u8, val: u8) {
        let addr = base.wrapping_add(idx as u16);
        self.dummy_read(bus, (base & 0xFF00) | (addr & 0x00FF));
        let v = val & ((base >> 8) as u8).wrapping_add(1);
        let target = if (addr ^ base) & 0xFF00 != 0 {
            ((v as u16) << 8) | (addr & 0x00FF)
        } else {
            addr
        };
        self.write(bus, target, v);
    }

    fn sh_abs<B: CpuBus>(&mut self, bus: &mut B, idx: u8, val: u8) {
        let base = self.fetch16(bus);
        self.sh_store(bus, base, idx, val);
    }

    // ---------------------------------------------------------------------
    // Control flow.

    fn branch<B: CpuBus>(&mut self, bus: &mut B, cond: bool) {
        let offset = self.fetch(bus) as i8;
        if !cond {
            return;
        }
        // A taken branch that doesn't cross a page doesn't poll interrupts on
        // its last cycle: an IRQ that arrived during the operand fetch waits
        // one more instruction.
        if self.run_irq && !self.prev_run_irq {
            self.run_irq = false;
        }
        self.dummy_read_pc(bus);
        let pc = self.regs.pc;
        let target = pc.wrapping_add(offset as i16 as u16);
        if (target ^ pc) & 0xFF00 != 0 {
            self.dummy_read(bus, (pc & 0xFF00) | (target & 0x00FF));
        }
        self.regs.pc = target;
    }

    fn brk<B: CpuBus>(&mut self, bus: &mut B) {
        // The padding byte after BRK is read and skipped.
        self.dummy_read_pc(bus);
        self.regs.pc = self.regs.pc.wrapping_add(1);
        let pc = self.regs.pc;
        self.push(bus, (pc >> 8) as u8);
        self.push(bus, pc as u8);
        // An NMI detected by now hijacks the vector fetch.
        let vector = if self.need_nmi {
            self.need_nmi = false;
            NMI_VECTOR
        } else {
            IRQ_VECTOR
        };
        let p = self.regs.p | B | U;
        self.push(bus, p);
        self.regs.p |= I;
        let lo = self.read(bus, vector);
        let hi = self.read(bus, vector + 1);
        self.regs.pc = u16::from_le_bytes([lo, hi]);
        // The handler's first instruction runs before any further NMI.
        self.prev_need_nmi = false;
    }

    fn interrupt<B: CpuBus>(&mut self, bus: &mut B) -> Step {
        self.dummy_read_pc(bus);
        self.dummy_read_pc(bus);
        let pc = self.regs.pc;
        self.push(bus, (pc >> 8) as u8);
        self.push(bus, pc as u8);
        let (vector, kind) = if self.need_nmi {
            self.need_nmi = false;
            (NMI_VECTOR, Step::Nmi)
        } else {
            (IRQ_VECTOR, Step::Irq)
        };
        let p = (self.regs.p & !B) | U;
        self.push(bus, p);
        self.regs.p |= I;
        let lo = self.read(bus, vector);
        let hi = self.read(bus, vector + 1);
        self.regs.pc = u16::from_le_bytes([lo, hi]);
        kind
    }

    fn set_p(&mut self, v: u8) {
        self.regs.p = (v & !B) | U;
    }

    // ---------------------------------------------------------------------

    fn execute<B: CpuBus>(&mut self, bus: &mut B, opcode: u8) {
        use opcodes::Mode::*;
        match opcode {
            // --- Loads
            0xA9 => self.rd(bus, Imm, Self::lda),
            0xA5 => self.rd(bus, Zp, Self::lda),
            0xB5 => self.rd(bus, Zpx, Self::lda),
            0xAD => self.rd(bus, Abs, Self::lda),
            0xBD => self.rd(bus, Abx, Self::lda),
            0xB9 => self.rd(bus, Aby, Self::lda),
            0xA1 => self.rd(bus, Izx, Self::lda),
            0xB1 => self.rd(bus, Izy, Self::lda),
            0xA2 => self.rd(bus, Imm, Self::ldx),
            0xA6 => self.rd(bus, Zp, Self::ldx),
            0xB6 => self.rd(bus, Zpy, Self::ldx),
            0xAE => self.rd(bus, Abs, Self::ldx),
            0xBE => self.rd(bus, Aby, Self::ldx),
            0xA0 => self.rd(bus, Imm, Self::ldy),
            0xA4 => self.rd(bus, Zp, Self::ldy),
            0xB4 => self.rd(bus, Zpx, Self::ldy),
            0xAC => self.rd(bus, Abs, Self::ldy),
            0xBC => self.rd(bus, Abx, Self::ldy),

            // --- Stores
            0x85 => self.wr(bus, Zp, self.regs.a),
            0x95 => self.wr(bus, Zpx, self.regs.a),
            0x8D => self.wr(bus, Abs, self.regs.a),
            0x9D => self.wr(bus, Abx, self.regs.a),
            0x99 => self.wr(bus, Aby, self.regs.a),
            0x81 => self.wr(bus, Izx, self.regs.a),
            0x91 => self.wr(bus, Izy, self.regs.a),
            0x86 => self.wr(bus, Zp, self.regs.x),
            0x96 => self.wr(bus, Zpy, self.regs.x),
            0x8E => self.wr(bus, Abs, self.regs.x),
            0x84 => self.wr(bus, Zp, self.regs.y),
            0x94 => self.wr(bus, Zpx, self.regs.y),
            0x8C => self.wr(bus, Abs, self.regs.y),

            // --- Arithmetic / logic
            0x69 => self.rd(bus, Imm, Self::adc),
            0x65 => self.rd(bus, Zp, Self::adc),
            0x75 => self.rd(bus, Zpx, Self::adc),
            0x6D => self.rd(bus, Abs, Self::adc),
            0x7D => self.rd(bus, Abx, Self::adc),
            0x79 => self.rd(bus, Aby, Self::adc),
            0x61 => self.rd(bus, Izx, Self::adc),
            0x71 => self.rd(bus, Izy, Self::adc),
            0xE9 | 0xEB => self.rd(bus, Imm, Self::sbc),
            0xE5 => self.rd(bus, Zp, Self::sbc),
            0xF5 => self.rd(bus, Zpx, Self::sbc),
            0xED => self.rd(bus, Abs, Self::sbc),
            0xFD => self.rd(bus, Abx, Self::sbc),
            0xF9 => self.rd(bus, Aby, Self::sbc),
            0xE1 => self.rd(bus, Izx, Self::sbc),
            0xF1 => self.rd(bus, Izy, Self::sbc),
            0x29 => self.rd(bus, Imm, Self::and),
            0x25 => self.rd(bus, Zp, Self::and),
            0x35 => self.rd(bus, Zpx, Self::and),
            0x2D => self.rd(bus, Abs, Self::and),
            0x3D => self.rd(bus, Abx, Self::and),
            0x39 => self.rd(bus, Aby, Self::and),
            0x21 => self.rd(bus, Izx, Self::and),
            0x31 => self.rd(bus, Izy, Self::and),
            0x09 => self.rd(bus, Imm, Self::ora),
            0x05 => self.rd(bus, Zp, Self::ora),
            0x15 => self.rd(bus, Zpx, Self::ora),
            0x0D => self.rd(bus, Abs, Self::ora),
            0x1D => self.rd(bus, Abx, Self::ora),
            0x19 => self.rd(bus, Aby, Self::ora),
            0x01 => self.rd(bus, Izx, Self::ora),
            0x11 => self.rd(bus, Izy, Self::ora),
            0x49 => self.rd(bus, Imm, Self::eor),
            0x45 => self.rd(bus, Zp, Self::eor),
            0x55 => self.rd(bus, Zpx, Self::eor),
            0x4D => self.rd(bus, Abs, Self::eor),
            0x5D => self.rd(bus, Abx, Self::eor),
            0x59 => self.rd(bus, Aby, Self::eor),
            0x41 => self.rd(bus, Izx, Self::eor),
            0x51 => self.rd(bus, Izy, Self::eor),
            0xC9 => self.rd(bus, Imm, Self::cmp),
            0xC5 => self.rd(bus, Zp, Self::cmp),
            0xD5 => self.rd(bus, Zpx, Self::cmp),
            0xCD => self.rd(bus, Abs, Self::cmp),
            0xDD => self.rd(bus, Abx, Self::cmp),
            0xD9 => self.rd(bus, Aby, Self::cmp),
            0xC1 => self.rd(bus, Izx, Self::cmp),
            0xD1 => self.rd(bus, Izy, Self::cmp),
            0xE0 => self.rd(bus, Imm, Self::cpx),
            0xE4 => self.rd(bus, Zp, Self::cpx),
            0xEC => self.rd(bus, Abs, Self::cpx),
            0xC0 => self.rd(bus, Imm, Self::cpy),
            0xC4 => self.rd(bus, Zp, Self::cpy),
            0xCC => self.rd(bus, Abs, Self::cpy),
            0x24 => self.rd(bus, Zp, Self::bit),
            0x2C => self.rd(bus, Abs, Self::bit),

            // --- Shifts / increments
            0x0A => self.rmw(bus, Acc, Self::asl),
            0x06 => self.rmw(bus, Zp, Self::asl),
            0x16 => self.rmw(bus, Zpx, Self::asl),
            0x0E => self.rmw(bus, Abs, Self::asl),
            0x1E => self.rmw(bus, Abx, Self::asl),
            0x4A => self.rmw(bus, Acc, Self::lsr),
            0x46 => self.rmw(bus, Zp, Self::lsr),
            0x56 => self.rmw(bus, Zpx, Self::lsr),
            0x4E => self.rmw(bus, Abs, Self::lsr),
            0x5E => self.rmw(bus, Abx, Self::lsr),
            0x2A => self.rmw(bus, Acc, Self::rol),
            0x26 => self.rmw(bus, Zp, Self::rol),
            0x36 => self.rmw(bus, Zpx, Self::rol),
            0x2E => self.rmw(bus, Abs, Self::rol),
            0x3E => self.rmw(bus, Abx, Self::rol),
            0x6A => self.rmw(bus, Acc, Self::ror),
            0x66 => self.rmw(bus, Zp, Self::ror),
            0x76 => self.rmw(bus, Zpx, Self::ror),
            0x6E => self.rmw(bus, Abs, Self::ror),
            0x7E => self.rmw(bus, Abx, Self::ror),
            0xE6 => self.rmw(bus, Zp, Self::inc),
            0xF6 => self.rmw(bus, Zpx, Self::inc),
            0xEE => self.rmw(bus, Abs, Self::inc),
            0xFE => self.rmw(bus, Abx, Self::inc),
            0xC6 => self.rmw(bus, Zp, Self::dec),
            0xD6 => self.rmw(bus, Zpx, Self::dec),
            0xCE => self.rmw(bus, Abs, Self::dec),
            0xDE => self.rmw(bus, Abx, Self::dec),

            // --- Register transfers and flag operations
            0xAA => self.imp(bus, |c| {
                c.regs.x = c.regs.a;
                c.set_zn(c.regs.x)
            }),
            0xA8 => self.imp(bus, |c| {
                c.regs.y = c.regs.a;
                c.set_zn(c.regs.y)
            }),
            0x8A => self.imp(bus, |c| {
                c.regs.a = c.regs.x;
                c.set_zn(c.regs.a)
            }),
            0x98 => self.imp(bus, |c| {
                c.regs.a = c.regs.y;
                c.set_zn(c.regs.a)
            }),
            0xBA => self.imp(bus, |c| {
                c.regs.x = c.regs.sp;
                c.set_zn(c.regs.x)
            }),
            0x9A => self.imp(bus, |c| c.regs.sp = c.regs.x),
            0xE8 => self.imp(bus, |c| {
                c.regs.x = c.regs.x.wrapping_add(1);
                c.set_zn(c.regs.x)
            }),
            0xC8 => self.imp(bus, |c| {
                c.regs.y = c.regs.y.wrapping_add(1);
                c.set_zn(c.regs.y)
            }),
            0xCA => self.imp(bus, |c| {
                c.regs.x = c.regs.x.wrapping_sub(1);
                c.set_zn(c.regs.x)
            }),
            0x88 => self.imp(bus, |c| {
                c.regs.y = c.regs.y.wrapping_sub(1);
                c.set_zn(c.regs.y)
            }),
            0x18 => self.imp(bus, |c| c.set_flag(C, false)),
            0x38 => self.imp(bus, |c| c.set_flag(C, true)),
            0x58 => self.imp(bus, |c| c.set_flag(I, false)),
            0x78 => self.imp(bus, |c| c.set_flag(I, true)),
            0xB8 => self.imp(bus, |c| c.set_flag(V, false)),
            0xD8 => self.imp(bus, |c| c.set_flag(D, false)),
            0xF8 => self.imp(bus, |c| c.set_flag(D, true)),
            0xEA | 0x1A | 0x3A | 0x5A | 0x7A | 0xDA | 0xFA => self.imp(bus, |_| {}),

            // --- Stack
            0x48 => {
                self.dummy_read_pc(bus);
                self.push(bus, self.regs.a);
            }
            0x08 => {
                self.dummy_read_pc(bus);
                self.push(bus, self.regs.p | B | U);
            }
            0x68 => {
                self.dummy_read_pc(bus);
                self.dummy_read_stack(bus);
                let v = self.pull(bus);
                self.lda(v);
            }
            0x28 => {
                self.dummy_read_pc(bus);
                self.dummy_read_stack(bus);
                let v = self.pull(bus);
                self.set_p(v);
            }

            // --- Jumps and subroutines
            0x4C => self.regs.pc = self.fetch16(bus),
            0x6C => {
                let ptr = self.fetch16(bus);
                let lo = self.read(bus, ptr);
                // The pointer's high byte is fetched without carrying into
                // the page: JMP ($10FF) reads $10FF and $1000.
                let hi = self.read(bus, (ptr & 0xFF00) | (ptr.wrapping_add(1) & 0x00FF));
                self.regs.pc = u16::from_le_bytes([lo, hi]);
            }
            0x20 => {
                let lo = self.fetch(bus);
                self.dummy_read_stack(bus);
                let pc = self.regs.pc;
                self.push(bus, (pc >> 8) as u8);
                self.push(bus, pc as u8);
                let hi = self.read(bus, self.regs.pc);
                self.regs.pc = u16::from_le_bytes([lo, hi]);
            }
            0x60 => {
                self.dummy_read_pc(bus);
                self.dummy_read_stack(bus);
                let lo = self.pull(bus);
                let hi = self.pull(bus);
                self.regs.pc = u16::from_le_bytes([lo, hi]);
                // RTS pulls the return address minus one; skip that byte.
                self.dummy_read_pc(bus);
                self.regs.pc = self.regs.pc.wrapping_add(1);
            }
            0x40 => {
                self.dummy_read_pc(bus);
                self.dummy_read_stack(bus);
                let p = self.pull(bus);
                self.set_p(p);
                let lo = self.pull(bus);
                let hi = self.pull(bus);
                self.regs.pc = u16::from_le_bytes([lo, hi]);
            }
            0x00 => self.brk(bus),

            // --- Branches
            0x10 => self.branch(bus, self.regs.p & N == 0),
            0x30 => self.branch(bus, self.regs.p & N != 0),
            0x50 => self.branch(bus, self.regs.p & V == 0),
            0x70 => self.branch(bus, self.regs.p & V != 0),
            0x90 => self.branch(bus, self.regs.p & C == 0),
            0xB0 => self.branch(bus, self.regs.p & C != 0),
            0xD0 => self.branch(bus, self.regs.p & Z == 0),
            0xF0 => self.branch(bus, self.regs.p & Z != 0),

            // --- Unofficial: NOPs that still perform their reads
            0x80 | 0x82 | 0x89 | 0xC2 | 0xE2 => self.rd(bus, Imm, Self::nop_read),
            0x04 | 0x44 | 0x64 => self.rd(bus, Zp, Self::nop_read),
            0x14 | 0x34 | 0x54 | 0x74 | 0xD4 | 0xF4 => self.rd(bus, Zpx, Self::nop_read),
            0x0C => self.rd(bus, Abs, Self::nop_read),
            0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC => self.rd(bus, Abx, Self::nop_read),

            // --- Unofficial: combined RMW
            0x07 => self.rmw(bus, Zp, Self::slo),
            0x17 => self.rmw(bus, Zpx, Self::slo),
            0x0F => self.rmw(bus, Abs, Self::slo),
            0x1F => self.rmw(bus, Abx, Self::slo),
            0x1B => self.rmw(bus, Aby, Self::slo),
            0x03 => self.rmw(bus, Izx, Self::slo),
            0x13 => self.rmw(bus, Izy, Self::slo),
            0x27 => self.rmw(bus, Zp, Self::rla),
            0x37 => self.rmw(bus, Zpx, Self::rla),
            0x2F => self.rmw(bus, Abs, Self::rla),
            0x3F => self.rmw(bus, Abx, Self::rla),
            0x3B => self.rmw(bus, Aby, Self::rla),
            0x23 => self.rmw(bus, Izx, Self::rla),
            0x33 => self.rmw(bus, Izy, Self::rla),
            0x47 => self.rmw(bus, Zp, Self::sre),
            0x57 => self.rmw(bus, Zpx, Self::sre),
            0x4F => self.rmw(bus, Abs, Self::sre),
            0x5F => self.rmw(bus, Abx, Self::sre),
            0x5B => self.rmw(bus, Aby, Self::sre),
            0x43 => self.rmw(bus, Izx, Self::sre),
            0x53 => self.rmw(bus, Izy, Self::sre),
            0x67 => self.rmw(bus, Zp, Self::rra),
            0x77 => self.rmw(bus, Zpx, Self::rra),
            0x6F => self.rmw(bus, Abs, Self::rra),
            0x7F => self.rmw(bus, Abx, Self::rra),
            0x7B => self.rmw(bus, Aby, Self::rra),
            0x63 => self.rmw(bus, Izx, Self::rra),
            0x73 => self.rmw(bus, Izy, Self::rra),
            0xC7 => self.rmw(bus, Zp, Self::dcp),
            0xD7 => self.rmw(bus, Zpx, Self::dcp),
            0xCF => self.rmw(bus, Abs, Self::dcp),
            0xDF => self.rmw(bus, Abx, Self::dcp),
            0xDB => self.rmw(bus, Aby, Self::dcp),
            0xC3 => self.rmw(bus, Izx, Self::dcp),
            0xD3 => self.rmw(bus, Izy, Self::dcp),
            0xE7 => self.rmw(bus, Zp, Self::isb),
            0xF7 => self.rmw(bus, Zpx, Self::isb),
            0xEF => self.rmw(bus, Abs, Self::isb),
            0xFF => self.rmw(bus, Abx, Self::isb),
            0xFB => self.rmw(bus, Aby, Self::isb),
            0xE3 => self.rmw(bus, Izx, Self::isb),
            0xF3 => self.rmw(bus, Izy, Self::isb),

            // --- Unofficial: loads and stores
            0xA7 => self.rd(bus, Zp, Self::lax),
            0xB7 => self.rd(bus, Zpy, Self::lax),
            0xAF => self.rd(bus, Abs, Self::lax),
            0xBF => self.rd(bus, Aby, Self::lax),
            0xA3 => self.rd(bus, Izx, Self::lax),
            0xB3 => self.rd(bus, Izy, Self::lax),
            0xBB => self.rd(bus, Aby, Self::las),
            0x87 => self.wr(bus, Zp, self.regs.a & self.regs.x),
            0x97 => self.wr(bus, Zpy, self.regs.a & self.regs.x),
            0x8F => self.wr(bus, Abs, self.regs.a & self.regs.x),
            0x83 => self.wr(bus, Izx, self.regs.a & self.regs.x),

            // --- Unofficial: immediate ALU
            0x0B | 0x2B => self.rd(bus, Imm, Self::anc),
            0x4B => self.rd(bus, Imm, Self::alr),
            0x6B => self.rd(bus, Imm, Self::arr),
            0xCB => self.rd(bus, Imm, Self::axs),
            0x8B => self.rd(bus, Imm, Self::ane),
            0xAB => self.rd(bus, Imm, Self::lxa),

            // --- Unofficial: unstable high-byte stores
            0x9C => self.sh_abs(bus, self.regs.x, self.regs.y),
            0x9E => self.sh_abs(bus, self.regs.y, self.regs.x),
            0x9F => self.sh_abs(bus, self.regs.y, self.regs.a & self.regs.x),
            0x9B => {
                self.regs.sp = self.regs.a & self.regs.x;
                self.sh_abs(bus, self.regs.y, self.regs.sp);
            }
            0x93 => {
                let ptr = self.fetch(bus);
                let lo = self.read(bus, ptr as u16);
                let hi = self.read(bus, ptr.wrapping_add(1) as u16);
                let base = u16::from_le_bytes([lo, hi]);
                self.sh_store(bus, base, self.regs.y, self.regs.a & self.regs.x);
            }

            // --- JAM: the CPU locks up until reset.
            0x02 | 0x12 | 0x22 | 0x32 | 0x42 | 0x52 | 0x62 | 0x72 | 0x92 | 0xB2 | 0xD2 | 0xF2 => {
                self.dummy_read_pc(bus);
                self.regs.pc = self.regs.pc.wrapping_sub(1);
                self.jammed = true;
            }
        }
    }
}
