//! The whole console: CPU + bus, plus run control for frontends and the
//! debugger.

use crate::bus::Bus;
use crate::cart::{self, Cartridge, Mapper, RomError, RomInfo};
use crate::cpu::disasm;
use crate::cpu::{Cpu, Registers, Step};
use crate::debug::{BreakReason, StepTarget, TraceEntry, cdl};
use crate::input::Buttons;
use crate::region::Region;
use crate::state;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Force a region; `None` uses the ROM header (NTSC if unspecified).
    pub region: Option<Region>,
    pub sample_rate: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            region: None,
            sample_rate: 48_000.0,
        }
    }
}

/// Why [`Nes::run_frame`] (or a step) returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// The PPU entered VBlank: a complete frame is in the framebuffer.
    FrameComplete,
    Break(BreakReason),
    /// The debugger's step target was reached.
    StepDone,
}

#[derive(Serialize, Deserialize)]
struct Machine {
    cpu: Cpu,
    bus: Bus,
}

pub struct Nes {
    pub cpu: Cpu,
    pub bus: Bus,
    info: RomInfo,
    /// The ROM image as loaded, for power cycling.
    rom: Arc<[u8]>,
    jam_reported: bool,
}

impl Nes {
    pub fn new(rom: &[u8], config: Config) -> Result<Self, RomError> {
        let (info, cart) = cart::load(rom)?;
        let region = config.region.or(info.region).unwrap_or_default();
        Ok(Self::from_cartridge(
            rom.into(),
            info,
            cart,
            region,
            config.sample_rate,
        ))
    }

    fn from_cartridge(
        rom: Arc<[u8]>,
        info: RomInfo,
        cart: Cartridge,
        region: Region,
        sample_rate: f64,
    ) -> Self {
        let mut nes = Self {
            cpu: Cpu::new(),
            bus: Bus::new(region, cart, sample_rate),
            info,
            rom,
            jam_reported: false,
        };
        let prg_len = nes.bus.cart.board().prg_rom_len();
        nes.bus.dbg.reset_cdl(prg_len);
        nes.cpu.reset(&mut nes.bus, true);
        nes
    }

    pub fn rom_info(&self) -> &RomInfo {
        &self.info
    }

    pub fn region(&self) -> Region {
        self.bus.region()
    }

    /// Power cycle, keeping battery RAM.
    pub fn power_cycle(&mut self) {
        let region = self.bus.region();
        let sample_rate = self.bus.apu.sample_rate();
        let save = self.bus.cart.save_data();
        // Rebuild the cartridge from the ROM image so mapper registers power
        // up too.
        let (_, cart) = cart::load(&self.rom).expect("ROM loaded before");
        let mut fresh = Bus::new(region, cart, sample_rate);
        fresh.adopt_unsaved(&mut self.bus);
        self.bus = fresh;
        if let Some(save) = save {
            self.bus.cart.load_save_data(&save);
        }
        self.cpu = Cpu::new();
        self.cpu.reset(&mut self.bus, true);
        self.jam_reported = false;
    }

    /// The console's RESET button.
    pub fn reset(&mut self) {
        self.bus.reset();
        self.cpu.reset(&mut self.bus, false);
        self.jam_reported = false;
    }

    pub fn set_buttons(&mut self, port: usize, buttons: Buttons) {
        self.bus.input.ports[port & 1].buttons = buttons;
    }

    pub fn framebuffer(&self) -> &[u16] {
        self.bus.ppu.framebuffer()
    }

    pub fn drain_audio(&mut self, out: &mut Vec<f32>) {
        self.bus.apu.drain_samples(out);
    }

    pub fn regs(&self) -> Registers {
        self.cpu.regs
    }

    pub fn peek(&self, addr: u16) -> u8 {
        self.bus.peek(addr)
    }

    pub fn frame(&self) -> u64 {
        self.bus.ppu.frame
    }

    pub fn cycles(&self) -> u64 {
        self.bus.cycles
    }

