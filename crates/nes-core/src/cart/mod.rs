//! Cartridges: ROM image parsing, header fixes, and mapper construction.

pub mod board;
mod db;
pub mod ines;
pub mod mapper;

pub use board::{Board, Mirroring};
pub use ines::{ConsoleType, HeaderFormat, Rom, RomError, RomInfo};
pub use mapper::{Cartridge, Mapper, SUPPORTED_MAPPERS};

/// Parses a `.nes` (or zipped `.nes`) image, corrects known-bad headers, and
/// builds the matching mapper.
pub fn load(data: &[u8]) -> Result<(RomInfo, Cartridge), RomError> {
    let data = ines::extract(data)?;
    let mut rom = ines::parse(&data)?;
    db::apply_fixes(&mut rom.info);
    let info = rom.info.clone();
    let cart = mapper::create(rom)?;
    Ok((info, cart))
}
