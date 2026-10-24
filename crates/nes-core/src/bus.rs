//! The CPU's memory map and the system clock.
//!
//! Each CPU cycle is split around its bus access in master-clock units (on
//! NTSC a read happens 5/12 of the way through the cycle, a write 7/12), and
//! the PPU is caught up to the exact master clock before and after the
//! access. That ordering is what makes `$2002` races, NMI timing and
//! mid-scanline register writes behave like hardware.
//!
//! OAM DMA and DMC DMA are serviced here by stealing read cycles from the
//! CPU, including the halt, dummy and alignment cycles and the side effects
//! of the repeated CPU reads they cause.

use crate::apu::Apu;
use crate::cart::mapper::Hooks;
use crate::cart::{Cartridge, Mapper};
use crate::cheats::Cheats;
use crate::cpu::CpuBus;
use crate::debug::{Debugger, cdl};
use crate::input::Input;
use crate::ppu::Ppu;
use crate::region::Region;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct DmaState {
    /// A DMA wants to halt the CPU on its next read cycle.
    need_halt: bool,
    dmc_running: bool,
    dmc_need_dummy: bool,
    dmc_abort: bool,
    oam_active: bool,
    oam_page: u8,
}

/// Open-bus bits of `$4016/$4017` on a front-loading NES: only D0-D4 are
/// driven by the controller ports.
const INPUT_OPEN_BUS_MASK: u8 = 0xE0;

#[derive(Serialize, Deserialize)]
pub struct Bus {
    region: Region,
    pub ram: Vec<u8>,
    pub ppu: Ppu,
    pub apu: Apu,
    pub cart: Cartridge,
    pub input: Input,
    pub open_bus: u8,
    master_clock: u64,
    /// CPU cycles since power-on, DMA cycles included.
    pub cycles: u64,
    /// CPU/PPU clock alignment in master clocks.
    ppu_offset: u64,
    dma: DmaState,
    /// Derived from the cartridge type; recomputed after loading a state.
    #[serde(skip)]
    hooks: Hooks,
    /// CPU cycle of the last read of each controller port.
    input_read_cycle: [u64; 2],
    #[serde(skip)]
    pub dbg: Debugger,
    #[serde(skip)]
    pub cheats: Cheats,
}

impl Bus {
    pub fn new(region: Region, cart: Cartridge, sample_rate: f64) -> Self {
        let hooks = cart.hooks();
        Self {
            region,
            ram: vec![0; 0x800],
            ppu: Ppu::new(region),
            apu: Apu::new(region, sample_rate),
            cart,
            input: Input::default(),
            open_bus: 0,
            master_clock: 0,
            cycles: 0,
            ppu_offset: 1,
            dma: DmaState::default(),
            hooks,
            input_read_cycle: [u64::MAX / 2; 2],
            dbg: Debugger::new(),
            cheats: Cheats::default(),
        }
    }

    pub fn region(&self) -> Region {
        self.region
    }

    /// Moves the parts that aren't saved in states (debugger, cheats) from
    /// the old bus into this one after loading.
    pub(crate) fn adopt_unsaved(&mut self, old: &mut Bus) {
        self.hooks = self.cart.hooks();
        std::mem::swap(&mut self.dbg, &mut old.dbg);
        std::mem::swap(&mut self.cheats, &mut old.cheats);
        self.ppu.take_framebuffer_from(&mut old.ppu);
        self.apu.mute = old.apu.mute;
    }

    pub(crate) fn reset(&mut self) {
        self.dma = DmaState::default();
        self.ppu.reset();
        self.apu.reset();
        self.cart.reset();
    }

    // ------------------------------------------------------------------
    // Clocking.

    #[inline]
    fn ppu_target(&self) -> u64 {
        self.master_clock.saturating_sub(self.ppu_offset)
    }

    #[inline]
    fn start_cycle(&mut self, read: bool) {
        let (before, after) = self.region.cycle_split();
        self.master_clock += if read { before } else { after };
        self.cycles += 1;
        let target = self.ppu_target();
        self.ppu.run_until(target, &mut self.cart);
        let expansion = if self.hooks.audio {
            self.cart.expansion_audio()
        } else {
            0.0
        };
        self.apu.tick(expansion);
        if self.hooks.clocked {
            self.cart.cpu_clock();
        }
        if self.apu.dmc.dma_pending {
            self.apu.dmc.dma_pending = false;
            self.start_dmc_dma();
        }
    }

    #[inline]
    fn end_cycle(&mut self, read: bool) {
        let (before, after) = self.region.cycle_split();
        self.master_clock += if read { after } else { before };
        let target = self.ppu_target();
        self.ppu.run_until(target, &mut self.cart);
    }

    fn start_dmc_dma(&mut self) {
        self.dma.dmc_running = true;
        self.dma.dmc_need_dummy = true;
        self.dma.need_halt = true;
    }

