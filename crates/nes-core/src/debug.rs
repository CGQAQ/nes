//! Debugger support: breakpoints, stepping targets, the instruction trace
//! ring buffer, and the code/data logger.
//!
//! Everything here is cheap when unused: breakpoint lookups are bitmap tests
//! guarded by "any breakpoints?" flags, and tracing/CDL only run when enabled.

use crate::cpu::Registers;
use crate::cpu::disasm::{self, Instruction};
use crate::cpu::opcodes::Mode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BreakKind {
    pub exec: bool,
    pub read: bool,
    pub write: bool,
}

impl BreakKind {
    pub const EXEC: BreakKind = BreakKind {
        exec: true,
        read: false,
        write: false,
    };
    pub const READ: BreakKind = BreakKind {
        exec: false,
        read: true,
        write: false,
    };
    pub const WRITE: BreakKind = BreakKind {
        exec: false,
        read: false,
        write: true,
    };
    pub const ACCESS: BreakKind = BreakKind {
        exec: false,
        read: true,
        write: true,
    };

    pub fn label(&self) -> String {
        let mut s = String::new();
        if self.exec {
            s.push('X');
        }
        if self.read {
            s.push('R');
        }
        if self.write {
            s.push('W');
        }
        s
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Breakpoint {
    pub start: u16,
    pub end: u16,
    pub kind: BreakKind,
    pub enabled: bool,
}

impl Breakpoint {
    pub fn exec(addr: u16) -> Self {
        Self {
            start: addr,
            end: addr,
            kind: BreakKind::EXEC,
            enabled: true,
        }
    }
}

/// Why execution stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakReason {
    Exec(u16),
    Read { addr: u16, value: u8 },
    Write { addr: u16, value: u8 },
    Nmi,
    Irq,
    Brk(u16),
    Jam(u16),
}

impl std::fmt::Display for BreakReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BreakReason::Exec(a) => write!(f, "breakpoint at ${a:04X}"),
            BreakReason::Read { addr, value } => write!(f, "read ${addr:04X} = ${value:02X}"),
            BreakReason::Write { addr, value } => write!(f, "write ${addr:04X} <- ${value:02X}"),
            BreakReason::Nmi => write!(f, "NMI"),
            BreakReason::Irq => write!(f, "IRQ"),
            BreakReason::Brk(a) => write!(f, "BRK at ${a:04X}"),
            BreakReason::Jam(a) => write!(f, "CPU jammed at ${a:04X}"),
        }
    }
}

/// Events the debugger can be asked to stop on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreakOn {
    pub nmi: bool,
    pub irq: bool,
    pub brk: bool,
    pub jam: bool,
}

/// A run-until condition for stepping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepTarget {
    /// Stop after `count` more instructions/interrupt entries.
    Instructions(u32),
    /// Step over: stop when PC returns to `pc` with the stack back at `sp`.
    Return { pc: u16, sp: u8 },
    /// Step out: stop after an RTS/RTI that pops the stack above `sp`.
    Out { sp: u8 },
    /// Stop when the PPU reaches this scanline.
    Scanline(i16),
}

struct BitSet(Vec<u64>);

impl BitSet {
    fn new() -> Self {
        BitSet(vec![0; 1024])
    }
    #[inline]
    fn get(&self, i: u16) -> bool {
        self.0[(i >> 6) as usize] & (1 << (i & 63)) != 0
    }
    fn set(&mut self, i: u16) {
        self.0[(i >> 6) as usize] |= 1 << (i & 63);
    }
    fn clear(&mut self) {
        self.0.fill(0);
    }
}

/// One executed instruction (or interrupt entry, with `interrupt` set).
#[derive(Clone, Copy, Debug)]
pub struct TraceEntry {
    pub ins: Instruction,
    pub regs: Registers,
    pub cycle: u64,
    pub scanline: i16,
    pub dot: u16,
    pub interrupt: Option<&'static str>,
    /// Memory the annotated disassembly refers to, captured before execution.
    snapshot: [(u16, u8); 3],
    snapshot_len: u8,
}

impl TraceEntry {
    /// Records `ins` about to execute, snapshotting the memory its annotated
    /// disassembly shows (memory will have changed by the time it's displayed).
    pub(crate) fn capture(
        ins: Instruction,
        regs: Registers,
        cycle: u64,
        scanline: i16,
        dot: u16,
        interrupt: Option<&'static str>,
        peek: impl Fn(u16) -> u8,
    ) -> Self {
        let mut snapshot = [(0u16, 0u8); 3];
        let mut snapshot_len = 0;
        if interrupt.is_none() {
            let (addrs, n) = snapshot_addrs(&ins, &regs, &peek);
            for i in 0..n {
                snapshot[i] = (addrs[i], peek(addrs[i]));
            }
            snapshot_len = n as u8;
        }
        TraceEntry {
            ins,
            regs,
            cycle,
            scanline,
            dot,
            interrupt,
            snapshot,
            snapshot_len,
        }
    }

