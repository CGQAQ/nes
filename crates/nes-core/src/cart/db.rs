//! Corrections for commonly circulated dumps with wrong iNES headers, keyed
//! by CRC32 of PRG+CHR. Only applied to headers that aren't NES 2.0, since a
//! NES 2.0 header was written deliberately.

use super::board::Mirroring;
use super::ines::{HeaderFormat, RomInfo};

struct Fix {
    crc32: u32,
    mapper: Option<u16>,
    submapper: Option<u8>,
    mirroring: Option<Mirroring>,
    battery: Option<bool>,
}

#[allow(dead_code)]
const fn fix(crc32: u32) -> Fix {
    Fix {
        crc32,
        mapper: None,
        submapper: None,
        mirroring: None,
        battery: None,
    }
}

/// Entries look like `Fix { mapper: Some(4), ..fix(0x0123_4567) }`. Only add
/// CRCs verified against a known dump.
static FIXES: &[Fix] = &[];

pub fn apply_fixes(info: &mut RomInfo) {
    if info.format == HeaderFormat::Nes2 {
        return;
    }
    if let Some(f) = FIXES.iter().find(|f| f.crc32 == info.crc32) {
        if let Some(m) = f.mapper {
            info.mapper = m;
        }
        if let Some(s) = f.submapper {
            info.submapper = s;
        }
        if let Some(m) = f.mirroring {
            info.mirroring = m;
        }
        if let Some(b) = f.battery {
            info.battery = b;
            if b && info.prg_nvram_size == 0 {
                info.prg_nvram_size = info.prg_ram_size.max(0x2000);
                info.prg_ram_size = 0;
            }
        }
    }
}
