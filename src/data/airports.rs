use super::geo_cache::{ensure_cached, haversine_nm, initial_bearing_deg};
use std::collections::HashSet;
use std::path::Path;
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

/// Header-name -> column-index lookup, built once per file. Indexing by
/// name (rather than relying on csv's serde support matching struct fields
/// positionally) is more robust to OurAirports adding/reordering columns —
/// both files have far more columns than this app needs.
fn header_index(headers: &csv::StringRecord, name: &str) -> Option<usize> {
    headers.iter().position(|h| h == name)
}

/// Airport idents worth drawing a runway silhouette for. Originally "any
/// real fixed-wing airport" (OurAirports' own large/medium/small `type`),
/// but with airport silhouettes now paired with the Class B/C airspace
/// overlay, that let through far more airports than the airspace context
/// around them — per feedback, narrowed to airports actually inside
/// controlled Class B or C airspace, which is both a tighter and a more
/// meaningful filter (it's "airports with real traffic control", not just
/// "airports big enough to have a paved runway").
///
/// `towered_idents` (from `airspace::load_towered_idents`) is the FAA's own
/// airspace `IDENT` field: for US airports it's the bare 3-letter code
/// (e.g. "LAX"), matching OurAirports' `iata_code` — NOT `ident`, which is
/// the 4-letter ICAO form ("KLAX") the FAA field never carries the leading
/// K on. For the handful of Canadian airports this dataset also includes,
/// `IDENT` is the full 4-letter ICAO code ("CYYZ"), matching OurAirports'
/// `ident` directly. Checking both (plus a K-stripped `ident` as a fallback
/// for rows with no `iata_code`) covers all three shapes without needing to
/// know which country a given row is from up front.
fn load_airport_idents(path: &Path, towered_idents: &HashSet<String>) -> Result<HashSet<String>, String> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|e| format!("could not open {}: {e}", path.display()))?;
    let headers = reader.headers().map_err(|e| e.to_string())?.clone();
    let ident_i = header_index(&headers, "ident").ok_or("airports.csv missing 'ident' column")?;
    let iata_i =
        header_index(&headers, "iata_code").ok_or("airports.csv missing 'iata_code' column")?;

    let mut idents = HashSet::new();
    for record in reader.records() {
        let record = record.map_err(|e| e.to_string())?;
        let Some(ident) = record.get(ident_i) else { continue };
        let iata = record.get(iata_i).unwrap_or("");
        let k_stripped = ident.strip_prefix('K').filter(|_| ident.len() == 4);
        let matched = (!iata.is_empty() && towered_idents.contains(iata))
            || towered_idents.contains(ident)
            || k_stripped.is_some_and(|s| towered_idents.contains(s));
        if matched {
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
    let airports_path = ensure_cached(AIRPORTS_URL, "airports.csv", CACHE_MAX_AGE)?;
    let runways_path = ensure_cached(RUNWAYS_URL, "runways.csv", CACHE_MAX_AGE)?;
    let towered_idents = super::airspace::load_towered_idents()?;
    let idents = load_airport_idents(&airports_path, &towered_idents)?;
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
