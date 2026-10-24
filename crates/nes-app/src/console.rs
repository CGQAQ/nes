//! The console look: an NES-style body drawn around the picture, with a
//! working POWER and RESET, and a dark theme with the console's red as the
//! accent color.

use egui::emath::GuiRounding;
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Mesh, Painter, Pos2, Rect, Response, Sense,
    Shadow, Stroke, StrokeKind, Theme, Ui, Vec2, pos2, vec2,
};
use std::f32::consts::{FRAC_PI_2, PI};

/// The red of the console's labels, also the UI's accent color.
pub const RED: Color32 = Color32::from_rgb(200, 32, 44);

const BACKDROP_TOP: Color32 = Color32::from_gray(40);
const BACKDROP_BOTTOM: Color32 = Color32::from_gray(16);
const BODY_TOP: Color32 = Color32::from_rgb(216, 216, 211);
const BODY_BOTTOM: Color32 = Color32::from_rgb(188, 188, 183);
const BODY_EDGE: Color32 = Color32::from_rgb(108, 108, 104);
const GROOVE: Color32 = Color32::from_rgb(158, 158, 153);
const STRIPE: Color32 = Color32::from_gray(20);
const FRONT_TOP: Color32 = Color32::from_gray(72);
const FRONT_BOTTOM: Color32 = Color32::from_gray(46);
const BUTTON: Color32 = Color32::from_rgb(204, 204, 199);
const BUTTON_HOVER: Color32 = Color32::from_rgb(218, 218, 213);
const BUTTON_DOWN: Color32 = Color32::from_rgb(184, 184, 179);
const BUTTON_LIT: Color32 = Color32::from_rgb(236, 236, 231);
const BUTTON_SHADE: Color32 = Color32::from_rgb(120, 120, 116);
const LABEL: Color32 = Color32::from_rgb(214, 214, 210);

// Proportions, as fractions of the picture height.
/// Dark plastic around the picture.
const BEZEL: f32 = 0.04;
/// Body left and right of the bezel.
const SIDE: f32 = 0.05;
/// Ridged lid above the bezel.
const LID: f32 = 0.075;
/// Body between the bezel and the label stripe.
const GAP: f32 = 0.035;
const STRIPE_H: f32 = 0.07;
/// Front panel with the buttons and controller ports.
const FRONT_H: f32 = 0.17;
const HEIGHT: f32 = 1.0 + LID + 2.0 * BEZEL + GAP + STRIPE_H + FRONT_H;

/// Below this picture height the printed labels would be unreadable.
const MIN_LABELLED: f32 = 240.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Led {
    Off,
    /// Paused or stopped in the debugger.
    Dim,
    On,
}

/// What the front panel shows.
pub struct Front {
    pub led: Led,
    /// Buttons are held on controller 1 / 2.
    pub active: [bool; 2],
}

/// The front panel's controls.
pub struct Controls {
    pub power: Response,
    pub reset: Response,
    pub ports: [Response; 2],
}

/// Largest scale at which `size` fits in `room`. With `integer`, whole
/// numbers only once the scale reaches 1.
pub fn fit(room: Vec2, size: Vec2, integer: bool) -> f32 {
    let mut scale = (room.x / size.x).min(room.y / size.y).max(0.0);
    if integer && scale >= 1.0 {
        scale = scale.floor();
    }
    scale
}

