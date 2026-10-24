//! Cycle-accurate NES emulation core.
//!
//! The core is deterministic and has no I/O, windowing or audio dependencies:
//! frontends feed it ROM bytes and controller state, and pull out frames,
//! audio samples and debugger state.
//!
//! ```no_run
//! use nes_core::{Nes, Config, Buttons, StopReason};
//! let rom = std::fs::read("game.nes").unwrap();
//! let mut nes = Nes::new(&rom, Config::default()).unwrap();
//! nes.set_buttons(0, Buttons(Buttons::START));
//! while nes.run_frame() != StopReason::FrameComplete {}
//! let pixels = nes.framebuffer(); // 256x240, see `ppu::palette::Palette`
//! # let _ = pixels;
//! ```

pub mod apu;
pub mod bus;
pub mod cart;
pub mod cheats;
pub mod cpu;
pub mod debug;
pub mod input;
mod nes;
pub mod ppu;
pub mod region;
mod state;
pub mod testing;

pub use cart::{RomError, RomInfo};
pub use input::Buttons;
pub use nes::{Config, Nes, StopReason};
pub use ppu::palette::Palette;
pub use region::Region;
