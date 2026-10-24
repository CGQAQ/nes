//! Standard controllers on `$4016` / `$4017`.

use serde::{Deserialize, Serialize};

/// Button bits in the order the controller shifts them out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Buttons(pub u8);

impl Buttons {
    pub const A: u8 = 0x01;
    pub const B: u8 = 0x02;
    pub const SELECT: u8 = 0x04;
    pub const START: u8 = 0x08;
    pub const UP: u8 = 0x10;
    pub const DOWN: u8 = 0x20;
    pub const LEFT: u8 = 0x40;
    pub const RIGHT: u8 = 0x80;

    pub const NAMES: [(&'static str, u8); 8] = [
        ("A", Self::A),
        ("B", Self::B),
        ("Select", Self::SELECT),
        ("Start", Self::START),
        ("Up", Self::UP),
        ("Down", Self::DOWN),
        ("Left", Self::LEFT),
        ("Right", Self::RIGHT),
    ];

    pub fn set(&mut self, bit: u8, pressed: bool) {
        if pressed {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }

    pub fn pressed(self, bit: u8) -> bool {
        self.0 & bit != 0
    }

    /// A real D-pad can't press opposite directions; some games crash if it
    /// happens, so drop both.
    pub fn without_opposites(self) -> Self {
        let mut b = self.0;
        if b & (Self::UP | Self::DOWN) == Self::UP | Self::DOWN {
            b &= !(Self::UP | Self::DOWN);
        }
        if b & (Self::LEFT | Self::RIGHT) == Self::LEFT | Self::RIGHT {
            b &= !(Self::LEFT | Self::RIGHT);
        }
        Buttons(b)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Controller {
    pub buttons: Buttons,
    shift: u8,
    reads: u8,
}

impl Controller {
    fn latch(&mut self) {
        self.shift = self.buttons.0;
        self.reads = 0;
    }

    fn read(&mut self, strobe: bool) -> u8 {
        if strobe {
            return self.buttons.0 & 1;
        }
        // Official pads return 1 after all eight buttons have been read.
        let bit = if self.reads >= 8 { 1 } else { self.shift & 1 };
        self.shift >>= 1;
        self.reads = self.reads.saturating_add(1);
        bit
    }

    fn peek(&self, strobe: bool) -> u8 {
        if strobe {
            self.buttons.0 & 1
        } else if self.reads >= 8 {
            1
        } else {
            self.shift & 1
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Input {
    pub ports: [Controller; 2],
    strobe: bool,
}

impl Input {
    /// `$4016` write: bit 0 is the strobe line to both ports.
    pub fn write_strobe(&mut self, val: u8) {
        let strobe = val & 1 != 0;
        if self.strobe || strobe {
            self.ports[0].latch();
            self.ports[1].latch();
        }
        self.strobe = strobe;
    }

    /// Serial bit for `$4016` (port 0) or `$4017` (port 1), in bit 0. The
    /// caller merges in open bus for the undriven upper bits.
    pub fn read(&mut self, port: usize) -> u8 {
        let strobe = self.strobe;
        self.ports[port].read(strobe)
    }

    pub fn peek(&self, port: usize) -> u8 {
        self.ports[port].peek(self.strobe)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifts_out_buttons_in_order() {
        let mut input = Input::default();
        input.ports[0].buttons = Buttons(Buttons::A | Buttons::START | Buttons::RIGHT);
        input.write_strobe(1);
        input.write_strobe(0);
        let bits: Vec<u8> = (0..10).map(|_| input.read(0)).collect();
        assert_eq!(bits, [1, 0, 0, 1, 0, 0, 0, 1, 1, 1]);
    }
}