/// Paints the console over all of `ui`, sized around a picture that is
/// `picture` points at 1x. Returns where the picture goes and the controls.
pub fn show(ui: &Ui, picture: Vec2, integer: bool, front: &Front) -> (Rect, Controls) {
    let area = ui.max_rect();
    let painter = ui.painter();
    painter.add(gradient(
        area,
        CornerRadius::ZERO,
        BACKDROP_TOP,
        BACKDROP_BOTTOM,
    ));

    let margin = (area.size().min_elem() * 0.04).clamp(6.0, 32.0);
    let unit = vec2(
        picture.x + 2.0 * (BEZEL + SIDE) * picture.y,
        HEIGHT * picture.y,
    );
    let room = (area.size() - Vec2::splat(2.0 * margin)).max(Vec2::ZERO);
    let scale = fit(room, unit, integer);
    // Every other size is a fraction of the picture height.
    let h = picture.y * scale;
    let px = |f: f32| f * h;
    let body = Rect::from_center_size(area.center(), unit * scale);
    let radius = corner(px(0.035));

    let bezel = Rect::from_min_size(
        pos2(body.left() + px(SIDE), body.top() + px(LID)),
        vec2(body.width() - 2.0 * px(SIDE), px(1.0 + 2.0 * BEZEL)),
    );
    let screen = bezel
        .shrink(px(BEZEL))
        .round_to_pixels(ui.pixels_per_point());
    let stripe = Rect::from_min_size(
        pos2(body.left(), bezel.bottom() + px(GAP)),
        vec2(body.width(), px(STRIPE_H)),
    );
    let panel = Rect::from_min_max(stripe.left_bottom(), body.max);
    // Panel contents line up with the picture's edges.
    let inset = px(SIDE + BEZEL);
    let labelled = h >= MIN_LABELLED;

    painter.add(
        Shadow {
            offset: [0, px(0.02).round().clamp(0.0, 100.0) as i8],
            blur: px(0.07).round().clamp(0.0, 255.0) as u8,
            spread: 0,
            color: Color32::from_black_alpha(170),
        }
        .as_shape(body, radius),
    );
    painter.add(gradient(body, radius, BODY_TOP, BODY_BOTTOM));

    // Grooves across the lid.
    let groove = px(0.005).max(1.0);
    let first = body.top() + px(0.02);
    let step = (bezel.top() - px(0.016) - first) / 3.0;
    for i in 0..4 {
        let y = first + step * i as f32;
        let (a, b) = (pos2(bezel.left(), y), pos2(bezel.right(), y));
        painter.line_segment([a, b], Stroke::new(groove, GROOVE));
        painter.line_segment(
            [a + vec2(0.0, groove), b + vec2(0.0, groove)],
            Stroke::new(1.0, Color32::from_white_alpha(120)),
        );
    }

    // The screen sits in a dark bezel set into the body.
    let lip = px(0.007).max(1.0);
    painter.add(gradient(
        bezel.expand(lip),
        corner(px(0.02) + lip),
        Color32::from_rgb(132, 132, 128),
        Color32::from_rgb(232, 232, 227),
    ));
    painter.add(gradient(
        bezel,
        corner(px(0.02)),
        Color32::from_gray(52),
        Color32::from_gray(28),
    ));
    painter.rect_stroke(
        bezel,
        corner(px(0.02)),
        Stroke::new(1.0, Color32::from_gray(18)),
        StrokeKind::Inside,
    );
    painter.rect_filled(screen, 0.0, Color32::BLACK);
    painter.rect_stroke(
        screen,
        0.0,
        Stroke::new(1.0, Color32::from_white_alpha(22)),
        StrokeKind::Outside,
    );

    // Label stripe.
    painter.rect_filled(stripe, 0.0, STRIPE);
    if labelled {
        let right = stripe.right() - inset;
        let cy = stripe.center().y;
        let name = painter.text(
            pos2(right, cy),
            Align2::RIGHT_CENTER,
            "ENTERTAINMENT SYSTEM",
            FontId::proportional(px(0.026)),
            LABEL,
        );
        let size = px(0.036);
        let text = painter.layout_no_wrap("nes".into(), FontId::proportional(size), Color32::WHITE);
        let pad = vec2(size * 0.4, size * 0.08);
        let badge = Rect::from_min_size(
            pos2(
                name.left() - px(0.016) - text.size().x - 2.0 * pad.x,
                cy - text.size().y / 2.0 - pad.y,
            ),
            text.size() + 2.0 * pad,
        );
        painter.rect_filled(badge, corner(px(0.008)), RED);
        bold(
            painter,
            badge.center(),
            Align2::CENTER_CENTER,
            "nes",
            size,
            Color32::WHITE,
        );
    }

    // Front panel.
    let panel_radius = CornerRadius {
        nw: 0,
        ne: 0,
        ..radius
    };
    painter.add(gradient(panel, panel_radius, FRONT_TOP, FRONT_BOTTOM));
    painter.line_segment(
        [panel.left_top(), panel.right_top()],
        Stroke::new(1.0, Color32::from_white_alpha(30)),
    );
    painter.rect_stroke(
        body,
        radius,
        Stroke::new(1.0, BODY_EDGE),
        StrokeKind::Outside,
    );

    let row = panel.top() + px(0.065);
    let size = vec2(px(0.11), px(0.04));
    let led_at = pos2(panel.left() + inset + px(0.012), row);
    let power_rect = Rect::from_center_size(pos2(led_at.x + px(0.04) + size.x / 2.0, row), size);
    let reset_rect = power_rect.translate(vec2(size.x + px(0.04), 0.0));
    draw_led(painter, led_at, px(0.011).max(2.0), front.led);
    let power = button(ui, power_rect, "power", h);
    let reset = button(ui, reset_rect, "reset", h);
    if labelled {
        for (rect, text) in [(power_rect, "POWER"), (reset_rect, "RESET")] {
            bold(
                painter,
                pos2(rect.center().x, rect.bottom() + px(0.03)),
                Align2::CENTER_CENTER,
                text,
                px(0.026),
                RED,
            );
        }
    }

    let port_size = vec2(px(0.13), px(0.062));
    let second = Rect::from_center_size(
        pos2(
            panel.right() - inset - port_size.x / 2.0,
            panel.top() + px(0.085),
        ),
        port_size,
    );
    let first = second.translate(vec2(-(port_size.x + px(0.06)), 0.0));
    let ports = [
        port(ui, first, 0, front.active[0], h, labelled),
        port(ui, second, 1, front.active[1], h, labelled),
    ];

    (
        screen,
        Controls {
            power,
            reset,
            ports,
        },
    )
}