    // ------------------------------------------------------------------
    // Run control.

    /// Runs until the next VBlank (a complete frame), a breakpoint, or the
    /// debugger's step target.
    pub fn run_frame(&mut self) -> StopReason {
        self.bus.ppu.frame_complete = false;
        loop {
            if let Some(stop) = self.step_checked() {
                return stop;
            }
            if self.bus.ppu.frame_complete {
                return StopReason::FrameComplete;
            }
        }
    }

    /// Runs for at most `max_cycles` CPU cycles; used by headless tools.
    pub fn run_cycles(&mut self, max_cycles: u64) -> Option<StopReason> {
        let end = self.bus.cycles + max_cycles;
        while self.bus.cycles < end {
            if let Some(stop) = self.step_checked() {
                return Some(stop);
            }
        }
        None
    }

    /// Executes exactly one instruction (or interrupt entry), ignoring an
    /// execution breakpoint at the current PC.
    pub fn step_instruction(&mut self) -> StopReason {
        self.resume();
        self.bus.dbg.step_target = Some(StepTarget::Instructions(1));
        self.step_checked().unwrap_or(StopReason::StepDone)
    }

    /// Lets the next run continue past a breakpoint at the current PC.
    pub fn resume(&mut self) {
        self.bus.dbg.resume_skip = Some(self.cpu.regs.pc);
    }

    /// Sets up "step over": runs through a JSR at PC. Call [`Nes::run_frame`]
    /// until it returns [`StopReason::StepDone`].
    pub fn begin_step_over(&mut self) {
        let pc = self.cpu.regs.pc;
        let op = self.bus.peek(pc);
        self.resume();
        self.bus.dbg.step_target = Some(if op == 0x20 && !self.cpu.interrupt_pending() {
            StepTarget::Return {
                pc: pc.wrapping_add(3),
                sp: self.cpu.regs.sp,
            }
        } else {
            StepTarget::Instructions(1)
        });
    }

    pub fn begin_step_out(&mut self) {
        self.resume();
        self.bus.dbg.step_target = Some(StepTarget::Out {
            sp: self.cpu.regs.sp,
        });
    }

    pub fn begin_run_to_scanline(&mut self, scanline: i16) {
        self.resume();
        self.bus.dbg.step_target = Some(StepTarget::Scanline(scanline));
    }

    /// One CPU step with all debugger checks. `Some` means stop.
    fn step_checked(&mut self) -> Option<StopReason> {
        if self.bus.dbg.is_idle() {
            // Fast path: no breakpoints, tracing or stepping.
            if self.cpu.step(&mut self.bus) == Step::Jammed && !self.jam_reported {
                self.jam_reported = true;
                log::warn!("CPU jammed at ${:04X}", self.cpu.regs.pc);
            }
            return None;
        }
        let pc = self.cpu.regs.pc;
        let interrupt = self.cpu.interrupt_pending();
        let dbg = &mut self.bus.dbg;

        if !interrupt && dbg.any_exec && dbg.has_exec_breakpoint(pc) && dbg.resume_skip != Some(pc)
        {
            dbg.step_target = None;
            return Some(StopReason::Break(BreakReason::Exec(pc)));
        }
        dbg.resume_skip = None;

        if dbg.trace_enabled || dbg.cdl_enabled {
            self.observe_instruction(pc, interrupt);
        }

        let opcode = self.bus.peek(pc);
        let start_scanline = self.bus.ppu.scanline;
        let step = self.cpu.step(&mut self.bus);

        let dbg = &mut self.bus.dbg;
        let mut stop = None;
        match step {
            Step::Nmi if dbg.break_on.nmi => stop = Some(BreakReason::Nmi),
            Step::Irq if dbg.break_on.irq => stop = Some(BreakReason::Irq),
            Step::Instruction if opcode == 0x00 && dbg.break_on.brk => {
                stop = Some(BreakReason::Brk(pc))
            }
            Step::Jammed if !self.jam_reported => {
                self.jam_reported = true;
                log::warn!("CPU jammed at ${pc:04X}");
                if dbg.break_on.jam {
                    stop = Some(BreakReason::Jam(pc));
                }
            }
            _ => {}
        }
        if stop.is_none() {
            stop = dbg.pending_hit.take();
        } else {
            dbg.pending_hit = None;
        }
        if let Some(reason) = stop {
            dbg.step_target = None;
            return Some(StopReason::Break(reason));
        }

        if let Some(target) = dbg.step_target {
            let regs = &self.cpu.regs;
            let done = match target {
                StepTarget::Instructions(n) => {
                    if n <= 1 {
                        true
                    } else {
                        dbg.step_target = Some(StepTarget::Instructions(n - 1));
                        false
                    }
                }
                StepTarget::Return { pc, sp } => regs.pc == pc && regs.sp == sp,
                StepTarget::Out { sp } => {
                    step == Step::Instruction && (opcode == 0x60 || opcode == 0x40) && regs.sp > sp
                }
                StepTarget::Scanline(line) => {
                    self.bus.ppu.scanline == line && start_scanline != line
                }
            };
            if done {
                dbg.step_target = None;
                return Some(StopReason::StepDone);
            }
        }
        None
    }

