# Changelog

All notable changes to this project are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning
follows [SemVer](https://semver.org/).

## [Unreleased]

## [0.3.1] - 2026-09-22

Docs-only release — no code changes — so crates.io's rendered README
(frozen at whatever a version had when published) matches what's
actually on GitHub.

### Changed

- README: documented the manual PKGBUILD install path (AUR account
  registrations are closed to new signups right now), updated the hero
  screenshot to show both render modes side by side, and added a
  screenshot of the bar widget's popup to the "Bar widget" section.

## [0.3.0] - 2026-09-22

### Added

- Real nearby airport/runway silhouettes on the sixel scope, sourced from
  [OurAirports](https://ourairports.com/) — dim by design, fading with
  distance and culled past the outer range ring so they read as
  backdrop, not clutter.
- Class B airspace boundaries overlaid as a dim reference layer, sourced
  live from the FAA's own ArcGIS airspace service.
- Per-category aircraft icons (airliner, regional, business jet, private,
  helicopter) that rotate with heading, sixel mode only.
- Fading PPI-style sweep trail behind the rotating beam, in both render
  modes.
- Aircraft labels, trails, and climb/descent indicators now color-coded
  by category, with a color-key legend under the scope.

### Changed

- Airport silhouettes only show towered airports — Class B/C nationwide,
  plus any Class D/E field within 10nm of your configured location —
  instead of every paved runway, to keep the display readable.
- Contact labels reworked to anchor at a corner of their icon with more
  padding, cutting down on overlap in busy airspace.
- Trail history doubled.
- Screensaver mode now hides per-contact text labels entirely (icons and
  trails still draw) — ambient motion to glance at, not data to read.

## [0.2.1] - 2026-09-17

### Added

- `flyover branding screensaver` enable/disable wired into Omarchy's own
  branding-screensaver menu.

### Fixed

- Screensaver no longer exits on an unrelated window resize (e.g. a
  monitor change) — only real keyboard/mouse input or focus loss ends it.

## [0.2.0] - 2026-09-16

### Added

- `flyover --screensaver` — runs the live scope (sweep, fading trails,
  theme sync) in place of Omarchy's stock `ttfx` screensaver text, with
  an optional patch script to wire it in. `--ascii`/`--sixel` flags
  override the render mode for one-off testing.
- Screensaver mode now persists whichever render mode you last picked
  with `v` in the interactive TUI, instead of always defaulting to Sixel.
- Published to [crates.io](https://crates.io/crates/flyover)
  (`cargo install flyover`) and to the AUR.

### Fixed

- A stdin race that could break Sixel terminal-support detection when
  launched as a screensaver.

## [0.1.0] - 2026-09-16

Initial release.

### Added

- Real-time ADS-B radar scope in the terminal, pulling live positions
  from [adsb.lol](https://adsb.lol) for your configured location.
- Two render modes: **Sixel** (anti-aliased CRT-glow raster) and
  **Braille** (character-based, animates smoothly everywhere).
- Range rings, rotating sweep, fading comet trails, and per-contact data
  tags (callsign, altitude, groundspeed, climb/descend).
- Live Omarchy theme sync — the scope always matches your active theme.
- Companion bar widget ([flyover-pill](https://github.com/linuxbren/flyover-pill))
  with an ambient aircraft count, click to launch the scope.

[Unreleased]: https://github.com/linuxbren/flyover/compare/v0.3.1...HEAD
[0.3.1]: https://github.com/linuxbren/flyover/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/linuxbren/flyover/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/linuxbren/flyover/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/linuxbren/flyover/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/linuxbren/flyover/releases/tag/v0.1.0
