mod braille_scope;
mod data;
mod font;
mod geometry;
mod icons;
mod raster;
mod scope;
mod settings;
mod sixel_scope;
mod theme;
mod trail;
mod tui;

use crossterm::event::{self, Event, KeyCode};
use data::aircraft::{Aircraft, Altitude};
use ratatui_image::picker::Picker;
use std::time::{Duration, Instant};
use theme::ThemeWatcher;
use trail::TrailStore;

const ZOOM_STEP_NM: f64 = 5.0;
const DEFAULT_ZOOM_NM: f64 = 40.0;
const THEME_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Dev-only: `flyover --preview out.png` renders a synthetic scene straight
/// to a PNG, bypassing the terminal/network entirely — useful for checking
/// the raster output without a live TTY (which this can't assume exists).
fn run_preview(out_path: &str) -> std::io::Result<()> {
    let font = font::load_monospace().map_err(std::io::Error::other)?;
    let palette = ThemeWatcher::new().palette;

    // A synthetic-fixture builder for this dev-only preview scene doesn't
    // need the ergonomics a real API would; one extra arg over clippy's
    // default threshold isn't worth a builder struct here.
    #[allow(clippy::too_many_arguments)]
    fn ac(
        hex: &str,
        flight: &str,
        t: &str,
        category: &str,
        alt_ft: i64,
        gs: f64,
        rate: f64,
        squawk: &str,
        dst: f64,
        dir: f64,
    ) -> Aircraft {
        Aircraft {
            hex: hex.to_string(),
            flight: Some(flight.to_string()),
            r: None,
            t: Some(t.to_string()),
            // Sentinel: negative alt_ft means "on the ground" rather than
            // a real negative altitude, so this fixture-only helper can
            // cover that case too without a second constructor.
            alt_baro: Some(if alt_ft < 0 {
                Altitude::Ground
            } else {
                Altitude::Feet(alt_ft)
            }),
            gs: Some(gs),
            track: Some(dir),
            baro_rate: Some(rate),
            geom_rate: None,
            squawk: Some(squawk.to_string()),
            lat: None,
            lon: None,
            dst: Some(dst),
            dir: Some(dir),
            category: Some(category.to_string()),
        }
    }

    // One of each AircraftKind, so this preview doubles as a way to
    // eyeball every icon at once.
    let aircraft = vec![
        ac(
            "a1", "UAL1234", "B738", "A3", 35000, 420.0, 0.0, "1200", 20.0, 45.0,
        ), // Airliner
        ac(
            "a2", "ENY3937", "E75L", "A2", 8000, 250.0, -1800.0, "1200", 12.0, 200.0,
        ), // Regional
        ac(
            "a3", "N247JH", "C172", "A1", 4500, 110.0, 0.0, "7700", 30.0, 300.0,
        ), // Private
        ac(
            "a4", "EDGE001", "GLF6", "A1", 41000, 480.0, 0.0, "1200", 15.0, 130.0,
        ), // BusinessJet
        ac(
            "a5", "AAL2159", "A321", "A3", 36000, 406.0, 1500.0, "1200", 5.0, 5.0,
        ), // Airliner, climbing
        ac(
            "a6", "N911PD", "H60", "A7", 1200, 90.0, 0.0, "1200", 39.5, 2.0,
        ), // Helicopter
        ac(
            "a7", "BLIMP01", "", "", 2000, 30.0, 0.0, "1200", 25.0, 250.0,
        ), // Unknown
        ac(
            "a8", "N55TX", "C172", "A1", -1, 0.0, 0.0, "1200", 10.0, 90.0,
        ), // Ground: icon only, no label
    ];

    let mut trails = TrailStore::default();
    // Feed a few slightly-shifted snapshots so each contact has real trail
    // history to render (a single point wouldn't show the comet fade).
    for step in 0..5 {
        let shifted: Vec<Aircraft> = aircraft
            .iter()
            .map(|a| {
                let mut a = a.clone();
                a.dst = a.dst.map(|d| d - f64::from(4 - step) * 0.8);
                a
            })
            .collect();
        trails.update(&shifted);
    }
    trails.update(&aircraft);

    // Sample runways at a spread of distances (close/mid/near-edge/beyond
    // the 40nm zoom used below) so --preview can eyeball both the
    // center-to-edge dim fade and the beyond-the-rings cutoff at once.
    let runways = vec![
        data::airports::RunwaySegment {
            dst_a: 6.0,
            dir_a: 260.0,
            dst_b: 8.0,
            dir_b: 265.0,
        },
        data::airports::RunwaySegment {
            dst_a: 18.0,
            dir_a: 350.0,
            dst_b: 20.5,
            dir_b: 10.0,
        },
        data::airports::RunwaySegment {
            dst_a: 33.0,
            dir_a: 95.0,
            dst_b: 35.0,
            dir_b: 100.0,
        },
        data::airports::RunwaySegment {
            dst_a: 45.0,
            dir_a: 210.0,
            dst_b: 47.0,
            dir_b: 215.0,
        },
    ];

    // A rough octagon offset from home (not a real Class B shape, just
    // something with real geometry to eyeball the closed-loop stroke, fade,
    // and partial-off-screen behavior at once). Built in a local x/y offset
    // then converted back to (dst, dir) — fine for a preview fixture, no
    // need for geodesic precision here.
    let airspace = vec![data::airspace::AirspaceBoundary {
        points: (0..8)
            .map(|i| {
                let angle = f64::from(i) * 45.0;
                let (center_dst, center_dir, radius) = (22.0, 40.0_f64, 10.0);
                let dx =
                    center_dst * center_dir.to_radians().sin() + radius * angle.to_radians().sin();
                let dy =
                    center_dst * center_dir.to_radians().cos() + radius * angle.to_radians().cos();
                let dst = dx.hypot(dy);
                let dir = dx.atan2(dy).to_degrees().rem_euclid(360.0);
                (dst, dir)
            })
            .collect(),
    }];

    let scene = raster::Scene {
        width_px: 900,
        height_px: 900,
        aircraft: &aircraft,
        trails: &trails,
        runways: &runways,
        airspace: &airspace,
        zoom_radius_nm: 40.0,
        sweep_angle_deg: 50.0,
        palette: &palette,
        font: &font,
        label_font_px: raster::label_font_px(18.0),
        hide_labels: false,
    };
    let image = raster::render(&scene);
    image.save(out_path).map_err(std::io::Error::other)?;
    println!("wrote {out_path}");
    Ok(())
}

