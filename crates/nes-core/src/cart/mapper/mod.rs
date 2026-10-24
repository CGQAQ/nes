//! Cartridge boards. Each mapper decides how the CPU and PPU address spaces
//! map onto cartridge memory, and may add IRQs or expansion audio.

mod bandai;
mod discrete;
mod fme7;
mod jp_misc;
mod mmc1;
mod mmc2;
mod mmc3;
mod mmc5;
mod multicart;
mod namco163;
mod nrom;
mod rambo1;
mod vrc2_4;
mod vrc6;
mod vrc_irq;
mod vrc_other;

pub use bandai::Bandai;
pub use discrete::Discrete;
pub use fme7::Fme7;
pub use jp_misc::JpMisc;
pub use mmc1::Mmc1;
pub use mmc2::Mmc2;
pub use mmc3::Mmc3;
pub use mmc5::Mmc5;
pub use multicart::Multicart;
pub use namco163::Namco163;
pub use nrom::Nrom;
pub use rambo1::Rambo1;
pub use vrc_other::{Vrc1, Vrc3, Vrc7};
pub use vrc2_4::Vrc24;
pub use vrc6::Vrc6;

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
    Discrete,
    Mmc1,
    Mmc2,
    Mmc3,
    Mmc5,
    Vrc24,
    Vrc6,
    Vrc7,
    Vrc1,
    Vrc3,
    Fme7,
    Namco163,
    Bandai,
    JpMisc,
    Rambo1,
    Multicart,
}

/// Which per-cycle hooks a board actually uses, so the bus can skip the
/// dispatch for the rest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hooks {
    /// `cpu_clock` and `irq` matter.
    pub clocked: bool,
    /// `expansion_audio` matters.
    pub audio: bool,
}

impl Cartridge {
    pub fn hooks(&self) -> Hooks {
        use Cartridge::*;
        Hooks {
            clocked: !matches!(
                self,
                Nrom(_) | Discrete(_) | Mmc2(_) | Multicart(_) | Vrc1(_)
            ),
            audio: matches!(self, Mmc5(_) | Vrc6(_) | Vrc7(_) | Fme7(_) | Namco163(_)),
        }
    }
}

/// Mapper numbers [`create`] accepts.
pub const SUPPORTED_MAPPERS: &[u16] = &[
    0, 1, 2, 3, 4, 5, 7, 9, 10, 11, 13, 16, 18, 19, 21, 22, 23, 24, 25, 26, 28, 30, 32, 33, 34, 38,
    48, 64, 65, 66, 68, 69, 70, 71, 73, 75, 78, 79, 80, 85, 87, 88, 89, 93, 94, 95, 113, 118, 119,
    140, 146, 152, 153, 154, 155, 159, 180, 184, 185, 206, 207, 210, 228, 232,
];

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
        1 | 155 => Mmc1::new(init).into(),
        4 => {
            let variant = match init.info.submapper {
                1 => mmc3::Variant::Mmc6,
                4 => mmc3::Variant::Mmc3A,
                _ => mmc3::Variant::Mmc3,
            };
            Mmc3::new(init, variant).into()
        }
        9 => Mmc2::new(init, false).into(),
        10 => Mmc2::new(init, true).into(),
        88 => Mmc3::new(init, mmc3::Variant::Namco88).into(),
        95 => Mmc3::new(init, mmc3::Variant::Namco95).into(),
        118 => Mmc3::new(init, mmc3::Variant::TxSrom).into(),
        119 => Mmc3::new(init, mmc3::Variant::Tqrom).into(),
        154 => Mmc3::new(init, mmc3::Variant::Namco154).into(),
        206 => Mmc3::new(init, mmc3::Variant::Namco108).into(),
        5 => Mmc5::new(init).into(),
        16 | 153 | 159 => Bandai::new(init).into(),
        19 | 210 => Namco163::new(init).into(),
        21 | 22 | 23 | 25 => Vrc24::new(init).into(),
        24 | 26 => Vrc6::new(init).into(),
        28 => Multicart::new(init, multicart::Kind::Action53).into(),
        30 => Multicart::new(init, multicart::Kind::Unrom512).into(),
        64 => Rambo1::new(init).into(),
        69 => Fme7::new(init).into(),
        73 => Vrc3::new(init).into(),
        75 => Vrc1::new(init).into(),
        85 => Vrc7::new(init).into(),
        228 => Multicart::new(init, multicart::Kind::Action52).into(),
        232 => Multicart::new(init, multicart::Kind::Quattro).into(),
        m if jp_misc::Kind::from_mapper(m).is_some() => {
            let kind = jp_misc::Kind::from_mapper(m).unwrap();
            JpMisc::new(init, kind).into()
        }
        m if discrete::Kind::from_mapper(m, init.info.chr_rom_size).is_some() => {
            let kind = discrete::Kind::from_mapper(m, init.info.chr_rom_size).unwrap();
            Discrete::new(init, kind).into()
        }
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
