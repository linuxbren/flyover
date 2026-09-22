use crate::data::aircraft::{Aircraft, Altitude};
use crate::geometry::{self, bearing_to_xy};
use crate::theme::Palette;
use crate::trail::TrailStore;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line as TextLine, Span};
use ratatui::widgets::canvas::{Canvas, Circle, Line as CanvasLine, Points};

const RING_COUNT: u32 = 4;
const LABEL_ROWS: usize = 3;

/// The 8 directions a contact label can be anchored in relative to its
/// blip, as (horizontal, vertical) signs: +1/-1 means the label extends
/// that way from the blip (which then sits at that edge, offset by the
/// search's padding); 0 means the label is centered on the blip along
/// that axis instead (used for the 4 cardinal directions). Vertical sign
/// is in world space (+1 = north/up), matching this renderer's
/// y-increases-north canvas convention — raster.rs's copy of this same
/// table is in screen/pixel space (+1 = down) instead, since that's what
/// it works in; don't copy one file's DIRECTIONS into the other as-is.
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

/// Fades a theme color from black (t=0, oldest) up toward its full
/// brightness (t close to 1), without ever reaching the full brightness
/// reserved for the live blip itself.
fn faded(base: Color, t: f64) -> Color {
    let Color::Rgb(r, g, b) = base else {
        return base;
    };
    let t = t.clamp(0.0, 1.0) * 0.8 + 0.1;
    Color::Rgb(
        (f64::from(r) * t) as u8,
        (f64::from(g) * t) as u8,
        (f64::from(b) * t) as u8,
    )
}

/// Axis-aligned box anchored at its top-left corner (x, y), extending right
/// by `w` and down by `h` — "down" meaning decreasing y, since canvas space
/// is y-up (north-up).
struct LabelBox {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn boxes_overlap(a: &LabelBox, b: &LabelBox) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && (a.y - a.h) < b.y && (b.y - b.h) < a.y
}

