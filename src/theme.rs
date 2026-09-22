use ratatui::style::Color;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Deserialize)]
struct RawColors {
    background: Option<String>,
    foreground: Option<String>,
    accent: Option<String>,
    muted: Option<String>,
    red: Option<String>,
    green: Option<String>,
    yellow: Option<String>,
    orange: Option<String>,
    cyan: Option<String>,
    blue: Option<String>,
    magenta: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub background: Color,
    pub foreground: Color,
    pub accent: Color,
    pub muted: Color,
    pub alert: Color,
    // The rest are for aircraft.rs::AircraftKind labels and the
    // climb/descent indicator — see Palette::kind_color/climb_color below.
    // Reusing the theme's own named ANSI-ish slots (already present in
    // every Omarchy colors.toml this app reads) rather than inventing new
    // theme surface just for this.
    pub climb: Color,
    pub descent: Color,
    pub regional: Color,
    pub business_jet: Color,
    pub private: Color,
    pub helicopter: Color,
}

impl Default for Palette {
    /// Classic CRT-green fallback for when colors.toml is missing or
    /// unparseable (e.g. running somewhere other than Omarchy). Uses Rgb
    /// (not the named Color variants) so trail fading — which only knows
    /// how to dim Rgb colors — still works outside Omarchy.
    fn default() -> Self {
        Self {
            background: Color::Rgb(0, 0, 0),
            foreground: Color::Rgb(0, 255, 0),
            accent: Color::Rgb(0, 255, 0),
            muted: Color::Rgb(80, 80, 80),
            alert: Color::Rgb(255, 0, 0),
            climb: Color::Rgb(0, 255, 0),
            descent: Color::Rgb(255, 255, 0),
            regional: Color::Rgb(0, 170, 255),
            business_jet: Color::Rgb(255, 0, 255),
            private: Color::Rgb(0, 255, 255),
            helicopter: Color::Rgb(255, 165, 0),
        }
    }
}

fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

fn colors_toml_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".local/state/omarchy/current/theme/colors.toml")
}

fn load_palette(path: &Path) -> Palette {
    let fallback = Palette::default();
    let Some(raw) = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str::<RawColors>(&s).ok())
    else {
        return fallback;
    };
    Palette {
        background: raw
            .background
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fallback.background),
        foreground: raw
            .foreground
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fallback.foreground),
        accent: raw
            .accent
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fallback.accent),
        muted: raw
            .muted
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fallback.muted),
        alert: raw
            .red
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fallback.alert),
        climb: raw
            .green
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fallback.climb),
        descent: raw
            .yellow
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fallback.descent),
        regional: raw
            .blue
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fallback.regional),
        business_jet: raw
            .magenta
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fallback.business_jet),
        private: raw
            .cyan
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fallback.private),
        helicopter: raw
            .orange
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fallback.helicopter),
    }
}

impl Palette {
    /// Label color for an aircraft's callsign, by category — Airliner and
    /// Unknown deliberately stay `foreground` (the majority of contacts,
    /// and "we don't actually know," both want to read as the default
    /// rather than draw the eye). Emergency-squawk `alert` always wins
    /// over this — callers check that first, not this function.
    pub fn kind_color(&self, kind: crate::data::aircraft::AircraftKind) -> Color {
        use crate::data::aircraft::AircraftKind;
        match kind {
            AircraftKind::Airliner | AircraftKind::Unknown => self.foreground,
            AircraftKind::Regional => self.regional,
            AircraftKind::BusinessJet => self.business_jet,
            AircraftKind::Private => self.private,
            AircraftKind::Helicopter => self.helicopter,
        }
    }

    /// Color for the climb/descent arrow specifically (not the rest of the
    /// altitude line) — `None` when the rate is within the existing
    /// +/-100fpm "level" threshold, i.e. there's no arrow to color.
    pub fn climb_rate_color(&self, rate: Option<f64>) -> Option<Color> {
        match rate {
            Some(r) if r > 100.0 => Some(self.climb),
            Some(r) if r < -100.0 => Some(self.descent),
            _ => None,
        }
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Watches Omarchy's active-theme colors.toml and re-reads it when it
/// changes. Omarchy re-stages this file (as a real file, not a symlink) on
/// every `omarchy theme set`, so a cheap mtime check is enough to pick up a
/// live theme switch without inotify or a hook install.
pub struct ThemeWatcher {
    path: PathBuf,
    last_mtime: Option<SystemTime>,
    pub palette: Palette,
}

impl ThemeWatcher {
    pub fn new() -> Self {
        let path = colors_toml_path();
        let palette = load_palette(&path);
        let last_mtime = mtime(&path);
        Self {
            path,
            last_mtime,
            palette,
        }
    }

    /// Cheap to call frequently — just a stat() unless the file actually
    /// changed. Returns true if the palette was reloaded.
    pub fn poll(&mut self) -> bool {
        let current = mtime(&self.path);
        if current.is_none() || current == self.last_mtime {
            return false;
        }
        self.palette = load_palette(&self.path);
        self.last_mtime = current;
        true
    }
}
