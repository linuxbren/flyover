# AGENTS.md

Instructions for AI coding agents working in this repo. See also
[README.md](README.md) for user-facing docs and [CHANGELOG.md](CHANGELOG.md)
for release history.

## Build, lint, test

Always use `--release`. Sixel encoding is real per-frame work; a debug
build is noticeably choppy (the app itself warns about this on startup).

```
cargo build --release
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --verbose
```

CI (`.github/workflows/ci.yml`) runs all four on every push/PR — clippy
and fmt must both be clean, not just warning-free by eye.

## Verifying a change

There's no test harness for the actual rendered output — verify visually:

- `cargo run --release -- --preview <path>.png` renders a synthetic
  scene (one aircraft per category, fixed positions) straight to a PNG,
  bypassing the terminal and network entirely. The fastest way to check
  a rendering change.
- `cargo run --release -- --bench` times the raster + Sixel-encode
  pipeline against an in-memory backend, for perf regressions.
- For anything `--preview` can't exercise (e.g. real network data,
  terminal-specific behavior), run the real binary in a throwaway
  terminal window and screenshot it — don't assume correctness from
  reading the code alone, especially for anything geometric (rotation
  conventions, label placement, coordinate transforms have all had real
  bugs that only showed up visually).

If you're testing the *installed* `flyover` binary specifically (e.g.
because something else on the system, like Omarchy's screensaver script,
shells out to `flyover` by name rather than a dev build), remember
`cargo install --path .` only happens when you run it — a `cargo build
--release` alone does not update `~/.cargo/bin/flyover`.

## Architecture

Two independent render backends, selected at runtime with `v`:

- `raster.rs` + `sixel_scope.rs` — off-screen rasterization via
  `tiny-skia`, encoded to the terminal via the Sixel graphics protocol
  (`ratatui-image`). Anti-aliased, supports filled shapes, per-category
  icons, and the airport/airspace overlays.
- `braille_scope.rs` — a `ratatui` Canvas using the Braille marker, plain
  terminal text. No image encoding, animates smoothly everywhere, but
  can only draw points/lines, not filled polygons or rotated icons.

**Some features are Sixel-only by design**, not by oversight — check
existing code (`grep` for the feature in both files) before assuming
something applies to both render modes. Aircraft icons and the airport/
airspace overlays are current examples.

`geometry.rs` holds constants and conversions shared by both backends
(e.g. `bearing_to_xy`, sweep timing) — when a value needs to match
between render modes (like the sweep period), it belongs there, not
duplicated in each backend.

`theme.rs` reads Omarchy's live theme colors from
`~/.local/state/omarchy/current/theme/colors.toml`, with a CRT-green
fallback `Palette` for non-Omarchy systems. Aircraft-category and
climb/descent colors are `Palette` methods (`kind_color`,
`climb_rate_color`), not hardcoded per call site.

`data/` holds everything that isn't live aircraft state:
- `fetch.rs` — polls adsb.lol every 10s in a background thread.
- `airports.rs` / `airspace.rs` — one-shot background loads of
  OurAirports/FAA data, cached under `~/.cache/flyover/`. Network calls
  here are best-effort: `geo_cache::ensure_cached` falls back to a stale
  cached copy on a failed refetch rather than losing the feature for one
  offline run, and failures in these loaders should degrade gracefully
  (empty overlay), never crash the app or block the aircraft poll.
- `location.rs` — reads the configured lat/lon from
  `~/.local/state/omarchy/settings/weather.json`.

## Packaging

Three release artifacts, all versioned together from `Cargo.toml`'s
`version` field: crates.io (`cargo publish`), the AUR
(`packaging/aur/PKGBUILD` + `.SRCINFO`, regenerate the latter with
`makepkg --printsrcinfo > .SRCINFO` after any bump), and a git tag
(`vX.Y.Z`, needed before the AUR tarball URL resolves). Bump the version,
commit, tag, push the tag, *then* update the PKGBUILD's `sha256sums`
against the real tagged tarball — not a locally-built one, they won't
match GitHub's archive checksum.
