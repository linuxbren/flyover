use super::geo_cache::{ensure_cached, haversine_nm, initial_bearing_deg};
use serde::Deserialize;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// The FAA's own hosted ArcGIS Feature Service for Class Airspace — found by
/// resolving the "Airspace Boundary" ArcGIS Open Data item to its
/// underlying service URL (`.../sharing/rest/content/items/<id>?f=json`);
/// the friendlier Open Data landing page is JS-rendered and has no plain
/// download link. Published every 8 weeks per the FAA's own item
/// description, so this is about as "static" a live-fetched dataset gets.
///
/// `maxAllowableOffset`/`geometryPrecision` ask the server to generalize
/// the geometry before sending it — real exported rings run into the
/// hundreds of vertices (ArcGIS densifies the charted arcs into many short
/// segments), far more precision than a dim backdrop outline needs.
/// Measured: ~9.9MB/467 max vertices per ring without these params, ~480KB/
/// ~20 vertices average with them — a simplification the server does far
/// better than a naive "keep every Nth point" ever could.
const CLASS_BC_URL: &str = "https://services6.arcgis.com/ssFJjBXIUyZDrSYZ/arcgis/rest/services/Class_Airspace/FeatureServer/0/query?where=CLASS=%27B%27+OR+CLASS=%27C%27&outFields=NAME,CLASS,IDENT&outSR=4326&geometryPrecision=5&maxAllowableOffset=0.003&f=geojson&returnGeometry=true";

/// Same reasoning as `airports::CACHE_MAX_AGE` — refreshed far less often
/// than the FAA actually republishes, no manual cache-busting needed.
const CACHE_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Class B rings run larger than a typical runway spread — a bit more
/// headroom than `airports::LOAD_RADIUS_NM`.
const LOAD_RADIUS_NM: f64 = 150.0;

/// How close a Class D/E airport has to be to count as "local" for
/// `load_local_class_de_idents` — see that function's doc comment.
const LOCAL_CLASS_DE_RADIUS_NM: f64 = 10.0;

/// One Class B boundary's outermost ring, already projected into the same
/// (distance, bearing)-from-home polar form used everywhere else in this
/// app, as a closed loop (drawing code connects the last point back to the
/// first). See `airports::RunwaySegment` for why this polar form rather
/// than raw lat/lon.
pub struct AirspaceBoundary {
    pub points: Vec<(f64, f64)>,
}

#[derive(Deserialize)]
struct FeatureCollection {
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    properties: Properties,
    geometry: Geometry,
}

#[derive(Deserialize)]
struct Properties {
    #[serde(rename = "NAME")]
    name: Option<String>,
    #[serde(rename = "CLASS")]
    class: Option<String>,
    #[serde(rename = "IDENT")]
    ident: Option<String>,
}

#[derive(Deserialize)]
struct Geometry {
    #[serde(rename = "type")]
    kind: String,
    coordinates: serde_json::Value,
}

/// Both `Polygon` and `MultiPolygon` GeoJSON nest coordinates differently
/// (rings vs. polygons-of-rings) — this pulls just the first (outer) ring
/// either way. Every Class B/C feature observed from this service is a
/// plain `Polygon`; `MultiPolygon` handling is defensive, not exercised.
fn outer_ring(geom: &Geometry) -> Option<Vec<(f64, f64)>> {
    let to_points = |ring: &serde_json::Value| -> Option<Vec<(f64, f64)>> {
        ring.as_array()?
            .iter()
            .map(|p| {
                let p = p.as_array()?;
                Some((p.first()?.as_f64()?, p.get(1)?.as_f64()?))
            })
            .collect()
    };
    match geom.kind.as_str() {
        "Polygon" => to_points(geom.coordinates.as_array()?.first()?),
        "MultiPolygon" => to_points(geom.coordinates.as_array()?.first()?.as_array()?.first()?),
        _ => None,
    }
}

fn shoelace_area(points: &[(f64, f64)]) -> f64 {
    let n = points.len();
    let mut sum = 0.0;
    for i in 0..n {
        let (x1, y1) = points[i];
        let (x2, y2) = points[(i + 1) % n];
        sum += x1 * y2 - x2 * y1;
    }
    (sum / 2.0).abs()
}

