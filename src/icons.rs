//! Per-aircraft-category vector icons for the sixel scope, each a small
//! polygon (or a couple of them) in a local "nose up" unit space — heading
//! 0 points straight up (north on screen), and rotates clockwise from
//! there, matching compass bearing directly. Sixel-only: braille mode has
//! no equivalent (its blips are plain terminal-font characters, which
//! can't rotate), so it keeps the plain dot it's always had.

use crate::data::aircraft::AircraftKind;
use ratatui::style::Color;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Shader, Stroke, Transform};

/// Base "reach" of an icon from center to its farthest point, in pixels.
/// Per-kind draw functions scale from this rather than hardcoding pixel
/// sizes, so every icon stays proportional if this is retuned later.
pub const ICON_SIZE_PX: f32 = 8.0;

fn to_skia(color: Color, alpha: u8) -> tiny_skia::Color {
    let (r, g, b) = match color {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (255, 255, 255),
    };
    tiny_skia::Color::from_rgba8(r, g, b, alpha)
}

fn solid_paint(color: tiny_skia::Color) -> Paint<'static> {
    Paint {
        shader: Shader::SolidColor(color),
        anti_alias: true,
        ..Default::default()
    }
}

/// Rotate-scale-translate from an icon's local "nose up" unit space to
/// pixmap coordinates, bundled so draw calls don't each need five separate
/// float arguments.
#[derive(Clone, Copy)]
struct IconTransform {
    cos_a: f32,
    sin_a: f32,
    scale: f32,
    cx: f32,
    cy: f32,
}

impl IconTransform {
    fn new(heading_deg: f64, scale: f32, cx: f32, cy: f32) -> Self {
        let rad = heading_deg.to_radians();
        Self {
            cos_a: rad.cos() as f32,
            sin_a: rad.sin() as f32,
            scale,
            cx,
            cy,
        }
    }

    fn scaled(&self, scale: f32) -> Self {
        Self { scale, ..*self }
    }

    fn apply(&self, (x, y): (f32, f32)) -> (f32, f32) {
        // Clockwise rotation in a y-down (screen) coordinate system — the
        // same formula as counterclockwise rotation in ordinary math's
        // y-up convention, which is exactly what makes it line up with
        // compass bearing (0 = north/up, increasing clockwise) with no
        // offset needed, unlike a pre-drawn glyph whose own rest pose you
        // don't control.
        let rx = x * self.cos_a - y * self.sin_a;
        let ry = x * self.sin_a + y * self.cos_a;
        (self.cx + rx * self.scale, self.cy + ry * self.scale)
    }
}

