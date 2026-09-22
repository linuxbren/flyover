/// adsb.lol gives distance (nm) and bearing (degrees, 0 = north, clockwise)
/// from the query point directly, so plotting a contact is just polar-to-
/// cartesian — no lat/lon projection math needed.
pub fn bearing_to_xy(dst_nm: f64, dir_deg: f64) -> (f64, f64) {
    let rad = dir_deg.to_radians();
    (dst_nm * rad.sin(), dst_nm * rad.cos())
}

// Shared by both render modes so switching modes doesn't also change the
// sweep's timing.
const SWEEP_PERIOD: std::time::Duration = std::time::Duration::from_secs(40);

// Also shared by both render modes, so the trailing fade behind the sweep
// beam covers the same angular span regardless of which one is active.
// Neither ratatui's Canvas nor tiny-skia exposes a sweep/conic-gradient
// shader, so both renderers approximate the fade with discrete steps
// stepping back from the beam, each dimmer than the last, rather than a
// true continuous gradient.
//
// The two renderers use SWEEP_TRAIL_STEPS differently. raster.rs fills a
// triangle per step (center + two arc points) — filled areas tile with no
// seam regardless of window size, so a fixed count here only affects
// gradient smoothness, not gap-freedom. braille_scope.rs can only paint
// points/lines, not filled polygons, so it treats this as a *floor* and
// scales the actual step count up to the canvas's real dot resolution —
// otherwise a fixed count leaves visible gaps once the physical radius (in
// dots) outgrows what that many radial samples can cover, which is exactly
// what a bigger terminal window exposed.
pub const SWEEP_TRAIL_SPAN_DEG: f64 = 25.0;
pub const SWEEP_TRAIL_STEPS: u32 = 32;

pub fn sweep_angle_deg(sweep_start: std::time::Instant) -> f64 {
    let elapsed = sweep_start.elapsed().as_secs_f64();
    let period = SWEEP_PERIOD.as_secs_f64();
    (elapsed / period * 360.0) % 360.0
}