    fn observe_instruction(&mut self, pc: u16, interrupt: bool) {
        let bus = &self.bus;
        let ins = disasm::decode(pc, |a| bus.peek(a));
        let len = if interrupt { 0 } else { ins.len };
        if self.bus.dbg.cdl_enabled && !interrupt {
            for i in 0..ins.len {
                if let Some(off) = self.bus.cart.board().prg_rom_offset(pc.wrapping_add(i))
                    && let Some(f) = self.bus.dbg.cdl.get_mut(off)
                {
                    *f |= cdl::CODE;
                }
            }
            if let Some(target) = ins.target()
                && let Some(off) = self.bus.cart.board().prg_rom_offset(target)
                && let Some(f) = self.bus.dbg.cdl.get_mut(off)
            {
                *f |= cdl::ENTRY;
            }
        }
        self.bus.dbg.cur_ins = (pc, len);
        if self.bus.dbg.trace_enabled {
            // NMI wins over IRQ when both are pending.
            let label = interrupt.then(|| if self.cpu.nmi_pending() { "NMI" } else { "IRQ" });
            let bus = &self.bus;
            let entry = TraceEntry::capture(
                ins,
                self.cpu.regs,
                bus.cycles,
                bus.ppu.scanline,
                bus.ppu.dot,
                label,
                |a| bus.peek(a),
            );
            self.bus.dbg.trace.push(entry);
        }
    }

    // ------------------------------------------------------------------
    // Save states.

    pub fn save_state(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct MachineRef<'a> {
            cpu: &'a Cpu,
            bus: &'a Bus,
        }
        let payload = postcard::to_allocvec(&MachineRef {
            cpu: &self.cpu,
            bus: &self.bus,
        })
        .expect("serializing in-memory state cannot fail");
        state::encode(self.info.crc32, &payload)
    }

    pub fn load_state(&mut self, data: &[u8]) -> Result<(), RomError> {
        let payload = state::decode(self.info.crc32, data)?;
        let mut m: Machine =
            postcard::from_bytes(&payload).map_err(|e| RomError::BadState(e.to_string()))?;
        let board = self.bus.cart.board();
        let (prg, chr) = (board.prg_rom.clone(), board.chr_rom.clone());
        m.bus.cart.board_mut().attach_rom(prg, chr);
        m.bus.adopt_unsaved(&mut self.bus);
        self.cpu = m.cpu;
        self.bus = m.bus;
        self.jam_reported = self.cpu.is_jammed();
        Ok(())
    }

    // ------------------------------------------------------------------

    pub fn save_data(&self) -> Option<Vec<u8>> {
        self.bus.cart.save_data()
    }

    pub fn load_save_data(&mut self, data: &[u8]) {
        self.bus.cart.load_save_data(data);
    }
}
