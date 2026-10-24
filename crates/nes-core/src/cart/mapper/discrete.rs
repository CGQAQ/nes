//! Boards built from discrete logic: a latch or two driving the bank lines.
//! Each variant only differs in which writes it decodes and how the latched
//! value maps to banks, so they share one implementation.

use super::{Mapper, MapperInit};
use crate::cart::board::{Board, Mirroring};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    /// 2: UxROM. 16K switchable at $8000, last bank fixed.
    Uxrom,
    /// 3: CNROM. 8K CHR switch.
    Cnrom,
    /// 7: AxROM. 32K PRG switch, one-screen mirroring select.
    Axrom,
    /// 11: Color Dreams.
    ColorDreams,
    /// 13: CPROM. 4K CHR RAM switch at $1000.
    Cprom,
    /// 34 (CHR ROM <= 8K): BNROM. 32K PRG switch.
    Bnrom,
    /// 34 (CHR ROM > 8K): NINA-001. Registers at $7FFD-$7FFF.
    Nina001,
    /// 38: Bit Corp. Register at $7000-$7FFF.
    BitCorp38,
    /// 66: GxROM.
    Gxrom,
    /// 70: Bandai 74161.
    Bandai70,
    /// 71: Camerica / Codemasters.
    Camerica,
    /// 78: Irem 74HC161 / Jaleco JF-16.
    Irem78,
    /// 79: NINA-03/06. Register at $4100.
    Nina03,
    /// 87: Jaleco/Konami CHR switch at $6000.
    Jaleco87,
    /// 89: Sunsoft-2 (Tenka no Goikenban).
    Sunsoft89,
    /// 93: Sunsoft-2 (Shanghai/Fantasy Zone).
    Sunsoft93,
    /// 94: UN1ROM (Senjou no Ookami).
    Un1rom,
    /// 140: Jaleco JF-11/14. Register at $6000.
    Jaleco140,
    /// 152: Bandai 74161 with one-screen mirroring.
    Bandai152,
    /// 180: UNROM with the first bank fixed (Crazy Climber).
    Unrom180,
    /// 184: Sunsoft-1. 4K CHR banks at $6000.
    Sunsoft184,
    /// 185: CNROM with a copy-protection diode on CHR enable.
    Cnrom185,
}

