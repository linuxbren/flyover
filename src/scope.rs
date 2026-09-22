use crate::data::aircraft::Aircraft;
use crate::geometry::sweep_angle_deg;
use crate::theme::Palette;
use crate::trail::TrailStore;
use crate::{braille_scope, sixel_scope};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui_image::picker::Picker;
use std::time::Instant;

pub const MIN_ZOOM_NM: f64 = 5.0;
pub const MAX_ZOOM_NM: f64 = 100.0;
const CONTROLS: &str = "q   +/-   0   v";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RenderMode {
    /// Anti-aliased CRT-glow graphics via an off-screen rasterizer + Sixel.
    /// Better looking, but real-world per-frame cost (mostly the terminal's
    /// own Sixel decode, not this app) can make animation less smooth.
    Sixel,
    /// The original braille-Canvas renderer: character-based, no image
    /// encoding, so it's inherently fast and animates smoothly.
    Braille,
}

impl RenderMode {
    pub fn toggled(self) -> Self {
        match self {
            RenderMode::Sixel => RenderMode::Braille,
            RenderMode::Braille => RenderMode::Sixel,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    area: Rect,
    title: String,
    status: String,
    aircraft: &[Aircraft],
    trails: &TrailStore,
    zoom_radius_nm: f64,
    sweep_start: Instant,
    palette: &Palette,
    picker: &Picker,
    font: &fontdue::Font,
    mode: RenderMode,
    // Screensaver mode: ambient motion to glance at, not something you
    // read individual flight data off of, and the screensaver terminal's
    // font is Omarchy's own (much larger than a normal desktop terminal,
    // sized for ttfx's branding text) — every aircraft label at that size
    // is real clutter with no code-level way to shrink it back down (only
    // sixel's own rasterized label text has a size knob at all; this is
    // simpler and fixes both render modes at once, no system font patch
    // needed). Icons/blips and trails still draw either way.
    hide_labels: bool,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1), Constraint::Length(1)])
        .split(area);
    let scope_area = chunks[0];
    let footer_area = chunks[1];
    let legend_area = chunks[2];

    let footer_text = if footer_area.width as usize >= CONTROLS.len() + status.len() + 4 {
        format!("{CONTROLS}   {status} ")
    } else {
        format!("{CONTROLS} ")
    };
    frame.render_widget(
        Paragraph::new(footer_text)
            .style(Style::default().fg(palette.muted))
            .alignment(Alignment::Right),
        footer_area,
    );

    render_legend(frame, legend_area, palette);

    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(scope_area);
    frame.render_widget(block, scope_area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let angle = sweep_angle_deg(sweep_start);
    match mode {
        RenderMode::Sixel => sixel_scope::render(
            frame,
            inner,
            aircraft,
            trails,
            zoom_radius_nm,
            angle,
            palette,
            picker,
            font,
            hide_labels,
        ),
        RenderMode::Braille => braille_scope::render(
            frame,
            inner,
            aircraft,
            trails,
            zoom_radius_nm,
            angle,
            palette,
            hide_labels,
        ),
    }
}

/// One line naming what each label/icon color denotes — the mapping is
/// otherwise only discoverable by noticing a differently-colored contact
/// and guessing. Built once here rather than per render mode since the
/// color scheme itself (`Palette::kind_color`/`climb_rate_color`) is
/// shared by both. Airliner and Unknown share `foreground` and are both
/// covered by the one "Airliner" entry — they're visually identical, so a
/// separate "Unknown" entry would just be clutter for a color a viewer
/// can't actually distinguish from it.
fn render_legend(frame: &mut Frame, area: Rect, palette: &Palette) {
    let entries: [(&str, ratatui::style::Color); 7] = [
        ("Airliner", palette.foreground),
        ("Regional", palette.regional),
        ("Business", palette.business_jet),
        ("Private", palette.private),
        ("Helicopter", palette.helicopter),
        ("▲ Climb", palette.climb),
        ("▼ Descent", palette.descent),
    ];

    let plain_len: usize = entries.iter().map(|(label, _)| label.len()).sum::<usize>()
        + (entries.len() - 1) * 2;
    if area.width as usize <= plain_len {
        // No graceful shrink for this one — a truncated, half-cut-off key
        // would be worse than no key at all. Just skip it on narrow
        // terminals; the controls/status footer above still fits.
        return;
    }

    let mut spans = Vec::with_capacity(entries.len() * 2 - 1);
    for (i, (label, color)) in entries.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(*label, Style::default().fg(*color)));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
        area,
    );
}
