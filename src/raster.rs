use crate::data::aircraft::{Aircraft, Altitude};
use crate::data::airports::RunwaySegment;
use crate::geometry::{self, bearing_to_xy};
use crate::theme::Palette;
use crate::trail::TrailStore;
use image::RgbaImage;
use ratatui::style::Color;
use tiny_skia::{
    FillRule, Paint, PathBuilder, Pixmap, PremultipliedColorU8, Shader, Stroke, Transform,
};

const RING_COUNT: u32 = 4;
const LABEL_ROWS: usize = 3;

/// The 8 directions a contact label can be anchored in relative to its
/// blip, as (horizontal, vertical) signs: +1/-1 means the label extends
/// that way from the blip (which then sits at that edge, offset by the
/// search's padding); 0 means the label is centered on the blip along
/// that axis instead (used for the 4 cardinal directions — e.g. "below"
/// centers horizontally and only offsets vertically). See the label
/// placement search in `draw_contacts` for how these turn into an actual
/// candidate box.
const DIRECTIONS: [(i8, i8); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];

/// Sixel/raster mode rasterizes its own label glyphs rather than relying on
/// the terminal's font, so it has to pick a size — this scales the detected
/// cell height. Braille mode has no equivalent knob: its labels are plain
/// terminal text, always at the terminal's own native size, so this is the
/// only lever for bringing the two modes' apparent text size in line with
/// each other.
///
/// History: 0.85 (an earlier bump from a smaller constant) still read
/// noticeably smaller than braille. 1.1 overshot the other way — sixel
/// read bigger than braille rather than matching it. Braille's own size is
/// fine as the reference point, just not reproducible via a "cell height
/// in px" calculation, since most monospace fonts' actual glyph height is
/// well under the full em-box that implies — so this stays a tunable
/// scale rather than a fixed ratio derived from font metrics.
pub const LABEL_FONT_SCALE: f32 = 0.7;

/// Hard ceiling on the derived label size, regardless of how big the
/// terminal's own cell height is. `omarchy-launch-screensaver` runs the
/// screensaver terminal at an explicit 18pt font (every supported
/// terminal — Alacritty/foot/ghostty/kitty — all hardcode `font-size=18`),
/// deliberately oversized so the stock `ttfx` branding text reads from
/// across a room. That's the right call for that content, but "match the
/// terminal's native size" is the wrong instinct for *this* app's dense
/// data labels once the terminal's own font is that large — without a
/// cap, the screensaver's labels scale up right along with it. Chosen to
/// comfortably cover an ordinary terminal's font size while meaningfully
/// reining in an 18pt one.
pub const LABEL_FONT_MAX_PX: f32 = 16.0;

/// The actual per-call-site computation — `cell_height_px * LABEL_FONT_SCALE`,
/// capped at `LABEL_FONT_MAX_PX`. Centralized so every caller applies both
/// the scale and the cap the same way rather than reimplementing the
/// `.min()` themselves.
pub fn label_font_px(cell_height_px: f32) -> f32 {
    (cell_height_px * LABEL_FONT_SCALE).min(LABEL_FONT_MAX_PX)
}

pub struct Scene<'a> {
    pub width_px: u32,
    pub height_px: u32,
    pub aircraft: &'a [Aircraft],
    pub trails: &'a TrailStore,
    /// Sixel-only backdrop layer (see draw_runways) — braille mode has no
    /// equivalent, by explicit choice, not an oversight.
    pub runways: &'a [RunwaySegment],
    pub zoom_radius_nm: f64,
    pub sweep_angle_deg: f64,
    pub palette: &'a Palette,
    pub font: &'a fontdue::Font,
    /// Label text size in px, derived from the terminal's actual detected
    /// cell height so labels read at the same size as the surrounding
    /// terminal/bar text instead of an arbitrary guessed constant.
    pub label_font_px: f32,
    /// Screensaver mode: skip the callsign/altitude/speed label entirely
    /// for every contact — icons and trails still draw. See scope::render
    /// for why.
    pub hide_labels: bool,
}