/// The original braille-based renderer, kept as a fast fallback mode: no
/// image encoding, so it animates smoothly even where Sixel's real-world
/// per-frame cost (mostly the terminal's own decode, not this app) doesn't.
#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    inner: Rect,
    aircraft: &[Aircraft],
    trails: &TrailStore,
    zoom_radius_nm: f64,
    sweep_angle_deg: f64,
    palette: &Palette,
    // Screensaver mode: skip the callsign/altitude/speed label entirely
    // for every contact — icons/blips and trails still draw. See
    // scope::render for why.
    hide_labels: bool,
) {
    // Terminal cells are roughly twice as tall as they are wide, so without
    // this correction the range rings render as ellipses, not circles.
    let aspect = if inner.height > 0 {
        (inner.width as f64) / (inner.height as f64 * 2.0)
    } else {
        1.0
    };
    let x_reach = zoom_radius_nm * aspect.max(0.5);

    // Dots per nm, the denser of the two axes (Braille is 2 dots wide × 4
    // tall per cell). Used both to size the sweep's small overshoot past
    // the outer ring in absolute dots, and to scale the trail's step count
    // below.
    let x_dots_per_nm = f64::from(inner.width) * 2.0 / (2.0 * x_reach).max(0.001);
    let y_dots_per_nm = f64::from(inner.height) * 4.0 / (2.0 * zoom_radius_nm).max(0.001);
    let dots_per_nm = x_dots_per_nm.max(y_dots_per_nm);

    // The beam/trail's own radius — a few dots past the outer ring rather
    // than sitting exactly on it, so it still reads as sweeping past the
    // edge. Deliberately NOT x_reach: the rings are true circles (equal
    // radius on both axes in world space, same as ratatui's own Circle
    // shape), but x_reach is the *aspect-corrected* x-axis reach, which is
    // only equal to zoom_radius_nm when the canvas happens to be square.
    // sweep_point used to take x_reach for its x-component and
    // zoom_radius_nm for y — tracing an ellipse, not the rings' circle —
    // so in a wide window the beam shot straight past the ring on the
    // sides while still landing right on it at the top/bottom.
    const SWEEP_OVERSHOOT_DOTS: f64 = 3.0;
    let sweep_radius_nm = zoom_radius_nm + SWEEP_OVERSHOOT_DOTS / dots_per_nm.max(0.001);
    let sweep_point = |angle_deg: f64| -> (f64, f64) {
        let rad = angle_deg.to_radians();
        (sweep_radius_nm * rad.sin(), sweep_radius_nm * rad.cos())
    };
    let (sweep_x, sweep_y) = sweep_point(sweep_angle_deg);

    // Scale the trail's step count to the canvas's actual dot resolution
    // rather than using a fixed count: a fixed count leaves visible gaps
    // once the physical radius, in dots, outgrows what that many radial
    // samples can cover — which is exactly what a bigger terminal window
    // exposed. The sub-dot target spacing keeps a safety margin against
    // rounding so adjacent Bresenham lines always touch.
    let outer_arc_len_dots =
        sweep_radius_nm * geometry::SWEEP_TRAIL_SPAN_DEG.to_radians() * dots_per_nm;
    let trail_steps =
        ((outer_arc_len_dots / 0.6).ceil() as u32).clamp(geometry::SWEEP_TRAIL_STEPS, 500);
    let trail_step_deg =
        geometry::SWEEP_TRAIL_SPAN_DEG / f64::from(trail_steps.saturating_sub(1).max(1));

    // Size a label's bounding box in nm using the actual terminal cell size,
    // since that's the granularity text is drawn at regardless of the
    // higher-resolution braille marker used for shapes. Denominator is
    // `- 1`, matching exactly how ratatui's own Canvas maps a label's world
    // Y to a cell row internally (`canvas_area.height - 1`, see its
    // `impl Widget for Canvas`) — get this wrong and this step is very
    // slightly smaller than what the library actually moves per cell, so
    // over multiple rows the accumulated position falls short of a full
    // N-cell move and two rows can truncate to the same output cell, with
    // the later row silently overwriting the earlier one. That's exactly
    // what caused an aircraft label to intermittently lose its altitude
    // line (the middle of its three rows).
    let nm_per_col = (2.0 * x_reach) / f64::from(inner.width.saturating_sub(1).max(1));
    let nm_per_row = (2.0 * zoom_radius_nm) / f64::from(inner.height.saturating_sub(1).max(1));
    let gap_x = nm_per_col * 1.5;
    let gap_y = nm_per_row * 1.5;

    let mut contacts: Vec<&Aircraft> = aircraft
        .iter()
        .filter(|ac| matches!((ac.dst, ac.dir), (Some(d), _) if d <= zoom_radius_nm))
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

    let palette = *palette;

    let canvas = Canvas::default()
        .background_color(palette.background)
        .marker(Marker::Braille)
        .x_bounds([-x_reach, x_reach])
        .y_bounds([-zoom_radius_nm, zoom_radius_nm])
        .paint(move |ctx| {
            for i in 1..=RING_COUNT {
                let frac = f64::from(i) / f64::from(RING_COUNT);
                ctx.draw(&Circle {
                    x: 0.0,
                    y: 0.0,
                    radius: zoom_radius_nm * frac,
                    color: palette.muted,
                });
            }

            // Fading trail behind the beam: discrete stepped lines dimming
            // toward the tail, drawn before the beam itself so the beam
            // paints on top at full brightness. Step count/angle computed
            // above, scaled to the canvas's actual dot resolution.
            for step in 0..trail_steps {
                let t = 1.0 - f64::from(step) / f64::from(trail_steps.saturating_sub(1).max(1));
                let (tx, ty) = sweep_point(sweep_angle_deg - f64::from(step) * trail_step_deg);
                ctx.draw(&CanvasLine {
                    x1: 0.0,
                    y1: 0.0,
                    x2: tx,
                    y2: ty,
                    color: faded(palette.accent, t),
                });
            }

            ctx.draw(&CanvasLine {
                x1: 0.0,
                y1: 0.0,
                x2: sweep_x,
                y2: sweep_y,
                color: palette.accent,
            });

            for ac in &contacts {
                if let Some(trail) = trails.get(&ac.hex) {
                    let len = trail.len();
                    let base = if ac.is_emergency_squawk() {
                        palette.alert
                    } else {
                        palette.kind_color(ac.kind())
                    };
                    for (i, (tx, ty)) in trail.iter().enumerate() {
                        let t = (i + 1) as f64 / (len + 1) as f64;
                        ctx.draw(&Points {
                            coords: &[(*tx, *ty)],
                            color: faded(base, t),
                        });
                    }
                }
            }

            let mut placed: Vec<LabelBox> = Vec::with_capacity(contacts.len());

            for ac in &contacts {
                let (dst, dir) = match (ac.dst, ac.dir) {
                    (Some(d), Some(b)) => (d, b),
                    _ => continue,
                };
                let (x, y) = bearing_to_xy(dst, dir);
                let color = if ac.is_emergency_squawk() {
                    palette.alert
                } else {
                    palette.foreground
                };

                ctx.draw(&Points {
                    coords: &[(x, y)],
                    color,
                });

                // Ground traffic (an airport's ramp/taxiways) is the
                // single densest source of clutter this app sees — easily
                // dozens of contacts in a tiny physical radius, which no
                // amount of smarter label search fixes. Blip only, no
                // label, for anything parked or taxiing.
                if matches!(ac.alt_baro, Some(Altitude::Ground)) {
                    continue;
                }
                if hide_labels {
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
                // Full text (glyph included) for width/collision boxing;
                // the actual print below splits it back apart so the
                // glyph can carry its own climb/descent color.
                let lines = [
                    ac.callsign().to_string(),
                    format!("{alt_base}{climb_glyph}"),
                    format!("{:.0}kt", ac.gs.unwrap_or(0.0)),
                ];
                let callsign_color = if ac.is_emergency_squawk() {
                    color
                } else {
                    palette.kind_color(ac.kind())
                };
                let climb_color = if ac.is_emergency_squawk() {
                    None
                } else {
                    palette.climb_rate_color(climb_rate)
                };

                let width_nm =
                    lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64 * nm_per_col;
                let height_nm = LABEL_ROWS as f64 * nm_per_row;

                // Corner-anchored search: 8 directions around the blip,
                // each placing the label so the blip sits at that
                // direction's near corner (or edge midpoint, for the 4
                // cardinal directions) of the label — never the label's
                // center. This is what all 4 of the original fixed
                // candidates already did (icon at a corner, never
                // floating in open space away from it); this just extends
                // that to all 8 directions and adds increasing padding
                // rings on top. x/y padding scale by their own gap unit
                // (gap_x/gap_y aren't the same nm-per-cell, since the
                // x-axis is aspect corrected — see x_reach above), so a
                // ring's padding looks even on *screen*, not just in raw
                // nm. Ring-major order (try every direction at the
                // tightest padding before accepting more padding in any
                // direction), matching the original candidates' own
                // preference for close-but-any-corner over
                // far-but-preferred.
                let mut chosen = (x + gap_x, y - gap_y);
                let mut placed_ok = false;
                'search: for ring in 0..4 {
                    let pad_x = gap_x * (1.0 + f64::from(ring));
                    let pad_y = gap_y * (1.0 + f64::from(ring));
                    for &(h, v) in &DIRECTIONS {
                        let left = match h {
                            1 => x + pad_x,
                            -1 => x - pad_x - width_nm,
                            _ => x - width_nm / 2.0,
                        };
                        let top = match v {
                            1 => y + pad_y + height_nm,
                            -1 => y - pad_y,
                            _ => y + height_nm / 2.0,
                        };
                        let candidate_box = LabelBox {
                            x: left,
                            y: top,
                            w: width_nm,
                            h: height_nm,
                        };
                        if !placed.iter().any(|p| boxes_overlap(p, &candidate_box)) {
                            chosen = (left, top);
                            placed_ok = true;
                            break 'search;
                        }
                    }
                }
                if !placed_ok {
                    // Every ring collided — genuinely no free space nearby.
                    // Same fallback as before: place it anyway, overlapping,
                    // rather than hiding the label entirely.
                    chosen = (x + gap_x, y - gap_y);
                }

                // Keep the label fully on-screen even when its contact is
                // near the edge of the visible range — sliding it back in
                // reads much better than letting text run off the canvas
                // and get clipped. Only the label moves; the blip stays put.
                chosen.0 = chosen
                    .0
                    .max(-x_reach)
                    .min((x_reach - width_nm).max(-x_reach));
                chosen.1 = chosen
                    .1
                    .max((-zoom_radius_nm + height_nm).min(zoom_radius_nm))
                    .min(zoom_radius_nm);

                placed.push(LabelBox {
                    x: chosen.0,
                    y: chosen.1,
                    w: width_nm,
                    h: height_nm,
                });

                let style = Style::default().fg(color);
                for (row, line) in lines.iter().enumerate() {
                    let y = chosen.1 - row as f64 * nm_per_row;
                    let rendered = match row {
                        0 => TextLine::styled(line.clone(), Style::default().fg(callsign_color)),
                        1 => match climb_color {
                            Some(climb_style_color) => TextLine::from(vec![
                                Span::styled(alt_base.clone(), style),
                                Span::styled(climb_glyph, Style::default().fg(climb_style_color)),
                            ]),
                            None => TextLine::styled(line.clone(), style),
                        },
                        _ => TextLine::styled(line.clone(), style),
                    };
                    ctx.print(chosen.0, y, rendered);
                }
            }
        });

    frame.render_widget(canvas, inner);
}
