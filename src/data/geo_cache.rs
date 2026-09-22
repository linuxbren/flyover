use std::path::PathBuf;
use std::time::Duration;

fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join(".cache/flyover")
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

/// Returns a local path (under `~/.cache/flyover/`) with reasonably fresh
/// contents, downloading only when the cached copy is missing or older than
/// `max_age`. Falls back to a stale cached copy if a re-download fails (e.g.
/// offline) rather than losing the feature entirely for one run. Shared by
/// every static/reference dataset this app caches (airports, runways,
/// airspace boundaries) — none of them change fast enough to justify a
/// per-launch fetch.
pub fn ensure_cached(url: &str, filename: &str, max_age: Duration) -> Result<PathBuf, String> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let path = dir.join(filename);

    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .map(|t| t.elapsed().unwrap_or(Duration::MAX) < max_age)
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

const EARTH_RADIUS_NM: f64 = 3440.065;

pub fn haversine_nm(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (lat1r, lat2r) = (lat1.to_radians(), lat2.to_radians());
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2) + lat1r.cos() * lat2r.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_NM * a.sqrt().clamp(0.0, 1.0).asin()
}

/// adsb.lol's `dir` is 0 = north, clockwise, same convention this produces —
/// see `geometry::bearing_to_xy`'s own doc comment.
pub fn initial_bearing_deg(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (lat1r, lat2r) = (lat1.to_radians(), lat2.to_radians());
    let dlon = (lon2 - lon1).to_radians();
    let y = dlon.sin() * lat2r.cos();
    let x = lat1r.cos() * lat2r.sin() - lat1r.sin() * lat2r.cos() * dlon.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}
