//! Keyboard and gamepad input mapped to NES controllers.

use crate::settings::{BUTTON_ORDER, Settings};
use nes_core::Buttons;

pub struct Input {
    gilrs: Option<gilrs::Gilrs>,
    pub gamepad_names: Vec<String>,
}

impl Input {
    pub fn new() -> Self {
        let gilrs = match gilrs::Gilrs::new() {
            Ok(g) => Some(g),
            Err(e) => {
                log::warn!("gamepads unavailable: {e}");
                None
            }
        };
        Self {
            gilrs,
            gamepad_names: Vec::new(),
        }
    }

    /// Current state of both controllers. `keyboard` is false while a text
    /// field has focus. `frame` drives turbo (on two frames, off two).
    pub fn poll(
        &mut self,
        ctx: &egui::Context,
        settings: &Settings,
        keyboard: bool,
        frame: u64,
    ) -> [Buttons; 2] {
        let mut pads = [Buttons::default(); 2];
        let turbo_on = frame & 2 == 0;

        if keyboard {
            ctx.input(|i| {
                for (port, pad) in pads.iter_mut().enumerate() {
                    for (slot, key) in settings.keys[port].0.iter().enumerate() {
                        if let Some(k) = key
                            && i.key_down(*k)
                        {
                            pad.set(BUTTON_ORDER[slot], true);
                        }
                    }
                    for (n, key) in settings.turbo_keys[port].iter().enumerate() {
                        if let Some(k) = key
                            && i.key_down(*k)
                            && turbo_on
                        {
                            pad.set(if n == 0 { Buttons::A } else { Buttons::B }, true);
                        }
                    }
                }
            });
        }

        if settings.gamepads_enabled
            && let Some(g) = self.gilrs.as_mut()
        {
            while g.next_event().is_some() {}
            self.gamepad_names.clear();
            for (port, (_, gp)) in g.gamepads().take(2).enumerate() {
                self.gamepad_names.push(gp.name().to_string());
                pads[port].0 |= gamepad_buttons(&gp, turbo_on).0;
            }
        }

        if !settings.allow_opposite_directions {
            for p in &mut pads {
                *p = p.without_opposites();
            }
        }
        pads
    }
}

fn gamepad_buttons(gp: &gilrs::Gamepad<'_>, turbo_on: bool) -> Buttons {
    use gilrs::{Axis, Button};
    let mut b = Buttons::default();
    // Positional mapping: NES B is the left face button, A the right one.
    b.set(Buttons::A, gp.is_pressed(Button::East));
    b.set(Buttons::B, gp.is_pressed(Button::South));
    if turbo_on {
        if gp.is_pressed(Button::North) {
            b.set(Buttons::A, true);
        }
        if gp.is_pressed(Button::West) {
            b.set(Buttons::B, true);
        }
    }
    b.set(Buttons::SELECT, gp.is_pressed(Button::Select));
    b.set(Buttons::START, gp.is_pressed(Button::Start));
    let x = gp.value(Axis::LeftStickX);
    let y = gp.value(Axis::LeftStickY);
    b.set(Buttons::UP, gp.is_pressed(Button::DPadUp) || y > 0.5);
    b.set(Buttons::DOWN, gp.is_pressed(Button::DPadDown) || y < -0.5);
    b.set(Buttons::LEFT, gp.is_pressed(Button::DPadLeft) || x < -0.5);
    b.set(Buttons::RIGHT, gp.is_pressed(Button::DPadRight) || x > 0.5);
    b
}