fn corner(radius: f32) -> CornerRadius {
    CornerRadius::same(radius.round().clamp(0.0, 255.0) as u8)
}

/// A rounded rectangle shaded from `top` to `bottom`. Mesh edges aren't
/// anti-aliased, so outline it where the edge shows.
fn gradient(rect: Rect, radius: CornerRadius, top: Color32, bottom: Color32) -> Mesh {
    const STEPS: usize = 8;
    let shade = |p: Pos2| {
        let t = ((p.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
        top.lerp_to_gamma(bottom, t)
    };
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.center(), shade(rect.center()));
    // Corners clockwise from the top left, each an arc around its center.
    let corners = [
        (radius.nw, rect.left_top(), vec2(1.0, 1.0), PI),
        (radius.ne, rect.right_top(), vec2(-1.0, 1.0), 1.5 * PI),
        (radius.se, rect.right_bottom(), vec2(-1.0, -1.0), 0.0),
        (radius.sw, rect.left_bottom(), vec2(1.0, -1.0), 0.5 * PI),
    ];
    let max = rect.width().min(rect.height()) / 2.0;
    for (r, at, inward, start) in corners {
        let r = f32::from(r).min(max);
        let center = at + inward * r;
        for i in 0..=STEPS {
            let a = start + FRAC_PI_2 * i as f32 / STEPS as f32;
            let p = center + r * vec2(a.cos(), a.sin());
            mesh.colored_vertex(p, shade(p));
        }
    }
    let n = mesh.vertices.len() as u32;
    for i in 1..n {
        mesh.add_triangle(0, i, if i + 1 < n { i + 1 } else { 1 });
    }
    mesh
}

/// The default font has no bold face; draw it a few times, slightly apart.
fn bold(painter: &Painter, at: Pos2, align: Align2, text: &str, size: f32, color: Color32) {
    let font = FontId::proportional(size);
    let step = (size * 0.035).max(0.4);
    for dx in [-step, 0.0, step] {
        painter.text(at + vec2(dx, 0.0), align, text, font.clone(), color);
    }
}

fn draw_led(painter: &Painter, at: Pos2, r: f32, led: Led) {
    if led == Led::On {
        painter.circle_filled(
            at,
            r * 2.6,
            Color32::from_rgba_unmultiplied(255, 40, 40, 28),
        );
        painter.circle_filled(
            at,
            r * 1.8,
            Color32::from_rgba_unmultiplied(255, 40, 40, 56),
        );
    }
    let color = match led {
        Led::Off => Color32::from_rgb(70, 24, 26),
        Led::Dim => Color32::from_rgb(150, 30, 34),
        Led::On => Color32::from_rgb(255, 52, 48),
    };
    painter.circle_filled(at, r * 1.3, Color32::from_gray(22));
    painter.circle_filled(at, r, color);
    let shine = if led == Led::On { 170 } else { 40 };
    painter.circle_filled(
        at - vec2(r, r) * 0.3,
        r * 0.3,
        Color32::from_white_alpha(shine),
    );
}

/// A light gray push button in a dark slot.
fn button(ui: &Ui, rect: Rect, id: &str, h: f32) -> Response {
    let response = ui
        .interact(rect, ui.scope_id().with(id), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let painter = ui.painter();
    let down = response.is_pointer_button_down_on();
    painter.rect_filled(
        rect.expand((h * 0.006).max(1.0)),
        corner(h * 0.01),
        Color32::from_gray(24),
    );
    let face = if down {
        rect.translate(vec2(0.0, (h * 0.004).max(1.0)))
    } else {
        rect
    };
    let edge = (h * 0.005).max(1.0);
    let r = corner(h * 0.006);
    painter.rect_filled(face, r, BUTTON_SHADE);
    painter.rect_filled(face.with_max_y(face.max.y - edge), r, BUTTON_LIT);
    let fill = if down {
        BUTTON_DOWN
    } else if response.hovered() {
        BUTTON_HOVER
    } else {
        BUTTON
    };
    painter.rect_filled(
        Rect::from_min_max(face.min + vec2(0.0, edge * 0.6), face.max - vec2(0.0, edge)),
        r,
        fill,
    );
    response
}

/// A controller port: a seven-pin socket with one rounded end. Lights up
/// while that controller has buttons held.
fn port(ui: &Ui, rect: Rect, index: usize, active: bool, h: f32, labelled: bool) -> Response {
    let surround = rect.expand((h * 0.01).max(1.0));
    let response = ui
        .interact(
            surround,
            ui.scope_id().with(("port", index)),
            Sense::click(),
        )
        .on_hover_cursor(CursorIcon::PointingHand);
    let painter = ui.painter();
    let r = corner(h * 0.014);
    painter.rect_filled(surround, r, Color32::from_gray(22));
    if active || response.hovered() {
        let color = if active { RED } else { Color32::from_gray(120) };
        painter.rect_stroke(
            surround,
            r,
            Stroke::new((h * 0.004).max(1.0), color),
            StrokeKind::Outside,
        );
    }
    let round = corner(rect.height() * 0.5);
    let small = corner(h * 0.004);
    painter.rect_filled(
        rect,
        CornerRadius {
            nw: small.nw,
            sw: small.sw,
            ..round
        },
        Color32::from_gray(60),
    );
    let hole = (h * 0.0065).max(1.0);
    for (row, count) in [(0.32, 4), (0.68, 3)] {
        for i in 0..count {
            let x = rect.left() + rect.width() * (0.18 + 0.17 * i as f32);
            let y = rect.top() + rect.height() * row;
            painter.circle_filled(pos2(x, y), hole, Color32::from_gray(14));
        }
    }
    if labelled {
        painter.text(
            pos2(surround.left() - h * 0.014, rect.center().y),
            Align2::RIGHT_CENTER,
            (index + 1).to_string(),
            FontId::proportional(h * 0.03),
            LABEL.gamma_multiply(0.7),
        );
    }
    response
}

/// Dark theme with the console's red as the accent color.
pub fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(Theme::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
        let v = &mut style.visuals;
        v.panel_fill = Color32::from_gray(24);
        v.window_fill = Color32::from_gray(30);
        v.window_stroke = Stroke::new(1.0, Color32::from_gray(56));
        v.window_corner_radius = CornerRadius::same(8);
        v.extreme_bg_color = Color32::from_gray(12);
        v.faint_bg_color = Color32::from_gray(34);
        v.hyperlink_color = Color32::from_rgb(255, 128, 128);
        v.selection.bg_fill = Color32::from_rgb(150, 26, 36);
        v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(255, 222, 222));
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, RED);
        v.widgets.active.bg_fill = RED;
        v.widgets.active.weak_bg_fill = RED;
        v.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(255, 150, 150));
    });
}
