//! Cartridge boards. Each mapper decides how the CPU and PPU address spaces
//! map onto cartridge memory, and may add IRQs or expansion audio.

mod nrom;

pub use nrom::Nrom;

use super::board::{Board, Mirroring};
use super::ines::{Rom, RomError, RomInfo};
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Serialize};

/// Everything a mapper constructor needs.
pub struct MapperInit {
    pub board: Board,
    pub info: RomInfo,
}

#[enum_dispatch]
pub trait Mapper {
    fn board(&self) -> &Board;
    fn board_mut(&mut self) -> &mut Board;

    /// Board name for display.
    fn name(&self) -> &'static str;

    /// CPU read in `$4020-$FFFF`. `None` leaves the data bus floating (open bus).
    fn cpu_read(&mut self, addr: u16) -> Option<u8> {
        self.board().read_prg(addr)
    }

    /// Side-effect-free version of [`Mapper::cpu_read`] for the debugger.
    fn cpu_peek(&self, addr: u16) -> Option<u8> {
        self.board().read_prg(addr)
    }

    /// CPU write in `$4020-$FFFF`.
    fn cpu_write(&mut self, addr: u16, val: u8) {
        self.board_mut().write_prg(addr, val);
    }

    /// Sees CPU writes below `$4020` (PPU and APU registers). MMC5 uses this
    /// to track PPUCTRL/PPUMASK.
    fn snoop_write(&mut self, _addr: u16, _val: u8) {}

    /// PPU read of `$0000-$3EFF` (pattern tables and nametables).
    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.board().read_ppu(addr)
    }

    fn ppu_peek(&self, addr: u16) -> u8 {
        self.board().read_ppu(addr)
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        self.board_mut().write_ppu(addr, val);
    }

    /// The PPU drove `addr` onto its address bus without reading (after a
    /// `$2006` write). Mappers that watch PPU A12 need this.
    fn ppu_address(&mut self, _addr: u16) {}

    /// Called once per CPU cycle.
    fn cpu_clock(&mut self) {}

    /// Level of the cartridge IRQ line.
    fn irq(&self) -> bool {
        false
    }

    /// Expansion audio output, sampled every CPU cycle, roughly in the same
    /// units as the APU mixer (0.0 - 1.0).
    fn expansion_audio(&self) -> f32 {
        0.0
    }

    /// RESET button (not power).
    fn reset(&mut self) {}

    /// Contents to persist across sessions, if the board has a battery.
    fn save_data(&self) -> Option<Vec<u8>> {
        let b = self.board();
        b.battery.then(|| b.prg_ram.clone())
    }

    fn load_save_data(&mut self, data: &[u8]) {
        let ram = &mut self.board_mut().prg_ram;
        let n = ram.len().min(data.len());
        ram[..n].copy_from_slice(&data[..n]);
    }

    /// Mapper registers for the debugger, as (name, value) rows.
    fn debug_state(&self) -> Vec<(&'static str, String)> {
        Vec::new()
    }
}

#[enum_dispatch(Mapper)]
#[derive(Clone, Serialize, Deserialize)]
pub enum Cartridge {
    Nrom,
}

/// Mapper numbers [`create`] accepts.
pub const SUPPORTED_MAPPERS: &[u16] = &[0];

pub fn create(rom: Rom) -> Result<Cartridge, RomError> {
    let info = rom.info.clone();
    let prg_ram = info.total_prg_ram();
    let mut chr_ram = info.chr_ram_size + info.chr_nvram_size;
    if rom.chr_rom.is_empty() && chr_ram == 0 {
        chr_ram = 0x2000;
    }
    let mut board = Board::new(
        rom.prg_rom,
        rom.chr_rom,
        chr_ram,
        prg_ram,
        info.mirroring,
        info.battery,
    );
    if let Some(trainer) = &rom.trainer
        && board.prg_ram.len() >= 0x1200
    {
        board.prg_ram[0x1000..0x1200].copy_from_slice(trainer);
    }
    let init = MapperInit { board, info };
    let cart: Cartridge = match init.info.mapper {
        0 => Nrom::new(init).into(),
        mapper => {
            return Err(RomError::UnsupportedMapper {
                mapper,
                submapper: init.info.submapper,
            });
        }
    };
    Ok(cart)
}

/// Shared helper: decode the mirroring bits most simple mappers use.
#[allow(dead_code)]
pub(crate) fn mirroring_hv(bit_vertical: bool) -> Mirroring {
    if bit_vertical {
        Mirroring::Vertical
    } else {
        Mirroring::Horizontal
    }
}
