//! A loaded game: the emulator plus everything tied to it (pacing, rewind
//! history, battery-save bookkeeping).

use nes_core::cart::Mapper;
use nes_core::{Config, Nes, Region, RomError, StopReason};
use std::collections::VecDeque;

/// Snapshot interval for rewind, in frames.
const REWIND_INTERVAL: u64 = 2;

pub struct Session {
    pub nes: Nes,
    /// ROM file name, for display.
    pub name: String,
    /// Stable key for save files: `<stem>-<crc32>`.
    pub id: String,
    pub paused: bool,
    /// Why the debugger stopped execution, if it did.
    pub stopped: Option<String>,
    /// Wall-clock time owed to the emulator, in seconds.
    pub time_debt: f64,
    rewind: VecDeque<Vec<u8>>,
    rewind_capacity: usize,
    frames_run: u64,
    last_saved_ram: Option<Vec<u8>>,
}

impl Session {
    pub fn load(
        name: &str,
        data: &[u8],
        region: Option<Region>,
        sample_rate: f64,
    ) -> Result<Self, RomError> {
        let nes = Nes::new(
            data,
            Config {
                region,
                sample_rate,
            },
        )?;
        let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
        let id = format!("{stem}-{:08x}", nes.rom_info().crc32);
        Ok(Self {
            nes,
            name: name.to_string(),
            id,
            paused: false,
            stopped: None,
            time_debt: 0.0,
            rewind: VecDeque::new(),
            rewind_capacity: 0,
            frames_run: 0,
            last_saved_ram: None,
        })
    }

    pub fn mapper_name(&self) -> &'static str {
        self.nes.bus.cart.name()
    }

    pub fn frame_duration(&self) -> f64 {
        1.0 / self.nes.region().frame_rate()
    }

    pub fn running(&self) -> bool {
        !self.paused && self.stopped.is_none()
    }

    /// Runs one frame (or until the debugger stops it). Returns true if a
    /// complete frame was produced.
    pub fn run_frame(&mut self, rewind_seconds: Option<u32>) -> bool {
        match self.nes.run_frame() {
            StopReason::FrameComplete => {
                self.frames_run += 1;
                if let Some(secs) = rewind_seconds {
                    self.rewind_capacity = (secs as f64 * self.nes.region().frame_rate()
                        / REWIND_INTERVAL as f64)
                        as usize;
                    if self.frames_run.is_multiple_of(REWIND_INTERVAL) {
                        self.push_rewind();
                    }
                }
                true
            }
            StopReason::Break(reason) => {
                self.stopped = Some(reason.to_string());
                false
            }
            StopReason::StepDone => {
                self.stopped = Some("step complete".into());
                false
            }
        }
    }

    fn push_rewind(&mut self) {
        self.rewind.push_back(self.nes.save_state());
        while self.rewind.len() > self.rewind_capacity.max(1) {
            self.rewind.pop_front();
        }
    }

    /// Steps back one rewind snapshot and renders a frame from it. Returns
    /// false when history is exhausted.
    pub fn rewind_step(&mut self) -> bool {
        let Some(state) = self.rewind.pop_back() else {
            return false;
        };
        if self.nes.load_state(&state).is_err() {
            return false;
        }
        // States don't include the picture; run one frame to draw it.
        let _ = self.nes.run_frame();
        self.nes.bus.apu.clear_samples();
        true
    }

    pub fn rewind_len_seconds(&self) -> f64 {
        self.rewind.len() as f64 * REWIND_INTERVAL as f64 / self.nes.region().frame_rate()
    }

    pub fn clear_rewind(&mut self) {
        self.rewind.clear();
    }

    pub fn save_key(&self) -> String {
        format!("saves/{}.sav", self.id)
    }

    pub fn state_key(&self, slot: u8) -> String {
        format!("states/{}-{slot}.state", self.id)
    }

    /// Battery RAM if it changed since the last call.
    pub fn dirty_save_data(&mut self) -> Option<Vec<u8>> {
        let data = self.nes.save_data()?;
        if self.last_saved_ram.as_ref() == Some(&data) {
            return None;
        }
        self.last_saved_ram = Some(data.clone());
        Some(data)
    }

    pub fn restore_save_data(&mut self, data: &[u8]) {
        self.nes.load_save_data(data);
        self.last_saved_ram = self.nes.save_data();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An NROM image that just spins in a loop.
    fn spin_rom() -> Vec<u8> {
        let mut rom = vec![b'N', b'E', b'S', 0x1A, 1, 1, 0, 0];
        rom.resize(16, 0);
        let mut prg = vec![0xEA; 0x4000];
        prg[0] = 0x4C; // JMP $C000
        prg[1] = 0x00;
        prg[2] = 0xC0;
        prg[0x3FFC] = 0x00;
        prg[0x3FFD] = 0xC0;
        rom.extend(prg);
        rom.extend(vec![0; 0x2000]);
        rom
    }

    #[test]
    fn rewind_goes_back_in_time() {
        let mut s = Session::load("spin.nes", &spin_rom(), None, 48_000.0).unwrap();
        for _ in 0..120 {
            assert!(s.run_frame(Some(30)));
        }
        let before = s.nes.frame();
        assert!(s.rewind_len_seconds() > 1.9);
        for _ in 0..20 {
            assert!(s.rewind_step());
        }
        assert!(s.nes.frame() < before - 20, "{} vs {before}", s.nes.frame());
    }

    #[test]
    fn rewind_history_is_bounded() {
        let mut s = Session::load("spin.nes", &spin_rom(), None, 48_000.0).unwrap();
        for _ in 0..600 {
            s.run_frame(Some(2));
        }
        assert!(s.rewind_len_seconds() <= 2.1, "{}", s.rewind_len_seconds());
    }

    #[test]
    fn save_ids_are_stable() {
        let s = Session::load("Some Game (U).nes", &spin_rom(), None, 48_000.0).unwrap();
        assert!(s.id.starts_with("Some Game (U)-"));
        assert_eq!(s.state_key(3), format!("states/{}-3.state", s.id));
    }
}