fn load_features() -> Result<Vec<Feature>, String> {
    let path = ensure_cached(CLASS_BC_URL, "class_bc_airspace.geojson", CACHE_MAX_AGE)?;
    let bytes =
        std::fs::read(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let fc: FeatureCollection =
        serde_json::from_slice(&bytes).map_err(|e| format!("could not parse airspace geojson: {e}"))?;
    Ok(fc.features)
}

/// Class B boundaries only, per explicit scope — Class C areas are smaller
/// and less visually distinct, so they're used only for the airport ident
/// allow-list (`load_towered_idents`), not drawn. Each airport can have
/// several stacked shelf rings (different floor/ceiling); this keeps only
/// the one with the largest lateral extent (by raw lon/lat shoelace area,
/// a fine proxy for relative comparison at this scale) — the outer edge is
/// what a lateral-only silhouette should show.
pub fn load_nearby(lat: f64, lon: f64) -> Result<Vec<AirspaceBoundary>, String> {
    let features = load_features()?;

    let mut largest: HashMap<String, (f64, Vec<(f64, f64)>)> = HashMap::new();
    for feature in &features {
        if feature.properties.class.as_deref() != Some("B") {
            continue;
        }
        let Some(name) = &feature.properties.name else { continue };
        let Some(ring) = outer_ring(&feature.geometry) else { continue };
        if ring.len() < 3 {
            continue;
        }
        let area = shoelace_area(&ring);
        let entry = largest.entry(name.clone()).or_insert((0.0, Vec::new()));
        if area > entry.0 {
            *entry = (area, ring);
        }
    }

    let mut boundaries = Vec::new();
    for (_, ring) in largest.into_values() {
        let mut points = Vec::with_capacity(ring.len());
        let mut within_range = false;
        for (lon2, lat2) in ring {
            let dst = haversine_nm(lat, lon, lat2, lon2);
            if dst <= LOAD_RADIUS_NM {
                within_range = true;
            }
            points.push((dst, initial_bearing_deg(lat, lon, lat2, lon2)));
        }
        if within_range {
            boundaries.push(AirspaceBoundary { points });
        }
    }
    Ok(boundaries)
}

/// Class B *and* C airport idents, nationwide, no distance filtering — used
/// by `airports::load_airport_idents` as the "does this airport get a
/// runway silhouette at all" allow-list. See that function's doc comment
/// for why (reducing overall silhouette noise) and for the 3-vs-4-letter
/// ident matching this feeds.
pub fn load_towered_idents() -> Result<HashSet<String>, String> {
    let features = load_features()?;
    Ok(features
        .into_iter()
        .filter_map(|f| f.properties.ident)
        .collect())
}

#[derive(Deserialize)]
struct EsriQueryResponse {
    features: Vec<EsriFeature>,
}

#[derive(Deserialize)]
struct EsriFeature {
    attributes: EsriAttributes,
}

#[derive(Deserialize)]
struct EsriAttributes {
    #[serde(rename = "IDENT")]
    ident: Option<String>,
}

/// Per-feedback exception to the Class B/C-only rule: nationwide B/C alone
/// missed small hometown fields entirely (a real user's own Hays, KS has
/// none), so this adds back any Class D or E airport within
/// `LOCAL_CLASS_DE_RADIUS_NM` of the *configured home location specifically*
/// — unlike B/C, D/E is far too numerous to apply nationwide without
/// reintroducing the original noise problem, so this stays local by
/// construction (server-side spatial query, not a client-side distance
/// filter over a bulk download).
///
/// Class E in particular includes both surface areas tied to one specific
/// airport (`IDENT` populated, e.g. "HYS" for "HAYS CLASS E2") and broader
/// extension/transition areas that aren't ("HAYS CLASS E5", "KANSAS CLASS
/// E5" both carry `IDENT: null`) — confirmed live against Hays, KS during
/// implementation. Filtering to non-null `IDENT` (`filter_map` below) keeps
/// only the airport-specific ones for free, no `LOCAL_TYPE` string-matching
/// needed.
pub fn load_local_class_de_idents(lat: f64, lon: f64) -> Result<HashSet<String>, String> {
    let url = format!(
        "https://services6.arcgis.com/ssFJjBXIUyZDrSYZ/arcgis/rest/services/Class_Airspace/FeatureServer/0/query?geometry={lon},{lat}&geometryType=esriGeometryPoint&inSR=4326&spatialRel=esriSpatialRelIntersects&distance={LOCAL_CLASS_DE_RADIUS_NM}&units=esriSRUnit_NauticalMile&where=CLASS=%27D%27+OR+CLASS=%27E%27&outFields=IDENT&returnGeometry=false&f=json"
    );
    // Small per-location result — the cache filename embeds the rounded
    // coordinates (rather than one fixed name like the nationwide B/C file)
    // so a different configured location doesn't reuse a stale result, but
    // this still avoids a live fetch on every single launch.
    let filename = format!("local_class_de_{lat:.3}_{lon:.3}.json");
    let path = ensure_cached(&url, &filename, CACHE_MAX_AGE)?;
    let bytes =
        std::fs::read(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let resp: EsriQueryResponse = serde_json::from_slice(&bytes)
        .map_err(|e| format!("could not parse local class D/E response: {e}"))?;
    Ok(resp
        .features
        .into_iter()
        .filter_map(|f| f.attributes.ident)
        .collect())
}

/// Same one-shot-load pattern as `airports::spawn_loader` — this data
/// doesn't change mid-session, so no repeating poll.
pub fn spawn_loader(lat: f64, lon: f64) -> mpsc::Receiver<Result<Vec<AirspaceBoundary>, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(load_nearby(lat, lon));
    });
    rx
}