fn fill_dot(pixmap: &mut Pixmap, x: f32, y: f32, r: f32, color: tiny_skia::Color) {
    let Some(path) = PathBuilder::from_circle(x, y, r) else {
        return;
    };
    let paint = solid_paint(color);
    pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn filled_polygon(
    pixmap: &mut Pixmap,
    points: &[(f32, f32)],
    xf: &IconTransform,
    color: tiny_skia::Color,
) {
    let mut pb = PathBuilder::new();
    let Some((&first, rest)) = points.split_first() else {
        return;
    };
    let (fx, fy) = xf.apply(first);
    pb.move_to(fx, fy);
    for &p in rest {
        let (px, py) = xf.apply(p);
        pb.line_to(px, py);
    }
    pb.close();
    let Some(path) = pb.finish() else { return };
    let paint = solid_paint(color);
    pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn stroked_segment(
    pixmap: &mut Pixmap,
    a: (f32, f32),
    b: (f32, f32),
    xf: &IconTransform,
    width: f32,
    color: tiny_skia::Color,
) {
    let (ax, ay) = xf.apply(a);
    let (bx, by) = xf.apply(b);
    let mut pb = PathBuilder::new();
    pb.move_to(ax, ay);
    pb.line_to(bx, by);
    let Some(path) = pb.finish() else { return };
    let paint = solid_paint(color);
    let stroke = Stroke {
        width,
        ..Default::default()
    };
    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

// Dart/delta silhouette shared by Airliner and BusinessJet (the latter just
// scaled down, plus a small T-tail bar) — nose, swept wings, small tail
// flare. Point order matters: this is traced as one closed polygon.
const JET_POINTS: &[(f32, f32)] = &[
    (0.0, -1.0),   // nose
    (0.15, -0.1),  // right wing root, leading edge
    (0.95, 0.55),  // right wingtip
    (0.15, 0.35),  // right wing root, trailing edge
    (0.25, 1.0),   // tail, right
    (0.0, 0.8),    // tail, center notch
    (-0.25, 1.0),  // tail, left
    (-0.15, 0.35), // left wing root, trailing edge
    (-0.95, 0.55), // left wingtip
    (-0.15, -0.1), // left wing root, leading edge
];

// Wider wingtips and a blunter nose than the jet dart — reads as a
// turboprop's higher aspect ratio at a glance. Paired with two small
// engine dots drawn separately (see draw_icon).
//
// Root width (the two "wing root" x-values below) is deliberately much
// wider than the jet dart's 0.15 — verified this is a *simple*
// (non-self-intersecting) polygon at any width, but a thin root still
// reads badly once actually rendered and rotated off the vertical: any
// concave 4-pointed dart (nose/wing/wing/tail) leans on the viewer
// reading "nose up" to parse as an airplane at all, and a thin pinched
// waist makes that reading collapse into "abstract star" the moment it's
// not nose-up anymore. This isn't a bug so much as a real limit of a thin
// dart silhouette carrying rotation — a wide, chunky root is what keeps
// the shape reading as one solid body rather than four disconnected
// points at an arbitrary heading. Same tradeoff applies to every icon
// here to some degree; this one just needed it most.
const REGIONAL_POINTS: &[(f32, f32)] = &[
    (0.0, -0.7),   // nose, blunter than the jet's
    (0.32, -0.05), // right wing root, leading edge
    (1.05, 0.35),  // right wingtip — wider than the jet, aft of the trailing edge
    (0.32, 0.25),  // right wing root, trailing edge
    (0.28, 0.9),   // tail, right
    (0.0, 0.78),   // tail, center notch
    (-0.28, 0.9),  // tail, left
    (-0.32, 0.25), // left wing root, trailing edge
    (-1.05, 0.35), // left wingtip
    (-0.32, -0.05), // left wing root, leading edge
];

pub fn draw_icon(
    pixmap: &mut Pixmap,
    cx: f32,
    cy: f32,
    heading_deg: f64,
    kind: AircraftKind,
    color: Color,
    alpha: u8,
) {
    let xf = IconTransform::new(heading_deg, ICON_SIZE_PX, cx, cy);
    let skia_color = to_skia(color, alpha);

    match kind {
        AircraftKind::Airliner => {
            filled_polygon(pixmap, JET_POINTS, &xf, skia_color);
        }
        AircraftKind::BusinessJet => {
            let xf = xf.scaled(ICON_SIZE_PX * 0.65);
            filled_polygon(pixmap, JET_POINTS, &xf, skia_color);
            // Small T-tail bar — the one detail that keeps this from
            // reading as just "a smaller airliner" at a glance.
            // Right at the tail tip and wider than the fuselage's own tail
            // flare (±0.25 there) so it actually pokes out and reads as a
            // distinct horizontal stabilizer, rather than sitting on top of
            // the tail notch and disappearing into it.
            stroked_segment(pixmap, (-0.45, 0.97), (0.45, 0.97), &xf, 1.3, skia_color);
        }
        AircraftKind::Regional => {
            filled_polygon(pixmap, REGIONAL_POINTS, &xf, skia_color);
            // Prop-engine marks, roughly mid-wing.
            for &p in &[(0.55, 0.08), (-0.55, 0.08)] {
                let (px, py) = xf.apply(p);
                fill_dot(pixmap, px, py, 0.9, skia_color);
            }
        }
        AircraftKind::Private => {
            let xf = xf.scaled(ICON_SIZE_PX * 0.9);
            // Classic small-plane cross: fuselage, one wing bar, one tail
            // bar — three strokes read faster at this size than a filled
            // outline would.
            stroked_segment(pixmap, (0.0, -1.0), (0.0, 0.9), &xf, 1.4, skia_color);
            stroked_segment(pixmap, (-0.9, -0.05), (0.9, -0.05), &xf, 1.6, skia_color);
            stroked_segment(pixmap, (-0.3, 0.85), (0.3, 0.85), &xf, 1.2, skia_color);
        }
        AircraftKind::Helicopter => {
            // Rotor disc: rotationally symmetric, so drawn as a plain
            // stroked circle rather than transformed points — rotating it
            // to heading would be a no-op anyway.
            let rotor_r = ICON_SIZE_PX * 0.85;
            if let Some(path) = PathBuilder::from_circle(cx, cy, rotor_r) {
                let dim_alpha = (u16::from(alpha) * 2 / 3) as u8;
                let paint = solid_paint(to_skia(color, dim_alpha));
                let stroke = Stroke {
                    width: 0.8,
                    ..Default::default()
                };
                pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            }
            // Tail boom, rotates with heading like every other icon.
            let xf = xf.scaled(rotor_r);
            stroked_segment(pixmap, (0.0, 0.0), (0.0, 0.9), &xf, 1.2, skia_color);
            // Fuselage.
            fill_dot(pixmap, cx, cy, ICON_SIZE_PX * 0.28, skia_color);
        }
        AircraftKind::Unknown => {
            // No dedicated silhouette — the plain glow-halo-plus-dot blip
            // this app has always used for every contact.
            fill_dot(pixmap, cx, cy, 4.5, to_skia(color, 45));
            fill_dot(pixmap, cx, cy, 1.4, skia_color);
        }
    }
}
