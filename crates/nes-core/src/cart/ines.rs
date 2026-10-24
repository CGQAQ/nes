//! iNES / NES 2.0 header parsing.

use super::board::Mirroring;
use crate::region::Region;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum RomError {
    #[error("not an iNES file (missing NES<EOF> signature)")]
    BadMagic,
    #[error("file is truncated: header promises {expected} bytes, file has {actual}")]
    Truncated { expected: usize, actual: usize },
    #[error("mapper {mapper} (submapper {submapper}) is not supported yet")]
    UnsupportedMapper { mapper: u16, submapper: u8 },
    #[error("archive contains no .nes file")]
    EmptyArchive,
    #[error("archive error: {0}")]
    Archive(String),
    #[error("save state does not match: {0}")]
    BadState(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeaderFormat {
    /// iNES with garbage in bytes 7-15 (e.g. "DiskDude!"); upper mapper bits ignored.
    Archaic,
    Ines,
    Nes2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsoleType {
    Nes,
    VsSystem,
    Playchoice,
    Extended,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RomInfo {
    pub format: HeaderFormat,
    pub mapper: u16,
    pub submapper: u8,
    pub prg_rom_size: usize,
    pub chr_rom_size: usize,
    /// Volatile PRG RAM.
    pub prg_ram_size: usize,
    /// Battery-backed PRG RAM.
    pub prg_nvram_size: usize,
    pub chr_ram_size: usize,
    pub chr_nvram_size: usize,
    pub mirroring: Mirroring,
    pub battery: bool,
    pub trainer: bool,
    pub console: ConsoleType,
    /// Timing the cartridge was made for, when the header says.
    pub region: Option<Region>,
    /// CRC32 of PRG + CHR ROM (header excluded), used for game DB lookups.
    pub crc32: u32,
    /// CRC32 of PRG ROM alone.
    pub prg_crc32: u32,
}

impl RomInfo {
    pub fn total_prg_ram(&self) -> usize {
        self.prg_ram_size + self.prg_nvram_size
    }
}

/// A parsed ROM image.
pub struct Rom {
    pub info: RomInfo,
    pub prg_rom: Vec<u8>,
    pub chr_rom: Vec<u8>,
    pub trainer: Option<Vec<u8>>,
}

fn nes2_rom_size(lsb: u8, msb_nibble: u8, unit: usize) -> usize {
    if msb_nibble == 0x0F {
        // Exponent-multiplier notation: 2^E * (MM*2+1).
        let exp = (lsb >> 2) as u32;
        let mul = (lsb & 3) as usize * 2 + 1;
        (1usize << exp.min(40)) * mul
    } else {
        (((msb_nibble as usize) << 8) | lsb as usize) * unit
    }
}

fn shift_size(shift: u8) -> usize {
    if shift == 0 { 0 } else { 64 << shift }
}

pub fn parse(data: &[u8]) -> Result<Rom, RomError> {
    if data.len() < 16 || &data[0..4] != b"NES\x1A" {
        return Err(RomError::BadMagic);
    }
    let h = &data[..16];
    let flags6 = h[6];
    let flags7 = h[7];
    let format = if flags7 & 0x0C == 0x08 {
        HeaderFormat::Nes2
    } else if flags7 & 0x0C == 0 && h[12..16].iter().all(|&b| b == 0) {
        HeaderFormat::Ines
    } else {
        HeaderFormat::Archaic
    };

    let mut mapper = (flags6 >> 4) as u16;
    let mut submapper = 0;
    let (prg_rom_size, chr_rom_size);
    let (prg_ram_size, prg_nvram_size, chr_ram_size, chr_nvram_size);
    let mut region = None;
    let battery = flags6 & 0x02 != 0;
    let console = match flags7 & 0x03 {
        0 => ConsoleType::Nes,
        1 => ConsoleType::VsSystem,
        2 => ConsoleType::Playchoice,
        _ => ConsoleType::Extended,
    };

    match format {
        HeaderFormat::Nes2 => {
            mapper |= (flags7 & 0xF0) as u16 | (((h[8] & 0x0F) as u16) << 8);
            submapper = h[8] >> 4;
            prg_rom_size = nes2_rom_size(h[4], h[9] & 0x0F, 0x4000);
            chr_rom_size = nes2_rom_size(h[5], h[9] >> 4, 0x2000);
            prg_ram_size = shift_size(h[10] & 0x0F);
            prg_nvram_size = shift_size(h[10] >> 4);
            chr_ram_size = shift_size(h[11] & 0x0F);
            chr_nvram_size = shift_size(h[11] >> 4);
            region = match h[12] & 0x03 {
                0 => Some(Region::Ntsc),
                1 => Some(Region::Pal),
                3 => Some(Region::Dendy),
                _ => None,
            };
        }
        HeaderFormat::Ines | HeaderFormat::Archaic => {
            if format == HeaderFormat::Ines {
                mapper |= (flags7 & 0xF0) as u16;
            }
            prg_rom_size = h[4] as usize * 0x4000;
            chr_rom_size = h[5] as usize * 0x2000;
            // iNES 1.0 can't describe RAM; give every board 8 KiB like most
            // emulators do, and CHR RAM when there is no CHR ROM.
            let ram = if format == HeaderFormat::Ines && h[8] > 0 {
                h[8] as usize * 0x2000
            } else {
                0x2000
            };
            if battery {
                prg_ram_size = 0;
                prg_nvram_size = ram;
            } else {
                prg_ram_size = ram;
                prg_nvram_size = 0;
            }
            chr_ram_size = if chr_rom_size == 0 { 0x2000 } else { 0 };
            chr_nvram_size = 0;
        }
    }

    let four_screen = flags6 & 0x08 != 0;
    let mirroring = if four_screen {
        Mirroring::FourScreen
    } else if flags6 & 0x01 != 0 {
        Mirroring::Vertical
    } else {
        Mirroring::Horizontal
    };
    let trainer = flags6 & 0x04 != 0;

    let mut pos = 16;
    let trainer_data = if trainer {
        let t = data.get(pos..pos + 512).ok_or(RomError::Truncated {
            expected: pos + 512,
            actual: data.len(),
        })?;
        pos += 512;
        Some(t.to_vec())
    } else {
        None
    };
    let prg_end = pos + prg_rom_size;
    if data.len() < prg_end {
        return Err(RomError::Truncated {
            expected: prg_end,
            actual: data.len(),
        });
    }
    let prg_rom = data[pos..prg_end].to_vec();
    // Some dumps are missing part of CHR; pad rather than refuse.
    let chr_end = prg_end + chr_rom_size;
    let mut chr_rom = data[prg_end.min(data.len())..chr_end.min(data.len())].to_vec();
    chr_rom.resize(chr_rom_size, 0);

    let crc32 = {
        let mut h = crc32fast::Hasher::new();
        h.update(&prg_rom);
        h.update(&chr_rom);
        h.finalize()
    };
    let prg_crc32 = crc32fast::hash(&prg_rom);

    Ok(Rom {
        info: RomInfo {
            format,
            mapper,
            submapper,
            prg_rom_size,
            chr_rom_size,
            prg_ram_size,
            prg_nvram_size,
            chr_ram_size,
            chr_nvram_size,
            mirroring,
            battery,
            trainer,
            console,
            region,
            crc32,
            prg_crc32,
        },
        prg_rom,
        chr_rom,
        trainer: trainer_data,
    })
}

/// Accepts a raw `.nes` image or a `.zip` containing one.
pub fn extract(data: &[u8]) -> Result<Vec<u8>, RomError> {
    if data.starts_with(b"PK\x03\x04") {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data))
            .map_err(|e| RomError::Archive(e.to_string()))?;
        let mut candidate = None;
        for i in 0..zip.len() {
            let file = zip
                .by_index(i)
                .map_err(|e| RomError::Archive(e.to_string()))?;
            if file
                .name()
                .is_ok_and(|n| n.to_ascii_lowercase().ends_with(".nes"))
            {
                candidate = Some(i);
                break;
            }
        }
        let idx = candidate.ok_or(RomError::EmptyArchive)?;
        let mut file = zip
            .by_index(idx)
            .map_err(|e| RomError::Archive(e.to_string()))?;
        let mut out = Vec::new();
        file.read_to_end(&mut out)
            .map_err(|e| RomError::Archive(e.to_string()))?;
        Ok(out)
    } else {
        Ok(data.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn make_rom(mapper: u8, prg_banks: u8, chr_banks: u8, flags6: u8) -> Vec<u8> {
        let mut v = vec![
            b'N',
            b'E',
            b'S',
            0x1A,
            prg_banks,
            chr_banks,
            (mapper << 4) | flags6,
            mapper & 0xF0,
        ];
        v.resize(16, 0);
        v.resize(
            16 + prg_banks as usize * 0x4000 + chr_banks as usize * 0x2000,
            0,
        );
        v
    }

    #[test]
    fn parses_ines1() {
        let rom = parse(&make_rom(4, 2, 1, 0x03)).unwrap();
        assert_eq!(rom.info.format, HeaderFormat::Ines);
        assert_eq!(rom.info.mapper, 4);
        assert_eq!(rom.info.mirroring, Mirroring::Vertical);
        assert!(rom.info.battery);
        assert_eq!(rom.info.prg_nvram_size, 0x2000);
        assert_eq!(rom.prg_rom.len(), 0x8000);
    }

    #[test]
    fn archaic_header_ignores_upper_mapper_bits() {
        let mut data = make_rom(1, 1, 1, 0);
        data[7] = b'D';
        data[8..16].copy_from_slice(b"iskDude!");
        let rom = parse(&data).unwrap();
        assert_eq!(rom.info.format, HeaderFormat::Archaic);
        assert_eq!(rom.info.mapper, 1);
    }

    #[test]
    fn nes2_sizes() {
        let mut data = make_rom(0, 2, 0, 0);
        data[7] = 0x08;
        data[8] = 0x31; // submapper 3, mapper bits 8-11 = 1
        data[10] = 0x07; // 8 KiB PRG RAM
        data[11] = 0x09; // 32 KiB CHR RAM
        data[12] = 0x01;
        let rom = parse(&data).unwrap();
        assert_eq!(rom.info.format, HeaderFormat::Nes2);
        assert_eq!(rom.info.mapper, 0x100);
        assert_eq!(rom.info.submapper, 3);
        assert_eq!(rom.info.prg_ram_size, 0x2000);
        assert_eq!(rom.info.chr_ram_size, 0x8000);
        assert_eq!(rom.info.region, Some(Region::Pal));
    }
}
