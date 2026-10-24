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
    /// field has focus (see [`keyboard_free`]). `frame` drives turbo (on two frames, off two).
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

/// Whether the keyboard drives the emulator (controllers and hotkeys) this
/// frame: always, except while the user is typing into a text field. Other
/// widgets can hold keyboard focus too, but must not swallow the controls.
pub fn keyboard_free(ctx: &egui::Context) -> bool {
    !ctx.text_edit_focused()
}

/// Takes keyboard focus away from anything but a text field or an open menu.
/// Tab hands focus to buttons, and a focused button is clicked by Space and
/// Enter while the arrow keys move focus around, all while the same keys are
/// playing the game. Call before any widgets are shown.
pub fn release_widget_focus(ctx: &egui::Context) {
    if ctx.text_edit_focused() || egui::Popup::is_any_open(ctx) {
        return;
    }
    if let Some(id) = ctx.memory(|m| m.focused()) {
        ctx.memory_mut(|m| m.surrender_focus(id));
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect};

    fn key(key: Key, pressed: bool) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn pointer(pos: Pos2, pressed: bool) -> Vec<Event> {
        vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ]
    }

    /// A button, a text field and a drag value, run frame by frame the way
    /// the app does it.
    struct Harness {
        ctx: egui::Context,
        text: String,
        value: i32,
        clicks: u32,
        rects: [Rect; 3],
    }

    impl Harness {
        fn new() -> Self {
            let mut h = Self {
                ctx: egui::Context::default(),
                text: String::new(),
                value: 0,
                clicks: 0,
                rects: [Rect::NOTHING; 3],
            };
            h.frame(Vec::new());
            h
        }

        fn frame(&mut self, events: Vec<Event>) {
            let input = RawInput {
                events,
                ..Default::default()
            };
            let Self {
                ctx,
                text,
                value,
                clicks,
                rects,
            } = self;
            let mut output = ctx.run_ui(input, |ui| {
                release_widget_focus(ui.ctx());
                let button = ui.button("Pause");
                if button.clicked() {
                    *clicks += 1;
                }
                let field = ui.text_edit_singleline(text);
                let drag = ui.add(egui::DragValue::new(value));
                *rects = [button.rect, field.rect, drag.rect];
            });
            output.textures_delta.clear();
        }

        fn click(&mut self, widget: usize) {
            let pos = self.rects[widget].center();
            self.frame(pointer(pos, true));
            self.frame(pointer(pos, false));
        }
    }

    #[test]
    fn focused_button_does_not_take_the_keyboard() {
        let mut h = Harness::new();
        // Tab (fast-forward) hands focus to the first button...
        h.frame(vec![key(Key::Tab, true)]);
        assert!(h.ctx.egui_wants_keyboard_input());
        // ...which must not block the controls,
        assert!(keyboard_free(&h.ctx));
        // nor be clicked by Enter on the next frame.
        h.frame(vec![key(Key::Tab, false), key(Key::Enter, true)]);
        h.frame(vec![key(Key::Enter, false)]);
        assert_eq!(h.clicks, 0);
        assert!(!h.ctx.egui_wants_keyboard_input());
    }

    #[test]
    fn text_field_takes_the_keyboard() {
        let mut h = Harness::new();
        h.click(1);
        h.frame(Vec::new());
        assert!(!keyboard_free(&h.ctx));
        h.frame(vec![key(Key::Escape, true)]);
        assert!(keyboard_free(&h.ctx));
    }

    #[test]
    fn editing_a_drag_value_takes_the_keyboard() {
        let mut h = Harness::new();
        h.click(2);
        h.frame(Vec::new());
        assert!(!keyboard_free(&h.ctx));
    }
}
