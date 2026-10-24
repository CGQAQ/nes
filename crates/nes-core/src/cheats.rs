//! Game Genie and raw ROM-read patches.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cheat {
    /// The code as the user typed it.
    pub code: String,
    pub addr: u16,
    pub value: u8,
    /// Only substitute when the ROM byte equals this (8-letter codes).
    pub compare: Option<u8>,
    pub enabled: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CheatError {
    #[error("Game Genie codes are 6 or 8 letters from APZLGITYEOXUKSVN")]
    BadGameGenie,
    #[error("raw codes look like 8123:45 or 8123?67:45 (hex)")]
    BadRaw,
}

const LETTERS: &[u8; 16] = b"APZLGITYEOXUKSVN";

impl Cheat {
    /// Parses a Game Genie code (`SXIOPO`, `YEUZUGAA`) or a raw patch
    /// (`8123:45`, `8123?67:45`).
    pub fn parse(code: &str) -> Result<Cheat, CheatError> {
        let trimmed: String = code
            .trim()
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .collect();
        if trimmed.contains(':') {
            return Self::parse_raw(&trimmed, code);
        }
        let upper = trimmed.to_ascii_uppercase();
        let n: Vec<u16> = upper
            .bytes()
            .map(|b| LETTERS.iter().position(|&l| l == b).map(|p| p as u16))
            .collect::<Option<_>>()
            .ok_or(CheatError::BadGameGenie)?;
        if n.len() != 6 && n.len() != 8 {
            return Err(CheatError::BadGameGenie);
        }
        let addr = 0x8000
            | ((n[3] & 7) << 12)
            | ((n[5] & 7) << 8)
            | ((n[4] & 8) << 8)
            | ((n[2] & 7) << 4)
            | ((n[1] & 8) << 4)
            | (n[4] & 7)
            | (n[3] & 8);
        let (value, compare) = if n.len() == 6 {
            (
                ((n[1] & 7) << 4) | ((n[0] & 8) << 4) | (n[0] & 7) | (n[5] & 8),
                None,
            )
        } else {
            let v = ((n[1] & 7) << 4) | ((n[0] & 8) << 4) | (n[0] & 7) | (n[7] & 8);
            let c = ((n[7] & 7) << 4) | ((n[6] & 8) << 4) | (n[6] & 7) | (n[5] & 8);
            (v, Some(c as u8))
        };
        Ok(Cheat {
            code: upper,
            addr,
            value: value as u8,
            compare,
            enabled: true,
        })
    }

    fn parse_raw(s: &str, original: &str) -> Result<Cheat, CheatError> {
        let (left, value) = s.split_once(':').ok_or(CheatError::BadRaw)?;
        let (addr, compare) = match left.split_once('?') {
            Some((a, c)) => (a, Some(c)),
            None => (left, None),
        };
        let hex16 = |t: &str| u16::from_str_radix(t, 16).map_err(|_| CheatError::BadRaw);
        let hex8 = |t: &str| u8::from_str_radix(t, 16).map_err(|_| CheatError::BadRaw);
        Ok(Cheat {
            code: original.trim().to_string(),
            addr: hex16(addr)?,
            value: hex8(value)?,
            compare: compare.map(hex8).transpose()?,
            enabled: true,
        })
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Cheats {
    pub list: Vec<Cheat>,
    active: bool,
}

impl Cheats {
    pub fn add(&mut self, cheat: Cheat) {
        self.list.push(cheat);
        self.refresh();
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.list.len() {
            self.list.remove(index);
        }
        self.refresh();
    }

    /// Call after toggling `enabled` on entries.
    pub fn refresh(&mut self) {
        self.active = self.list.iter().any(|c| c.enabled);
    }

    #[inline]
    pub fn apply(&self, addr: u16, value: u8) -> u8 {
        if !self.active {
            return value;
        }
        for c in &self.list {
            if c.enabled && c.addr == addr && c.compare.is_none_or(|cmp| cmp == value) {
                return c.value;
            }
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_game_genie() {
        // SXIOPO: Super Mario Bros. infinite lives, $91D9 = $AD.
        let c = Cheat::parse("SXIOPO").unwrap();
        assert_eq!((c.addr, c.value, c.compare), (0x91D9, 0xAD, None));
        let c = Cheat::parse("YEUZUGAA").unwrap();
        assert!(c.compare.is_some());
        assert!(Cheat::parse("SXIOP").is_err());
    }

    #[test]
    fn raw_codes_and_compare() {
        let mut cheats = Cheats::default();
        cheats.add(Cheat::parse("8123?67:45").unwrap());
        assert_eq!(cheats.apply(0x8123, 0x67), 0x45);
        assert_eq!(cheats.apply(0x8123, 0x68), 0x68);
    }
}