impl Kind {
    pub fn from_mapper(mapper: u16, chr_rom_size: usize) -> Option<Kind> {
        Some(match mapper {
            2 => Kind::Uxrom,
            3 => Kind::Cnrom,
            7 => Kind::Axrom,
            11 => Kind::ColorDreams,
            13 => Kind::Cprom,
            34 if chr_rom_size > 0x2000 => Kind::Nina001,
            34 => Kind::Bnrom,
            38 => Kind::BitCorp38,
            66 => Kind::Gxrom,
            70 => Kind::Bandai70,
            71 => Kind::Camerica,
            78 => Kind::Irem78,
            79 | 113 | 146 => Kind::Nina03,
            87 => Kind::Jaleco87,
            89 => Kind::Sunsoft89,
            93 => Kind::Sunsoft93,
            94 => Kind::Un1rom,
            140 => Kind::Jaleco140,
            152 => Kind::Bandai152,
            180 => Kind::Unrom180,
            184 => Kind::Sunsoft184,
            185 => Kind::Cnrom185,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Uxrom => "UxROM",
            Kind::Cnrom => "CNROM",
            Kind::Axrom => "AxROM",
            Kind::ColorDreams => "Color Dreams",
            Kind::Cprom => "CPROM",
            Kind::Bnrom => "BNROM",
            Kind::Nina001 => "NINA-001",
            Kind::BitCorp38 => "Bit Corp 38",
            Kind::Gxrom => "GxROM",
            Kind::Bandai70 => "Bandai 74161",
            Kind::Camerica => "Camerica",
            Kind::Irem78 => "Irem 74HC161",
            Kind::Nina03 => "NINA-03/06",
            Kind::Jaleco87 => "Jaleco 87",
            Kind::Sunsoft89 => "Sunsoft-2 (89)",
            Kind::Sunsoft93 => "Sunsoft-2 (93)",
            Kind::Un1rom => "UN1ROM",
            Kind::Jaleco140 => "Jaleco JF-11/14",
            Kind::Bandai152 => "Bandai 74161 (1-screen)",
            Kind::Unrom180 => "UNROM (180)",
            Kind::Sunsoft184 => "Sunsoft-1",
            Kind::Cnrom185 => "CNROM (185)",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Discrete {
    board: Board,
    kind: Kind,
    submapper: u8,
    bus_conflicts: bool,
    latch: u8,
    chr_enabled: bool,
}

impl Discrete {
    pub fn new(init: MapperInit, kind: Kind) -> Self {
        let sub = init.info.submapper;
        // NES 2.0 submapper 2 means "has bus conflicts" for these boards;
        // Color Dreams, GxROM and CPROM boards always have them.
        let bus_conflicts = match kind {
            Kind::Uxrom | Kind::Cnrom | Kind::Axrom => sub == 2,
            Kind::ColorDreams | Kind::Gxrom | Kind::Cprom | Kind::Bnrom => sub != 1,
            _ => false,
        };
        let mut m = Self {
            board: init.board,
            kind,
            submapper: sub,
            bus_conflicts,
            latch: 0,
            chr_enabled: true,
        };
        match kind {
            Kind::Axrom
            | Kind::Bnrom
            | Kind::Nina001
            | Kind::ColorDreams
            | Kind::Gxrom
            | Kind::BitCorp38
            | Kind::Nina03
            | Kind::Jaleco140 => m.board.set_prg_rom_32k(0),
            Kind::Unrom180 => {
                m.board.set_prg_rom_16k(0x8000, 0);
                m.board.set_prg_rom_16k(0xC000, 0);
            }
            _ => {}
        }
        if kind == Kind::Axrom {
            m.board.set_mirroring(Mirroring::SingleScreenA);
        }
        m
    }

    fn apply(&mut self, addr: u16, v: u8) {
        let b = &mut self.board;
        match self.kind {
            Kind::Uxrom => b.set_prg_rom_16k(0x8000, v as i32),
            Kind::Cnrom => b.set_chr_8k(v as i32),
            Kind::Axrom => {
                b.set_prg_rom_32k((v & 0x0F) as i32);
                b.set_mirroring(if v & 0x10 != 0 {
                    Mirroring::SingleScreenB
                } else {
                    Mirroring::SingleScreenA
                });
            }
            Kind::ColorDreams => {
                b.set_prg_rom_32k((v & 0x03) as i32);
                b.set_chr_8k((v >> 4) as i32);
            }
            Kind::Cprom => b.set_chr_4k(4, (v & 0x03) as i32),
            Kind::Bnrom => b.set_prg_rom_32k(v as i32),
            Kind::Nina001 => match addr {
                0x7FFD => b.set_prg_rom_32k((v & 1) as i32),
                0x7FFE => b.set_chr_4k(0, (v & 0x0F) as i32),
                0x7FFF => b.set_chr_4k(4, (v & 0x0F) as i32),
                _ => {}
            },
            Kind::BitCorp38 => {
                b.set_prg_rom_32k((v & 0x03) as i32);
                b.set_chr_8k(((v >> 2) & 0x03) as i32);
            }
            Kind::Gxrom => {
                b.set_prg_rom_32k(((v >> 4) & 0x03) as i32);
                b.set_chr_8k((v & 0x03) as i32);
            }
            Kind::Bandai70 | Kind::Bandai152 => {
                b.set_prg_rom_16k(0x8000, ((v >> 4) & 0x07) as i32);
                b.set_chr_8k((v & 0x0F) as i32);
                if self.kind == Kind::Bandai152 {
                    b.set_mirroring(if v & 0x80 != 0 {
                        Mirroring::SingleScreenB
                    } else {
                        Mirroring::SingleScreenA
                    });
                }
            }
            Kind::Camerica => {
                if addr >= 0xC000 {
                    b.set_prg_rom_16k(0x8000, v as i32);
                } else if addr < 0xA000 && self.submapper == 1 {
                    // Fire Hawk: one-screen mirroring control.
                    b.set_mirroring(if v & 0x10 != 0 {
                        Mirroring::SingleScreenB
                    } else {
                        Mirroring::SingleScreenA
                    });
                }
            }
            Kind::Irem78 => {
                b.set_prg_rom_16k(0x8000, (v & 0x07) as i32);
                b.set_chr_8k((v >> 4) as i32);
                b.set_mirroring(if self.submapper == 3 {
                    if v & 0x08 != 0 {
                        Mirroring::Vertical
                    } else {
                        Mirroring::Horizontal
                    }
                } else if v & 0x08 != 0 {
                    Mirroring::SingleScreenB
                } else {
                    Mirroring::SingleScreenA
                });
            }
            Kind::Nina03 => {
                if addr & 0xE100 == 0x4100 {
                    b.set_prg_rom_32k(((v >> 3) & 1) as i32);
                    b.set_chr_8k((v & 0x07) as i32);
                }
            }
            Kind::Jaleco87 => b.set_chr_8k((((v & 1) << 1) | ((v >> 1) & 1)) as i32),
            Kind::Sunsoft89 => {
                b.set_prg_rom_16k(0x8000, ((v >> 4) & 0x07) as i32);
                b.set_chr_8k(((v & 0x07) | ((v & 0x80) >> 4)) as i32);
                b.set_mirroring(if v & 0x08 != 0 {
                    Mirroring::SingleScreenB
                } else {
                    Mirroring::SingleScreenA
                });
            }
            Kind::Sunsoft93 => {
                b.set_prg_rom_16k(0x8000, ((v >> 4) & 0x07) as i32);
                self.chr_enabled = v & 1 != 0;
            }
            Kind::Un1rom => b.set_prg_rom_16k(0x8000, ((v >> 2) & 0x07) as i32),
            Kind::Jaleco140 => {
                b.set_prg_rom_32k(((v >> 4) & 0x03) as i32);
                b.set_chr_8k((v & 0x0F) as i32);
            }
            Kind::Unrom180 => b.set_prg_rom_16k(0xC000, (v & 0x07) as i32),
            Kind::Sunsoft184 => {
                b.set_chr_4k(0, (v & 0x07) as i32);
                b.set_chr_4k(4, ((v >> 4) & 0x07) as i32);
            }
            Kind::Cnrom185 => {
                // The protection diode only lets specific values through.
                self.chr_enabled = match self.submapper {
                    4 => v & 3 == 0,
                    5 => v & 3 == 1,
                    6 => v & 3 == 2,
                    7 => v & 3 == 3,
                    _ => v & 0x0F != 0 && v != 0x13,
                };
            }
        }
    }

    fn decodes(&self, addr: u16) -> bool {
        match self.kind {
            Kind::Nina001 => (0x7FFD..=0x7FFF).contains(&addr),
            Kind::BitCorp38 => (0x7000..=0x7FFF).contains(&addr),
            Kind::Nina03 => (0x4100..0x6000).contains(&addr),
            Kind::Jaleco87 | Kind::Jaleco140 | Kind::Sunsoft184 => (0x6000..0x8000).contains(&addr),
            _ => addr >= 0x8000,
        }
    }
}

impl Mapper for Discrete {
    fn board(&self) -> &Board {
        &self.board
    }
    fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }
    fn name(&self) -> &'static str {
        self.kind.name()
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if self.decodes(addr) {
            let v = if self.bus_conflicts && addr >= 0x8000 {
                val & self.board.read_prg(addr).unwrap_or(0xFF)
            } else {
                val
            };
            self.latch = v;
            self.apply(addr, v);
        }
        // NINA-001's registers overlay PRG RAM, so RAM still sees the write.
        if addr < 0x8000 {
            self.board.write_prg(addr, val);
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.ppu_peek(addr)
    }

    fn ppu_peek(&self, addr: u16) -> u8 {
        if addr < 0x2000 && !self.chr_enabled {
            // Disabled CHR (protection or Sunsoft-2 CHR RAM gate) floats high.
            return 0xFF;
        }
        self.board.read_ppu(addr)
    }

    fn debug_state(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Latch", format!("${:02X}", self.latch)),
            ("Bus conflicts", self.bus_conflicts.to_string()),
            ("CHR enabled", self.chr_enabled.to_string()),
        ]
    }
}