    /// Disassembly with effective addresses and values (`LDA $0200 = 5A`).
    pub fn text(&self) -> String {
        if let Some(name) = self.interrupt {
            return format!("-- {name} --");
        }
        disasm::format_annotated(&self.ins, &self.regs, |a| self.snapshot_value(a))
    }

    fn snapshot_value(&self, addr: u16) -> u8 {
        self.snapshot[..self.snapshot_len as usize]
            .iter()
            .find(|(a, _)| *a == addr)
            .map(|(_, v)| *v)
            .unwrap_or(0)
    }

    /// A nestest.log-format line.
    pub fn nestest_line(&self) -> String {
        if let Some(name) = self.interrupt {
            return format!("-- {name} --");
        }
        format!(
            "{} PPU:{:3},{:3} CYC:{}",
            disasm::nestest_prefix(&self.ins, &self.regs, |a| self.snapshot_value(a)),
            self.scanline,
            self.dot,
            self.cycle
        )
    }
}

/// Addresses the annotated disassembly of `ins` will display.
fn snapshot_addrs(
    ins: &Instruction,
    regs: &Registers,
    peek: &impl Fn(u16) -> u8,
) -> ([u16; 3], usize) {
    let b1 = ins.bytes[1];
    let w = u16::from_le_bytes([ins.bytes[1], ins.bytes[2]]);
    let zp16 = |p: u8| u16::from_le_bytes([peek(p as u16), peek(p.wrapping_add(1) as u16)]);
    match ins.info().mode {
        Mode::Zp => ([b1 as u16, 0, 0], 1),
        Mode::Zpx => ([b1.wrapping_add(regs.x) as u16, 0, 0], 1),
        Mode::Zpy => ([b1.wrapping_add(regs.y) as u16, 0, 0], 1),
        Mode::Abs => ([w, 0, 0], 1),
        Mode::Abx => ([w.wrapping_add(regs.x as u16), 0, 0], 1),
        Mode::Aby => ([w.wrapping_add(regs.y as u16), 0, 0], 1),
        Mode::Ind => ([w, (w & 0xFF00) | (w.wrapping_add(1) & 0xFF), 0], 2),
        Mode::Izx => {
            let p = b1.wrapping_add(regs.x);
            ([p as u16, p.wrapping_add(1) as u16, zp16(p)], 3)
        }
        Mode::Izy => (
            [
                b1 as u16,
                b1.wrapping_add(1) as u16,
                zp16(b1).wrapping_add(regs.y as u16),
            ],
            3,
        ),
        _ => ([0; 3], 0),
    }
}

/// Fixed-capacity ring of [`TraceEntry`], with a running sequence number so
/// consumers (a file logger, a UI) can ask for "everything since N".
pub struct Trace {
    entries: Vec<TraceEntry>,
    capacity: usize,
    next: u64,
}

impl Trace {
    fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            capacity,
            next: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(1);
        self.entries.clear();
        self.next = 0;
    }

    pub(crate) fn push(&mut self, e: TraceEntry) {
        if self.entries.len() < self.capacity {
            self.entries.push(e);
        } else {
            let i = (self.next % self.capacity as u64) as usize;
            self.entries[i] = e;
        }
        self.next += 1;
    }

    /// Total entries ever recorded.
    pub fn total(&self) -> u64 {
        self.next
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `i`-th oldest retained entry.
    pub fn get(&self, i: usize) -> Option<&TraceEntry> {
        if i >= self.entries.len() {
            return None;
        }
        let oldest = self.next - self.entries.len() as u64;
        let idx = ((oldest + i as u64) % self.capacity as u64) as usize;
        self.entries.get(idx)
    }

    /// Entries with sequence number >= `seq` that are still retained.
    pub fn since(&self, seq: u64) -> impl Iterator<Item = &TraceEntry> {
        let oldest = self.next - self.entries.len() as u64;
        let start = seq.max(oldest);
        (start..self.next)
            .filter_map(move |s| self.entries.get((s % self.capacity as u64) as usize))
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.next = 0;
    }
}

pub mod cdl {
    pub const CODE: u8 = 0x01;
    pub const DATA: u8 = 0x02;
    /// Target of a jump, branch or interrupt vector.
    pub const ENTRY: u8 = 0x04;
}

pub struct Debugger {
    pub breakpoints: Vec<Breakpoint>,
    exec_bps: BitSet,
    read_bps: BitSet,
    write_bps: BitSet,
    pub(crate) any_exec: bool,
    pub(crate) any_rw: bool,
    pub break_on: BreakOn,
    /// Set by the bus when a read/write breakpoint matches; consumed by `Nes`.
    pub(crate) pending_hit: Option<BreakReason>,
    /// Lets execution continue past the breakpoint it is sitting on.
    pub(crate) resume_skip: Option<u16>,
    pub step_target: Option<StepTarget>,

    pub trace_enabled: bool,
    pub trace: Trace,

