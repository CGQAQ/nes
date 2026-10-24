//! Save-state envelope: `NESS` magic, format version, ROM CRC32, then the
//! LZ4-compressed postcard encoding of the machine.

use crate::cart::RomError;

const MAGIC: &[u8; 4] = b"NESS";
/// Bump whenever a serialized struct changes shape.
pub const VERSION: u16 = 1;

pub fn encode(rom_crc: u32, payload: &[u8]) -> Vec<u8> {
    let compressed = lz4_flex::compress_prepend_size(payload);
    let mut out = Vec::with_capacity(compressed.len() + 10);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&rom_crc.to_le_bytes());
    out.extend_from_slice(&compressed);
    out
}

pub fn decode(rom_crc: u32, data: &[u8]) -> Result<Vec<u8>, RomError> {
    if data.len() < 10 || &data[..4] != MAGIC {
        return Err(RomError::BadState("not a save state".into()));
    }
    let version = u16::from_le_bytes([data[4], data[5]]);
    if version != VERSION {
        return Err(RomError::BadState(format!(
            "state version {version}, expected {VERSION}"
        )));
    }
    let crc = u32::from_le_bytes([data[6], data[7], data[8], data[9]]);
    if crc != rom_crc {
        return Err(RomError::BadState(
            "state was saved from a different ROM".into(),
        ));
    }
    lz4_flex::decompress_size_prepended(&data[10..]).map_err(|e| RomError::BadState(e.to_string()))
}