/// Dev-only: `flyover --bench` times the raster + Sixel-encode pipeline
/// against an in-memory TestBackend (no real TTY needed) to diagnose actual
/// per-frame cost, since this environment can't be used to eyeball a live
/// frame rate.
fn run_bench() -> Result<(), Box<dyn std::error::Error>> {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui_image::FontSize;
    use ratatui_image::picker::Picker;

    let font = font::load_monospace()?;
    let palette = ThemeWatcher::new().palette;
    let trails = TrailStore::default();
    let aircraft: Vec<Aircraft> = Vec::new();

    // Representative of a bigger tiled window: ~160 cols x 50 rows at a typical
    // JetBrainsMono cell size.
    let cols = 160u16;
    let rows = 50u16;
    #[allow(deprecated)]
    let mut picker = Picker::from_fontsize(FontSize {
        width: 9,
        height: 18,
    });
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Sixel);

    let backend = TestBackend::new(cols, rows);
    let mut terminal = Terminal::new(backend)?;
    let sweep_start = Instant::now();

    const N: u32 = 10;
    let mut raster_total = Duration::ZERO;
    let mut draw_total = Duration::ZERO;

    for _ in 0..N {
        let t0 = Instant::now();
        let font_size = picker.font_size();
        let width_px = u32::from(cols) * u32::from(font_size.width);
        let height_px = u32::from(rows) * u32::from(font_size.height);
        let scene = raster::Scene {
            width_px,
            height_px,
            aircraft: &aircraft,
            trails: &trails,
            runways: &[],
            airspace: &[],
            zoom_radius_nm: 40.0,
            sweep_angle_deg: 10.0,
            palette: &palette,
            font: &font,
            label_font_px: raster::label_font_px(f32::from(font_size.height)),
            hide_labels: false,
        };
        let image = raster::render(&scene);
        raster_total += t0.elapsed();

        let t1 = Instant::now();
        terminal.draw(|frame| {
            scope::render(
                frame,
                frame.area(),
                "bench".to_string(),
                "".to_string(),
                &aircraft,
                &trails,
                &[],
                &[],
                40.0,
                sweep_start,
                &palette,
                &picker,
                &font,
                scope::RenderMode::Sixel,
                false,
            );
        })?;
        draw_total += t1.elapsed();
        std::hint::black_box(&image);
    }

    println!(
        "sixel: raster::render {:.1}ms/frame, full terminal.draw {:.1}ms/frame",
        raster_total.as_secs_f64() * 1000.0 / f64::from(N),
        draw_total.as_secs_f64() * 1000.0 / f64::from(N)
    );

    let mut braille_total = Duration::ZERO;
    for _ in 0..N {
        let t0 = Instant::now();
        terminal.draw(|frame| {
            scope::render(
                frame,
                frame.area(),
                "bench".to_string(),
                "".to_string(),
                &aircraft,
                &trails,
                &[],
                &[],
                40.0,
                sweep_start,
                &palette,
                &picker,
                &font,
                scope::RenderMode::Braille,
                false,
            );
        })?;
        braille_total += t0.elapsed();
    }
    println!(
        "braille: full terminal.draw {:.1}ms/frame",
        braille_total.as_secs_f64() * 1000.0 / f64::from(N)
    );
    Ok(())
}