    pub cdl_enabled: bool,
    pub cdl: Vec<u8>,
    /// Current instruction's bytes, so operand fetches aren't logged as data.
    pub(crate) cur_ins: (u16, u16),
}

impl Default for Debugger {
    fn default() -> Self {
        Self::new()
    }
}

impl Debugger {
    pub fn new() -> Self {
        Self {
            breakpoints: Vec::new(),
            exec_bps: BitSet::new(),
            read_bps: BitSet::new(),
            write_bps: BitSet::new(),
            any_exec: false,
            any_rw: false,
            break_on: BreakOn::default(),
            pending_hit: None,
            resume_skip: None,
            step_target: None,
            trace_enabled: false,
            trace: Trace::new(100_000),
            cdl_enabled: false,
            cdl: Vec::new(),
            cur_ins: (0, 0),
        }
    }

    /// Nothing needs per-instruction attention.
    #[inline]
    pub fn is_idle(&self) -> bool {
        !self.any_exec
            && !self.any_rw
            && !self.trace_enabled
            && !self.cdl_enabled
            && self.step_target.is_none()
            && self.break_on == BreakOn::default()
    }

    /// Rebuilds the lookup bitmaps; call after editing `breakpoints`.
    pub fn update_breakpoints(&mut self) {
        self.exec_bps.clear();
        self.read_bps.clear();
        self.write_bps.clear();
        for bp in self.breakpoints.iter().filter(|b| b.enabled) {
            let (lo, hi) = (bp.start.min(bp.end), bp.start.max(bp.end));
            for a in lo..=hi {
                if bp.kind.exec {
                    self.exec_bps.set(a);
                }
                if bp.kind.read {
                    self.read_bps.set(a);
                }
                if bp.kind.write {
                    self.write_bps.set(a);
                }
            }
        }
        let enabled = self.breakpoints.iter().filter(|b| b.enabled);
        self.any_exec = enabled.clone().any(|b| b.kind.exec);
        self.any_rw = enabled.clone().any(|b| b.kind.read || b.kind.write);
    }

    pub fn toggle_exec_breakpoint(&mut self, addr: u16) {
        if let Some(i) = self
            .breakpoints
            .iter()
            .position(|b| b.kind == BreakKind::EXEC && b.start == addr && b.end == addr)
        {
            self.breakpoints.remove(i);
        } else {
            self.breakpoints.push(Breakpoint::exec(addr));
        }
        self.update_breakpoints();
    }

    pub fn has_exec_breakpoint(&self, addr: u16) -> bool {
        self.any_exec && self.exec_bps.get(addr)
    }

    #[inline]
    pub(crate) fn check_read(&mut self, addr: u16, value: u8) {
        if self.read_bps.get(addr) && self.pending_hit.is_none() {
            self.pending_hit = Some(BreakReason::Read { addr, value });
        }
    }

    #[inline]
    pub(crate) fn check_write(&mut self, addr: u16, value: u8) {
        if self.write_bps.get(addr) && self.pending_hit.is_none() {
            self.pending_hit = Some(BreakReason::Write { addr, value });
        }
    }

    pub fn reset_cdl(&mut self, prg_len: usize) {
        self.cdl = vec![0; prg_len];
    }

    /// Fraction of PRG ROM bytes classified as code / data.
    pub fn cdl_coverage(&self) -> (f32, f32) {
        if self.cdl.is_empty() {
            return (0.0, 0.0);
        }
        let n = self.cdl.len() as f32;
        let code = self.cdl.iter().filter(|&&f| f & cdl::CODE != 0).count() as f32;
        let data = self.cdl.iter().filter(|&&f| f & cdl::DATA != 0).count() as f32;
        (code / n, data / n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_ring_wraps() {
        let mut t = Trace::new(3);
        let e = |pc: u16| TraceEntry {
            ins: Instruction {
                addr: pc,
                opcode: 0xEA,
                bytes: [0xEA, 0, 0],
                len: 1,
            },
            regs: Registers::default(),
            cycle: 0,
            scanline: 0,
            dot: 0,
            interrupt: None,
            snapshot: [(0, 0); 3],
            snapshot_len: 0,
        };
        for pc in 0..5 {
            t.push(e(pc));
        }
        assert_eq!(t.len(), 3);
        assert_eq!(t.get(0).unwrap().ins.addr, 2);
        assert_eq!(t.get(2).unwrap().ins.addr, 4);
        let since: Vec<u16> = t.since(3).map(|e| e.ins.addr).collect();
        assert_eq!(since, [3, 4]);
    }

    #[test]
    fn breakpoint_ranges() {
        let mut d = Debugger::new();
        d.breakpoints.push(Breakpoint {
            start: 0x2000,
            end: 0x2007,
            kind: BreakKind::WRITE,
            enabled: true,
        });
        d.update_breakpoints();
        assert!(d.any_rw && !d.any_exec);
        d.check_write(0x2005, 1);
        assert_eq!(
            d.pending_hit,
            Some(BreakReason::Write {
                addr: 0x2005,
                value: 1
            })
        );
    }
}
