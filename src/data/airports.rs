use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const AIRPORTS_URL: &str = "https://davidmegginson.github.io/ourairports-data/airports.csv";
const RUNWAYS_URL: &str = "https://davidmegginson.github.io/ourairports-data/runways.csv";

/// Runway layouts change on the order of years, not days — a generous cache
/// window avoids re-downloading ~16MB of CSV on every launch while still
/// picking up eventual changes without any manual cache-busting.
const CACHE_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Beyond flyover's own MAX_ZOOM_NM (100), so zooming all the way out never
/// clips a runway that would otherwise have been in range.
const LOAD_RADIUS_NM: f64 = 120.0;

/// One runway, already projected into the same (distance, bearing)-from-home
/// polar form adsb.lol hands us for aircraft, so drawing code can reuse
/// `geometry::bearing_to_xy` unmodified. Unlike aircraft, OurAirports gives
/// raw lat/lon with no server-side distance/bearing, so this crate computes
/// that itself via haversine at load time (once), not per frame.
pub struct RunwaySegment {
    pub dst_a: f64,
    pub dir_a: f64,
    pub dst_b: f64,
    pub dir_b: f64,
}

fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join(".cache/flyover")
}

/// Returns a local path with reasonably fresh contents, downloading only
/// when the cached copy is missing or stale. Falls back to a stale cached
/// copy if a re-download fails (e.g. offline) rather than losing the
/// feature entirely for one run.
fn ensure_cached(url: &str, filename: &str) -> Result<PathBuf, String> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let path = dir.join(filename);

    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .map(|t| t.elapsed().unwrap_or(Duration::MAX) < CACHE_MAX_AGE)
        .unwrap_or(false);
    if fresh {
        return Ok(path);
    }

    match download(url) {
        Ok(bytes) => {
            std::fs::write(&path, &bytes)
                .map_err(|e| format!("could not write {}: {e}", path.display()))?;
            Ok(path)
        }
        Err(err) => {
            if path.exists() {
                Ok(path)
            } else {
                Err(err)
            }
        }
    }
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let mut response = ureq::get(url)
        .call()
        .map_err(|e| format!("could not download {url}: {e}"))?;
    response
        .body_mut()
        .read_to_vec()
        .map_err(|e| format!("could not read {url} response body: {e}"))
}

const EARTH_RADIUS_NM: f64 = 3440.065;

fn haversine_nm(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (lat1r, lat2r) = (lat1.to_radians(), lat2.to_radians());
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2) + lat1r.cos() * lat2r.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_NM * a.sqrt().clamp(0.0, 1.0).asin()
}

/// adsb.lol's `dir` is 0 = north, clockwise, same convention this produces —
/// see `geometry::bearing_to_xy`'s own doc comment.
fn initial_bearing_deg(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (lat1r, lat2r) = (lat1.to_radians(), lat2.to_radians());
    let dlon = (lon2 - lon1).to_radians();
    let y = dlon.sin() * lat2r.cos();
    let x = lat1r.cos() * lat2r.sin() - lat1r.sin() * lat2r.cos() * dlon.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// Header-name -> column-index lookup, built once per file. Indexing by
/// name (rather than relying on csv's serde support matching struct fields
/// positionally) is more robust to OurAirports adding/reordering columns —
/// both files have far more columns than this app needs.
fn header_index(headers: &csv::StringRecord, name: &str) -> Option<usize> {
    headers.iter().position(|h| h == name)
}

/// Airport idents worth drawing a runway silhouette for — real fixed-wing
/// airports, not heliports/balloonports/seaplane bases (no runway shape to
/// speak of) or closed fields (stale geometry, nothing actually there).
fn load_airport_idents(path: &Path) -> Result<HashSet<String>, String> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|e| format!("could not open {}: {e}", path.display()))?;
    let headers = reader.headers().map_err(|e| e.to_string())?.clone();
    let ident_i = header_index(&headers, "ident").ok_or("airports.csv missing 'ident' column")?;
    let type_i = header_index(&headers, "type").ok_or("airports.csv missing 'type' column")?;

    let mut idents = HashSet::new();
    for record in reader.records() {
        let record = record.map_err(|e| e.to_string())?;
        let kind = record.get(type_i).unwrap_or("");
        if matches!(kind, "large_airport" | "medium_airport" | "small_airport")
            && let Some(ident) = record.get(ident_i)
        {
            idents.insert(ident.to_string());
        }
    }
    Ok(idents)
}

fn load_runways_near(
    path: &Path,
    idents: &HashSet<String>,
    home_lat: f64,
    home_lon: f64,
) -> Result<Vec<RunwaySegment>, String> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|e| format!("could not open {}: {e}", path.display()))?;
    let headers = reader.headers().map_err(|e| e.to_string())?.clone();
    let col = |name: &str| header_index(&headers, name).ok_or(format!("runways.csv missing '{name}' column"));
    let airport_ident_i = col("airport_ident")?;
    let closed_i = col("closed")?;
    let le_lat_i = col("le_latitude_deg")?;
    let le_lon_i = col("le_longitude_deg")?;
    let he_lat_i = col("he_latitude_deg")?;
    let he_lon_i = col("he_longitude_deg")?;

    let mut segments = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| e.to_string())?;
        if record.get(closed_i) == Some("1") {
            continue;
        }
        let Some(ident) = record.get(airport_ident_i) else { continue };
        if !idents.contains(ident) {
            continue;
        }
        let (Some(le_lat), Some(le_lon), Some(he_lat), Some(he_lon)) = (
            record.get(le_lat_i).and_then(|v| v.parse::<f64>().ok()),
            record.get(le_lon_i).and_then(|v| v.parse::<f64>().ok()),
            record.get(he_lat_i).and_then(|v| v.parse::<f64>().ok()),
            record.get(he_lon_i).and_then(|v| v.parse::<f64>().ok()),
        ) else {
            continue;
        };

        let dst_a = haversine_nm(home_lat, home_lon, le_lat, le_lon);
        let dst_b = haversine_nm(home_lat, home_lon, he_lat, he_lon);
        if dst_a > LOAD_RADIUS_NM && dst_b > LOAD_RADIUS_NM {
            continue;
        }
        segments.push(RunwaySegment {
            dst_a,
            dir_a: initial_bearing_deg(home_lat, home_lon, le_lat, le_lon),
            dst_b,
            dir_b: initial_bearing_deg(home_lat, home_lon, he_lat, he_lon),
        });
    }
    Ok(segments)
}

pub fn load_nearby(lat: f64, lon: f64) -> Result<Vec<RunwaySegment>, String> {
    let airports_path = ensure_cached(AIRPORTS_URL, "airports.csv")?;
    let runways_path = ensure_cached(RUNWAYS_URL, "runways.csv")?;
    let idents = load_airport_idents(&airports_path)?;
    load_runways_near(&runways_path, &idents, lat, lon)
}

/// Airport/runway geometry is effectively static, so unlike
/// `data::fetch::spawn_poller` this loads once in the background and the
/// channel is done after its single send — the render loop just drains it
/// opportunistically until it shows up.
pub fn spawn_loader(lat: f64, lon: f64) -> mpsc::Receiver<Result<Vec<RunwaySegment>, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(load_nearby(lat, lon));
    });
    rx
}