pub fn render(scene: &Scene) -> RgbaImage {
    let mut pixmap =
        Pixmap::new(scene.width_px.max(1), scene.height_px.max(1)).expect("nonzero pixmap size");
    pixmap.fill(to_skia(scene.palette.background, 255));

    let cx = scene.width_px as f32 / 2.0;
    let cy = scene.height_px as f32 / 2.0;
    // Pixels are square, unlike terminal cells, so no aspect correction is
    // needed here for the rings to actually look circular.
    let radius_px = (scene.width_px.min(scene.height_px) as f32 / 2.0) * 0.94;
    let px_per_nm = radius_px / scene.zoom_radius_nm.max(0.001) as f32;

    let to_px = |nm_x: f64, nm_y: f64| -> (f32, f32) {
        (
            cx + nm_x as f32 * px_per_nm,
            cy - nm_y as f32 * px_per_nm, // screen y grows downward; nm y is north-up
        )
    };

    draw_rings(&mut pixmap, cx, cy, radius_px, scene.palette.muted);
    draw_runways(&mut pixmap, scene, &to_px, scene.palette.accent);
    draw_sweep(
        &mut pixmap,
        cx,
        cy,
        radius_px,
        scene.sweep_angle_deg,
        scene.palette.accent,
    );
    draw_trails(&mut pixmap, scene, &to_px);
    draw_contacts(&mut pixmap, scene, &to_px, px_per_nm);

    pixmap_to_image(pixmap)
}

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