/// Screensaver-mode focus check: true once Hyprland's active window is no
/// longer the screensaver itself. Shelled out rather than piped in from the
/// wrapping script — an earlier version had Omarchy's screensaver script
/// background this process and poll `hyprctl`/read stdin itself, but that
/// made two processes race to read the same tty (this process's own
/// terminal-capability query at startup vs. the script's exit-on-keypress
/// read), which sporadically broke Sixel detection. Now this process owns
/// both checks itself and runs in the foreground with nothing else reading
/// its stdin.
fn screensaver_lost_focus() -> bool {
    let Ok(output) = std::process::Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()
    else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return false;
    };
    value.get("class").and_then(|c| c.as_str()) != Some("org.omarchy.screensaver")
}

/// `flyover --screensaver [--ascii]`: runs the same live scope (sweep,
/// fading trails, theme sync) as the interactive TUI, meant to be launched
/// by Omarchy's screensaver script in place of `ttfx`. Exits on any
/// keypress or when the screensaver window loses focus, so the wrapping
/// script only needs to run it in the foreground and clean up once it
/// returns. `--ascii` selects the Braille render mode, which — unlike
/// Sixel — never queries the terminal for image support, so it works
/// without ever touching stdout/stdin at startup.
fn run_screensaver(mode: scope::RenderMode) -> std::io::Result<()> {
    if cfg!(debug_assertions) {
        eprintln!("flyover: running a debug build — the scope will look choppy.");
        eprintln!("         use `cargo run --release` for smooth animation.");
    }

    let location = match data::location::load() {
        Ok(loc) => loc,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };

    let rx = data::fetch::spawn_poller(location.latitude, location.longitude);
    let airports_rx = data::airports::spawn_loader(location.latitude, location.longitude);
    let airspace_rx = data::airspace::spawn_loader(location.latitude, location.longitude);
    let mut aircraft: Vec<Aircraft> = Vec::new();
    let mut runways: Vec<data::airports::RunwaySegment> = Vec::new();
    let mut airspace: Vec<data::airspace::AirspaceBoundary> = Vec::new();
    let mut trails = TrailStore::default();
    let sweep_start = Instant::now();
    let mut theme = ThemeWatcher::new();
    let mut last_theme_check = Instant::now();
    let font = match font::load_monospace() {
        Ok(f) => f,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };

    let mut terminal = tui::init()?;
    let picker = match mode {
        scope::RenderMode::Sixel => match Picker::from_query_stdio() {
            Ok(p) => p,
            Err(err) => {
                tui::restore()?;
                eprintln!("couldn't detect terminal image support: {err}");
                std::process::exit(1);
            }
        },
        // Braille mode never draws an image, so the real query (and its
        // startup round-trip over stdin/stdout) is unnecessary — these
        // placeholder metrics are never used for anything.
        scope::RenderMode::Braille =>
        {
            #[allow(deprecated)]
            Picker::from_fontsize(ratatui_image::FontSize {
                width: 9,
                height: 18,
            })
        }
    };

    // Only enforce exit-on-focus-loss when actually launched as the real
    // screensaver window (Hyprland already reports us focused as
    // `org.omarchy.screensaver` by the time this runs, since the launcher
    // focuses the window before executing it). Run manually in an ordinary
    // terminal for testing, and this check would otherwise immediately see
    // "not focused" and self-terminate after about a second — so it's
    // disabled for the rest of the run whenever that's the case, falling
    // back to exit-on-keypress only.
    let watch_focus = !screensaver_lost_focus();
    let mut last_focus_check = Instant::now();
    const FOCUS_POLL_INTERVAL: Duration = Duration::from_secs(1);

    loop {
        if event::poll(Duration::from_millis(80))?
            && matches!(event::read()?, Event::Key(_) | Event::Mouse(_))
        {
            // Any keyboard or mouse input ends the screensaver. crossterm
            // also reports terminal Resize/FocusGained/FocusLost as events
            // here, which aren't user input -- treating those as "a key
            // was pressed" too meant an unrelated window resize (e.g. a
            // monitor change, or a compositor reflow) would silently kill
            // the screensaver.
            break;
        }

        if watch_focus && last_focus_check.elapsed() >= FOCUS_POLL_INTERVAL {
            if screensaver_lost_focus() {
                break;
            }
            last_focus_check = Instant::now();
        }

        if last_theme_check.elapsed() >= THEME_POLL_INTERVAL {
            theme.poll();
            last_theme_check = Instant::now();
        }

        while let Ok(result) = rx.try_recv()
            && let Ok(list) = result
        {
            trails.update(&list);
            aircraft = list;
        }
        if let Ok(Ok(list)) = airports_rx.try_recv() {
            runways = list;
        }
        if let Ok(Ok(list)) = airspace_rx.try_recv() {
            airspace = list;
        }

        let title = format!(
            " flyover — {} — {} contact(s) ",
            location.name,
            aircraft.len()
        );

        terminal.draw(|frame| {
            scope::render(
                frame,
                frame.area(),
                title,
                String::new(),
                &aircraft,
                &trails,
                &runways,
                &airspace,
                DEFAULT_ZOOM_NM,
                sweep_start,
                &theme.palette,
                &picker,
                &font,
                mode,
                true,
            );
        })?;
    }

    tui::restore()?;
    Ok(())
}

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--preview") => {
            let out_path = args.next().unwrap_or_else(|| "preview.png".to_string());
            return run_preview(&out_path);
        }
        Some("--bench") => {
            return run_bench().map_err(|e| std::io::Error::other(e.to_string()));
        }
        Some("--screensaver") => {
            // --ascii/--sixel override the saved setting for one-off manual
            // testing without touching what the interactive TUI is set to.
            let mode = match args.next().as_deref() {
                Some("--ascii") => scope::RenderMode::Braille,
                Some("--sixel") => scope::RenderMode::Sixel,
                _ => settings::load_render_mode(scope::RenderMode::Sixel),
            };
            return run_screensaver(mode);
        }
        _ => {}
    }

    // Sixel encoding is real per-frame work; in a debug build it dominates
    // frame time so badly (~400ms/frame measured vs ~10ms in release) that
    // the sweep animation looks like it's jumping every few seconds instead
    // of rotating smoothly. `cargo run` defaults to debug, so warn plainly
    // rather than let that read as a rendering bug.
    if cfg!(debug_assertions) {
        eprintln!("flyover: running a debug build — the scope will look choppy.");
        eprintln!("         use `cargo run --release` for smooth animation.");
    }

    let location = match data::location::load() {
        Ok(loc) => loc,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };

    let rx = data::fetch::spawn_poller(location.latitude, location.longitude);
    let airports_rx = data::airports::spawn_loader(location.latitude, location.longitude);
    let airspace_rx = data::airspace::spawn_loader(location.latitude, location.longitude);
    let mut aircraft: Vec<Aircraft> = Vec::new();
    let mut runways: Vec<data::airports::RunwaySegment> = Vec::new();
    let mut airspace: Vec<data::airspace::AirspaceBoundary> = Vec::new();
    let mut trails = TrailStore::default();
    let mut last_error: Option<String> = None;
    let mut last_update: Option<Instant> = None;
    let mut zoom_radius_nm: f64 = DEFAULT_ZOOM_NM;
    let mut render_mode = settings::load_render_mode(scope::RenderMode::Sixel);
    let sweep_start = Instant::now();
    let mut theme = ThemeWatcher::new();
    let mut last_theme_check = Instant::now();
    let font = match font::load_monospace() {
        Ok(f) => f,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };

    let mut terminal = tui::init()?;
    let picker = match Picker::from_query_stdio() {
        Ok(p) => p,
        Err(err) => {
            tui::restore()?;
            eprintln!("couldn't detect terminal image support: {err}");
            std::process::exit(1);
        }
    };

    loop {
        if event::poll(Duration::from_millis(80))?
            && let Event::Key(key) = event::read()?
        {
            // Letter keys reflect Caps Lock (crossterm reports the actual
            // character produced, so 'q' becomes 'Q' with Caps Lock on) —
            // lowercase before matching so shortcuts work regardless.
            let code = match key.code {
                KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
                other => other,
            };
            match code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Up => {
                    zoom_radius_nm = (zoom_radius_nm - ZOOM_STEP_NM).max(scope::MIN_ZOOM_NM);
                }
                KeyCode::Char('-') | KeyCode::Char('_') | KeyCode::Down => {
                    zoom_radius_nm = (zoom_radius_nm + ZOOM_STEP_NM).min(scope::MAX_ZOOM_NM);
                }
                KeyCode::Char('0') => zoom_radius_nm = DEFAULT_ZOOM_NM,
                KeyCode::Char('v') => {
                    render_mode = render_mode.toggled();
                    settings::save_render_mode(render_mode);
                }
                _ => {}
            }
        }

        if last_theme_check.elapsed() >= THEME_POLL_INTERVAL {
            theme.poll();
            last_theme_check = Instant::now();
        }

        while let Ok(result) = rx.try_recv() {
            match result {
                Ok(list) => {
                    trails.update(&list);
                    aircraft = list;
                    last_error = None;
                    last_update = Some(Instant::now());
                }
                Err(err) => last_error = Some(err),
            }
        }
        if let Ok(Ok(list)) = airports_rx.try_recv() {
            runways = list;
        }
        if let Ok(Ok(list)) = airspace_rx.try_recv() {
            airspace = list;
        }

        let title = format!(
            " flyover — {} — {} contact(s) — {:.0}nm range ",
            location.name,
            aircraft.len(),
            zoom_radius_nm
        );
        let status = match (last_update, &last_error) {
            (_, Some(_)) => "err".to_string(),
            (Some(t), None) => {
                let remaining = data::fetch::REFRESH_INTERVAL.saturating_sub(t.elapsed());
                format!("↻{}s", remaining.as_secs())
            }
            (None, None) => "…".to_string(),
        };

        terminal.draw(|frame| {
            scope::render(
                frame,
                frame.area(),
                title,
                status,
                &aircraft,
                &trails,
                &runways,
                &airspace,
                zoom_radius_nm,
                sweep_start,
                &theme.palette,
                &picker,
                &font,
                render_mode,
                false,
            );
        })?;
    }

    tui::restore()?;
    Ok(())
}