    fn stop_dmc_dma(&mut self) {
        if self.dma.dmc_running {
            if self.dma.need_halt {
                // Not started yet: cancel outright.
                self.dma.dmc_running = false;
                self.dma.dmc_need_dummy = false;
                self.dma.need_halt = false;
            } else {
                self.dma.dmc_abort = true;
            }
        }
    }

    // ------------------------------------------------------------------
    // Memory map (no clocking).

    fn read_mem(&mut self, addr: u16) -> u8 {
        let v = match addr {
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize],
            0x2000..=0x3FFF => self.ppu.cpu_read(addr, &mut self.cart),
            0x4015 => {
                // Internal to the CPU: doesn't drive the external data bus.
                let s = self.apu.read_status();
                return (s & !0x20) | (self.open_bus & 0x20);
            }
            0x4016 | 0x4017 => {
                // The pad shifts when /OE is released, so reads on back-to-back
                // cycles (DMA halt repeats) only clock it once.
                let port = (addr & 1) as usize;
                let held = self.cycles == self.input_read_cycle[port] + 1;
                self.input_read_cycle[port] = self.cycles;
                let bit = if held {
                    self.input.peek(port)
                } else {
                    self.input.read(port)
                };
                (self.open_bus & INPUT_OPEN_BUS_MASK) | (bit & !INPUT_OPEN_BUS_MASK)
            }
            0x4000..=0x401F => self.open_bus,
            _ => {
                let v = self.cart.cpu_read(addr).unwrap_or(self.open_bus);
                if addr >= 0x8000 {
                    self.cheats.apply(addr, v)
                } else {
                    v
                }
            }
        };
        self.open_bus = v;
        v
    }

    fn write_mem(&mut self, addr: u16, val: u8) {
        self.open_bus = val;
        match addr {
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize] = val,
            0x2000..=0x3FFF => self.ppu.cpu_write(addr, val, &mut self.cart),
            0x4014 => {
                self.dma.oam_page = val;
                self.dma.oam_active = true;
                self.dma.need_halt = true;
            }
            0x4016 => self.input.write_strobe(val),
            0x4015 => {
                self.apu.write(addr, val);
                if val & 0x10 == 0 {
                    self.stop_dmc_dma();
                }
            }
            0x4000..=0x4017 => self.apu.write(addr, val),
            _ => {}
        }
        if addr < 0x4020 {
            self.cart.snoop_write(addr, val);
        } else {
            self.cart.cpu_write(addr, val);
        }
    }

    /// Side-effect-free read for the debugger and tracer.
    pub fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize],
            0x2000..=0x3FFF => self.ppu.cpu_peek(addr),
            0x4015 => (self.apu.peek_status() & !0x20) | (self.open_bus & 0x20),
            0x4016 | 0x4017 => {
                (self.open_bus & INPUT_OPEN_BUS_MASK) | (self.input.peek((addr & 1) as usize) & 1)
            }
            0x4000..=0x401F => self.open_bus,
            _ => {
                let v = self.cart.cpu_peek(addr).unwrap_or(self.open_bus);
                if addr >= 0x8000 {
                    self.cheats.apply(addr, v)
                } else {
                    v
                }
            }
        }
    }

    /// Debugger write: RAM and cartridge RAM only, no register side effects.
    pub fn poke(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize] = val,
            0x6000..=0xFFFF => {
                let b = self.cart.board_mut();
                if let Some(off) = b.prg_rom_offset(addr) {
                    // Patch ROM in place (copy-on-write of the shared image).
                    std::sync::Arc::make_mut(&mut b.prg_rom)[off] = val;
                } else {
                    b.write_prg(addr, val);
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // DMA.

    /// A DMA read cycle. Reproduces the 2A03 quirk where, if the CPU was
    /// halted while reading `$4000-$401F`, the DMA unit's reads also hit the
    /// internal APU/input registers.
    fn dma_read(&mut self, addr: u16, prev: &mut u16, internal: bool) -> u8 {
        let pal = self.region == Region::Pal;
        if !internal {
            *prev = addr;
            if (0x4000..=0x401F).contains(&addr) {
                // Nothing on the external bus answers there.
                return self.open_bus;
            }
            return self.read_mem(addr);
        }
        let reg = 0x4000 | (addr & 0x1F);
        let same = reg == addr;
        let val = match reg {
            0x4015 => {
                let v = self.read_mem(reg);
                if !same {
                    self.read_mem(addr);
                }
                v
            }
            0x4016 | 0x4017 => {
                // Two reads of the same input register in a row only clock
                // the controller once.
                let mut v = if pal || *prev == reg {
                    self.open_bus
                } else {
                    self.read_mem(reg)
                };
                if !same {
                    let ext = self.read_mem(addr);
                    let m = INPUT_OPEN_BUS_MASK;
                    v = (ext & m) | ((v & !m) & (ext & !m));
                }
                v
            }
            _ => self.read_mem(addr),
        };
        *prev = reg;
        val
    }

    fn dma_cycle_start(&mut self) {
        if self.dma.dmc_abort {
            self.dma.dmc_running = false;
            self.dma.dmc_abort = false;
            self.dma.dmc_need_dummy = false;
            self.dma.need_halt = false;
        } else if self.dma.need_halt {
            self.dma.need_halt = false;
        } else if self.dma.dmc_need_dummy {
            self.dma.dmc_need_dummy = false;
        }
        self.start_cycle(true);
    }

    /// Runs all pending DMA, entered in place of the CPU's read of `cpu_addr`.
    fn run_dma(&mut self, cpu_addr: u16) {
        let internal = (cpu_addr & 0xFFE0) == 0x4000;
        // PAL consoles don't repeat the CPU's read during halt/dummy cycles.
        let repeats_read = self.region != Region::Pal;
        let mut skip_first = false;
        if internal && self.dma.dmc_running && (cpu_addr == 0x4016 || cpu_addr == 0x4017) {
            let dmc = self.apu.dmc.dma_address();
            if dmc & 0x1F == cpu_addr & 0x1F {
                skip_first = true;
            }
        }

        // Halt cycle: the CPU's read happens, and is thrown away.
        self.start_cycle(true);
        if repeats_read && !skip_first {
            self.read_mem(cpu_addr);
        }
        self.end_cycle(true);
        self.dma.need_halt = false;

        let mut prev = cpu_addr;
        let mut oam_count: u16 = 0;
        let mut oam_lo: u8 = 0;
        let mut oam_val: u8 = 0;

        while self.dma.dmc_running || self.dma.oam_active {
            let get = self.cycles & 1 == 0;
            if get {
                if self.dma.dmc_running && !self.dma.need_halt && !self.dma.dmc_need_dummy {
                    self.dma_cycle_start();
                    let addr = self.apu.dmc.dma_address();
                    let v = self.dma_read(addr, &mut prev, internal);
                    self.end_cycle(true);
                    self.dma.dmc_running = false;
                    self.dma.dmc_abort = false;
                    self.apu.dmc.dma_complete(v);
                } else if self.dma.oam_active {
                    self.dma_cycle_start();
                    let addr = (self.dma.oam_page as u16) << 8 | oam_lo as u16;
                    oam_val = self.dma_read(addr, &mut prev, internal);
                    self.end_cycle(true);
                    oam_lo = oam_lo.wrapping_add(1);
                    oam_count += 1;
                } else {
                    // DMC still needs its halt/dummy cycles.
                    self.dma_cycle_start();
                    if repeats_read {
                        self.read_mem(cpu_addr);
                    }
                    self.end_cycle(true);
                }
            } else if self.dma.oam_active && oam_count & 1 == 1 {
                self.dma_cycle_start();
                self.ppu.oam_dma_write(oam_val, &mut self.cart);
                self.open_bus = oam_val;
                self.end_cycle(true);
                oam_count += 1;
                if oam_count == 512 {
                    self.dma.oam_active = false;
                }
            } else {
                // Alignment cycle.
                self.dma_cycle_start();
                if repeats_read {
                    self.read_mem(cpu_addr);
                }
                self.end_cycle(true);
            }
        }
    }
}

impl Bus {
    #[inline(always)]
    fn read_cycle(&mut self, addr: u16) -> u8 {
        if self.dma.need_halt {
            self.run_dma(addr);
        }
        self.start_cycle(true);
        let v = self.read_mem(addr);
        self.end_cycle(true);
        v
    }
}

impl CpuBus for Bus {
    // Out of line on purpose: inlining the whole bus cycle into every
    // access site of the CPU core bloats it and costs ~40% speed.
    #[inline(never)]
    fn read(&mut self, addr: u16) -> u8 {
        let v = self.read_cycle(addr);
        if self.dbg.any_rw {
            self.dbg.check_read(addr, v);
        }
        if self.dbg.cdl_enabled {
            let (start, len) = self.dbg.cur_ins;
            if addr.wrapping_sub(start) >= len
                && let Some(off) = self.cart.board().prg_rom_offset(addr)
                && let Some(f) = self.dbg.cdl.get_mut(off)
            {
                *f |= cdl::DATA;
            }
        }
        v
    }

    /// Same cycle as `read`, but invisible to read breakpoints and the
    /// code/data logger (the value is discarded by the CPU).
    #[inline(never)]
    fn dummy_read(&mut self, addr: u16) {
        self.read_cycle(addr);
    }

    // Out of line on purpose: inlining the whole bus cycle into every
    // access site of the CPU core bloats it and costs ~40% speed.
    #[inline(never)]
    fn write(&mut self, addr: u16, val: u8) {
        self.start_cycle(false);
        self.write_mem(addr, val);
        self.end_cycle(false);
        if self.dbg.any_rw {
            self.dbg.check_write(addr, val);
        }
    }

    #[inline]
    fn nmi_line(&self) -> bool {
        self.ppu.nmi_line()
    }

    #[inline]
    fn irq_line(&self) -> bool {
        self.apu.irq_line() || (self.hooks.clocked && self.cart.irq())
    }
}