fn fill_circle(pixmap: &mut Pixmap, x: f32, y: f32, r: f32, color: tiny_skia::Color) {
    let Some(path) = PathBuilder::from_circle(x, y, r) else {
        return;
    };
    pixmap.fill_path(
        &path,
        &solid_paint(color),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn draw_rings(pixmap: &mut Pixmap, cx: f32, cy: f32, radius_px: f32, color: Color) {
    let paint = solid_paint(to_skia(color, 140));
    let stroke = Stroke {
        width: 1.0,
        ..Default::default()
    };
    for i in 1..=RING_COUNT {
        let r = radius_px * f32::from(i as u16) / f32::from(RING_COUNT as u16);
        if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }
}

/// Static backdrop layer: real nearby runways, oriented and sized from
/// OurAirports data (see `data::airports`), drawn dim so they read as
/// terrain/infrastructure rather than competing with live traffic. Half
/// brightness is done the same way the rest of this renderer dims things —
/// alpha over the dark background — rather than halving the color's own
/// RGB channels.
///
/// `data::airports::load_nearby` loads everything within a fixed 120nm
/// (independent of the live zoom level, which the loader has no access to),
/// so at typical zoom levels this can carry plenty of airports the current
/// view doesn't need — per feedback that this reads as clutter, brightness
/// now fades linearly from `RUNWAY_MAX_ALPHA` at the center to fully
/// transparent at the outer ring, and anything beyond the outer ring
/// (`zoom_radius_nm`, not the fixed load radius) is skipped outright rather
/// than drawn at 0 alpha, so panning/zooming never pays to rasterize
/// off-screen geometry.
const RUNWAY_MAX_ALPHA: f32 = 140.0;

fn draw_runways(
    pixmap: &mut Pixmap,
    scene: &Scene,
    to_px: &dyn Fn(f64, f64) -> (f32, f32),
    color: Color,
) {
    let stroke = Stroke {
        width: 2.0,
        ..Default::default()
    };
    for runway in scene.runways {
        let dst = (runway.dst_a + runway.dst_b) / 2.0;
        if dst > scene.zoom_radius_nm {
            continue;
        }
        let fade = (1.0 - dst / scene.zoom_radius_nm.max(0.001)) as f32;
        let alpha = (RUNWAY_MAX_ALPHA * fade).round() as u8;
        if alpha == 0 {
            continue;
        }
        let paint = solid_paint(to_skia(color, alpha));

        let (xa, ya) = bearing_to_xy(runway.dst_a, runway.dir_a);
        let (xb, yb) = bearing_to_xy(runway.dst_b, runway.dir_b);
        let (xa, ya) = to_px(xa, ya);
        let (xb, yb) = to_px(xb, yb);
        let mut pb = PathBuilder::new();
        pb.move_to(xa, ya);
        pb.line_to(xb, yb);
        if let Some(path) = pb.finish() {
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }
}

fn draw_sweep(pixmap: &mut Pixmap, cx: f32, cy: f32, radius_px: f32, angle_deg: f64, color: Color) {
    let point_at = |a: f64| -> (f32, f32) {
        let rad = a.to_radians();
        (cx + radius_px * rad.sin() as f32, cy - radius_px * rad.cos() as f32)
    };

    // Fading trail behind the beam: a fan of filled triangles (center plus
    // two points on the arc), each a shade dimmer than the last, rather
    // than stroked radial lines. Strokes were the original approach here,
    // but a fixed-width stroke only touches its neighbor at a fixed radius
    // — past that, the arc-length between adjacent strokes outgrows the
    // stroke width and leaves a visible gap, which is exactly what showed
    // up once the window (and so the radius in pixels) was large enough.
    // Filled triangles sharing an edge tile with no gap at any radius, so
    // this is gap-free at any window size rather than needing a step count
    // tuned to it (see geometry::SWEEP_TRAIL_STEPS).
    let steps = geometry::SWEEP_TRAIL_STEPS;
    let step_deg = geometry::SWEEP_TRAIL_SPAN_DEG / f64::from(steps);
    for step in 0..steps {
        let t0 = 1.0 - f64::from(step) / f64::from(steps);
        let t1 = 1.0 - f64::from(step + 1) / f64::from(steps);
        // A hair of angular overlap on the trailing edge so tiny-skia's
        // per-shape antialiasing can't leave a faint seam where two
        // adjacently-filled, differently-alpha'd triangles meet.
        let overlap_deg = step_deg * 0.15;
        let (x0, y0) = point_at(angle_deg - f64::from(step) * step_deg);
        let (x1, y1) = point_at(angle_deg - f64::from(step + 1) * step_deg - overlap_deg);
        let mut pb = PathBuilder::new();
        pb.move_to(cx, cy);
        pb.line_to(x0, y0);
        pb.line_to(x1, y1);
        pb.close();
        let Some(path) = pb.finish() else { continue };
        // Midpoint alpha of the segment's two edges, so it reads as one
        // continuous gradient step rather than a hard-edged band.
        let alpha = (((t0 + t1) / 2.0) * 140.0) as u8;
        let paint = solid_paint(to_skia(color, alpha));
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    let (ex, ey) = point_at(angle_deg);

    // Layered strokes, wide+dim to narrow+bright, simulate a phosphor glow.
    const LAYERS: [(f32, u8); 3] = [(8.0, 30), (3.5, 90), (1.2, 220)];
    for (width, alpha) in LAYERS {
        let mut pb = PathBuilder::new();
        pb.move_to(cx, cy);
        pb.line_to(ex, ey);
        let Some(path) = pb.finish() else { continue };
        let paint = solid_paint(to_skia(color, alpha));
        let stroke = Stroke {
            width,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

fn draw_trails(pixmap: &mut Pixmap, scene: &Scene, to_px: &dyn Fn(f64, f64) -> (f32, f32)) {
    for ac in scene.aircraft {
        let Some(trail) = scene.trails.get(&ac.hex) else {
            continue;
        };
        let base = if ac.is_emergency_squawk() {
            scene.palette.alert
        } else {
            scene.palette.kind_color(ac.kind())
        };
        let len = trail.len();
        for (i, (tx, ty)) in trail.iter().enumerate() {
            let t = (i + 1) as f64 / (len + 1) as f64;
            let (px, py) = to_px(*tx, *ty);
            let alpha = (30.0 + t * 140.0) as u8;
            fill_circle(pixmap, px, py, 1.6, to_skia(base, alpha));
        }
    }
}

struct LabelBox {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

fn boxes_overlap(a: &LabelBox, b: &LabelBox) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

fn draw_contacts(
    pixmap: &mut Pixmap,
    scene: &Scene,
    to_px: &dyn Fn(f64, f64) -> (f32, f32),
    px_per_nm: f32,
) {
    let mut contacts: Vec<&Aircraft> = scene
        .aircraft
        .iter()
        .filter(|ac| matches!(ac.dst, Some(d) if d <= scene.zoom_radius_nm))
        .collect();
    // Closest-first, not the old arbitrary hex order: under real crowding
    // (an airport's worth of ground traffic, a cluster of helicopters) not
    // every contact can win a non-overlapping label spot, and the ones
    // that lose should be the distant/less relevant ones, not whoever
    // happened to sort first by tail number.
    contacts.sort_by(|a, b| {
        a.dst
            .partial_cmp(&b.dst)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut placed: Vec<LabelBox> = Vec::with_capacity(contacts.len());
    // Padding has to clear the icon's own drawn size, not just its center
    // point — icons.rs draws well past the bare (px, py) coordinate this
    // function otherwise treats as a zero-size point, so without this a
    // small gap still visually touches the icon even though it's
    // correctly offset from the *position*.
    let gap = (px_per_nm * 0.9).max(6.0) + crate::icons::ICON_SIZE_PX;

    for ac in &contacts {
        let (Some(dst), Some(dir)) = (ac.dst, ac.dir) else {
            continue;
        };
        let (nx, ny) = bearing_to_xy(dst, dir);
        let (px, py) = to_px(nx, ny);
        let base = if ac.is_emergency_squawk() {
            scene.palette.alert
        } else {
            scene.palette.foreground
        };

        crate::icons::draw_icon(
            pixmap,
            px,
            py,
            ac.track.unwrap_or(0.0),
            ac.kind(),
            base,
            255,
        );

        // Ground traffic (an airport's ramp/taxiways) is the single
        // densest source of clutter this app sees — easily dozens of
        // contacts in a tiny physical radius, which no amount of smarter
        // label search fixes. Blip only, no label, for anything parked or
        // taxiing.
        if matches!(ac.alt_baro, Some(Altitude::Ground)) {
            continue;
        }
        if scene.hide_labels {
            continue;
        }

        let climb_rate = ac.climb_rate();
        let climb_glyph = match climb_rate {
            Some(r) if r > 100.0 => " ▲",
            Some(r) if r < -100.0 => " ▼",
            _ => "",
        };
        let alt_base = match ac.alt_baro {
            Some(Altitude::Feet(ft)) => format!("FL{:03}", ft / 100),
            Some(Altitude::Ground) => "GND".to_string(),
            None => "?".to_string(),
        };
        // For width measurement and collision boxing, the full string
        // (glyph included) — the actual draw below splits it back apart
        // so the glyph can carry its own climb/descent color.
        let lines = [
            ac.callsign().to_string(),
            format!("{alt_base}{climb_glyph}"),
            format!("{:.0}kt", ac.gs.unwrap_or(0.0)),
        ];
        let callsign_color = if ac.is_emergency_squawk() {
            base
        } else {
            scene.palette.kind_color(ac.kind())
        };
        let climb_color = if ac.is_emergency_squawk() {
            None
        } else {
            scene.palette.climb_rate_color(climb_rate)
        };

        let (width_px, line_h) = measure(scene.font, scene.label_font_px, &lines);
        let height_px = line_h * LABEL_ROWS as f32;

        // Corner-anchored search: 8 directions around the blip, each
        // placing the label so the blip sits at that direction's near
        // corner (or edge midpoint, for the 4 cardinal directions) of the
        // label — never the label's center. This is what all 4 of the
        // original fixed candidates already did (icon at a corner, never
        // floating in open space away from it); this just extends that to
        // all 8 directions and adds increasing padding rings on top, so a
        // label still has a real search to fall back on in a crowded area
        // without losing the "icon anchors a corner" relationship that
        // makes the pairing readable at a glance. Ring-major order (try
        // every direction at the tightest padding before accepting more
        // padding in any direction), matching the original candidates'
        // own preference for close-but-any-corner over far-but-preferred.
        let mut chosen = (px + gap, py - gap - height_px);
        let mut placed_ok = false;
        'search: for ring in 0..4 {
            let pad = gap * (1.0 + ring as f32);
            for &(h, v) in &DIRECTIONS {
                let left = match h {
                    1 => px + pad,
                    -1 => px - pad - width_px,
                    _ => px - width_px / 2.0,
                };
                let top = match v {
                    1 => py + pad,
                    -1 => py - pad - height_px,
                    _ => py - height_px / 2.0,
                };
                let candidate_box = LabelBox {
                    x: left,
                    y: top,
                    w: width_px,
                    h: height_px,
                };
                if !placed.iter().any(|p| boxes_overlap(p, &candidate_box)) {
                    chosen = (left, top);
                    placed_ok = true;
                    break 'search;
                }
            }
        }
        if !placed_ok {
            // Every ring collided — genuinely no free space nearby. Same
            // fallback as before: place it anyway, overlapping, rather
            // than hiding the label entirely.
            chosen = (px + gap, py - gap - height_px);
        }

        // Keep the label fully on-screen even when its contact is near the
        // edge of the visible range — sliding it back in reads much better
        // than letting text run off the image and get clipped. Only clamps
        // the label's own position; the blip itself is untouched, so this
        // stays correct as the aircraft keeps moving toward/along the edge.
        let margin = gap;
        chosen.0 = chosen
            .0
            .max(margin)
            .min((scene.width_px as f32 - width_px - margin).max(margin));
        chosen.1 = chosen
            .1
            .max(margin)
            .min((scene.height_px as f32 - height_px - margin).max(margin));

        placed.push(LabelBox {
            x: chosen.0,
            y: chosen.1,
            w: width_px,
            h: height_px,
        });

        // draw_text's y is a text baseline, not the top of the glyph — the
        // ascent sits above it. chosen.1/LabelBox treat the label as
        // starting at its visual top (for collision-avoidance and edge
        // clamping), so only the actual draw call needs the baseline
        // conversion, via an approximate ascent fraction of the row height.
        let baseline_offset = line_h * 0.8;
        for (row, line) in lines.iter().enumerate() {
            let y = chosen.1 + row as f32 * line_h + baseline_offset;
            let color = match row {
                0 => callsign_color,
                _ => base,
            };
            draw_text(pixmap, scene.font, scene.label_font_px, line, chosen.0, y, color);
        }
        // Redraw the climb/descent glyph on top, in its own color, right
        // where it already landed as part of the altitude line above —
        // simpler than threading multi-color runs through draw_text, at
        // the cost of rasterizing that one glyph twice.
        if let Some(color) = climb_color {
            let alt_row_y = chosen.1 + line_h + baseline_offset;
            let glyph_x = chosen.0 + text_width(scene.font, scene.label_font_px, &alt_base);
            draw_text(
                pixmap,
                scene.font,
                scene.label_font_px,
                climb_glyph,
                glyph_x,
                alt_row_y,
                color,
            );
        }
    }
}

fn text_width(font: &fontdue::Font, font_px: f32, text: &str) -> f32 {
    text.chars()
        .map(|ch| font.metrics(ch, font_px).advance_width)
        .sum()
}

fn measure(font: &fontdue::Font, font_px: f32, lines: &[String; LABEL_ROWS]) -> (f32, f32) {
    let max_w = lines
        .iter()
        .map(|line| text_width(font, font_px, line))
        .fold(0.0f32, f32::max);
    (max_w, font_px * 1.25)
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    pixmap: &mut Pixmap,
    font: &fontdue::Font,
    font_px: f32,
    text: &str,
    x: f32,
    y: f32,
    color: Color,
) {
    let (r, g, b) = match color {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (255, 255, 255),
    };
    let mut pen_x = x;
    for ch in text.chars() {
        let (metrics, bitmap) = font.rasterize(ch, font_px);
        let glyph_x = pen_x + metrics.xmin as f32;
        let glyph_y = y - metrics.ymin as f32 - metrics.height as f32;
        blit_glyph(
            pixmap,
            &bitmap,
            metrics.width,
            metrics.height,
            glyph_x,
            glyph_y,
            (r, g, b),
        );
        pen_x += metrics.advance_width;
    }
}

/// Alpha-composites (standard "over", in premultiplied space) a fontdue
/// coverage bitmap onto the pixmap with a solid color.
fn blit_glyph(
    pixmap: &mut Pixmap,
    bitmap: &[u8],
    w: usize,
    h: usize,
    x0: f32,
    y0: f32,
    (r, g, b): (u8, u8, u8),
) {
    if w == 0 || h == 0 {
        return;
    }
    let pw = pixmap.width() as i32;
    let ph = pixmap.height() as i32;
    let x0i = x0.round() as i32;
    let y0i = y0.round() as i32;
    let data = pixmap.pixels_mut();
    for row in 0..h as i32 {
        let py = y0i + row;
        if py < 0 || py >= ph {
            continue;
        }
        for col in 0..w as i32 {
            let px = x0i + col;
            if px < 0 || px >= pw {
                continue;
            }
            let coverage = u32::from(bitmap[row as usize * w + col as usize]);
            if coverage == 0 {
                continue;
            }
            let idx = (py * pw + px) as usize;
            let dst = data[idx];
            let inv = 255 - coverage;
            let out_r = (((r as u32 * coverage) + dst.red() as u32 * inv) / 255) as u8;
            let out_g = (((g as u32 * coverage) + dst.green() as u32 * inv) / 255) as u8;
            let out_b = (((b as u32 * coverage) + dst.blue() as u32 * inv) / 255) as u8;
            let out_a = ((coverage * 255 + dst.alpha() as u32 * inv) / 255) as u8;
            if let Some(color) = PremultipliedColorU8::from_rgba(out_r, out_g, out_b, out_a) {
                data[idx] = color;
            }
        }
    }
}

fn pixmap_to_image(pixmap: Pixmap) -> RgbaImage {
    let width = pixmap.width();
    let height = pixmap.height();
    let mut out = RgbaImage::new(width, height);
    for (i, px) in pixmap.pixels().iter().enumerate() {
        let x = i as u32 % width;
        let y = i as u32 / width;
        let straight = px.demultiply();
        out.put_pixel(
            x,
            y,
            image::Rgba([
                straight.red(),
                straight.green(),
                straight.blue(),
                px.alpha(),
            ]),
        );
    }
    out
}
