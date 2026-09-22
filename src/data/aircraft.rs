use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer};

#[derive(Debug, Clone)]
pub enum Altitude {
    Feet(i64),
    Ground,
}

impl<'de> Deserialize<'de> for Altitude {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Number(n) => n
                .as_i64()
                .map(Altitude::Feet)
                .ok_or_else(|| DeError::custom("alt_baro number out of range")),
            // adsb.lol (and the underlying readsb/dump1090 format) reports "ground"
            // as a literal string instead of a number for aircraft on the ground.
            serde_json::Value::String(s) if s == "ground" => Ok(Altitude::Ground),
            other => Err(DeError::custom(format!(
                "unexpected alt_baro value: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Aircraft {
    pub hex: String,
    pub flight: Option<String>,
    // Kept for a future route (VRS Standing Data) or real-projection feature
    // — see project-ideas.md — not read by anything yet.
    #[allow(dead_code)]
    pub r: Option<String>,
    /// ICAO aircraft type designator (e.g. "B738", "A21N", "C172", "GLF6").
    /// Used as the tie-breaker in `kind()` for the one case the ADS-B
    /// emitter category can't settle on its own: category A1 covers both a
    /// light single-engine Cessna and a small business jet.
    pub t: Option<String>,
    pub alt_baro: Option<Altitude>,
    pub gs: Option<f64>,
    /// True track over the ground in degrees (0 = north, clockwise) — the
    /// heading the sixel scope's per-aircraft icon rotates to.
    pub track: Option<f64>,
    pub baro_rate: Option<f64>,
    pub geom_rate: Option<f64>,
    pub squawk: Option<String>,
    #[allow(dead_code)]
    pub lat: Option<f64>,
    #[allow(dead_code)]
    pub lon: Option<f64>,
    /// Distance from the query point in nautical miles, precomputed by adsb.lol.
    pub dst: Option<f64>,
    /// Bearing from the query point in degrees, precomputed by adsb.lol.
    pub dir: Option<f64>,
    /// ADS-B emitter category (DO-260 "wake vortex"/emitter set): "A1" light,
    /// "A2" small, "A3" large, "A4" high-vortex large, "A5" heavy, "A6" high
    /// performance, "A7" rotorcraft, "B*"/"C*" balloons/gliders/UAVs/surface
    /// vehicles/etc. Primary signal for `kind()` — far more reliable than
    /// parsing `t`, since it's a small standardized set rather than
    /// free-form type strings.
    pub category: Option<String>,
}

/// Broad shape bucket for the sixel scope's per-aircraft icon. Deliberately
/// coarse — a handful of readable silhouettes, not a lookup covering every
/// ICAO type designator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AircraftKind {
    /// Category A3/A4/A5: the majority of scheduled commercial traffic.
    Airliner,
    /// Category A2, or an A1 whose type code matches a known turboprop.
    Regional,
    /// Category A1 whose type code matches a known business-jet family.
    BusinessJet,
    /// Category A1 that isn't a recognized business jet — the common case
    /// for light single/twin-engine piston and turboprop GA.
    Private,
    /// Category A7.
    Helicopter,
    /// No category (some feeds omit it), or a category this app doesn't
    /// draw a dedicated icon for (gliders, balloons, UAVs, surface
    /// vehicles, military high-performance). Falls back to the plain dot.
    Unknown,
}

// Prefix match on the ICAO type designator, not an exact list — catches
// family variants (e.g. "C560" and "C56X") without enumerating every one.
// Deliberately short: this only has to break the A1 tie between a business
// jet and a light GA aircraft, not classify anything on its own.
const BUSINESS_JET_TYPE_PREFIXES: &[&str] = &[
    "GLF", "CL3", "CL6", "LJ", "FA", "C25", "C56", "C68", "C700", "C750", "PC24", "E50", "E55",
];
const TURBOPROP_TYPE_PREFIXES: &[&str] =
    &["AT4", "AT7", "DH8", "SW4", "B190", "C208", "PC12", "TBM"];

impl Aircraft {
    pub fn callsign(&self) -> &str {
        self.flight.as_deref().unwrap_or(&self.hex).trim()
    }

    pub fn climb_rate(&self) -> Option<f64> {
        self.baro_rate.or(self.geom_rate)
    }

    pub fn is_emergency_squawk(&self) -> bool {
        matches!(
            self.squawk.as_deref(),
            Some("7500") | Some("7600") | Some("7700")
        )
    }

    pub fn kind(&self) -> AircraftKind {
        let type_code = self.t.as_deref().unwrap_or("");
        let is_business_jet = BUSINESS_JET_TYPE_PREFIXES
            .iter()
            .any(|p| type_code.starts_with(p));
        match self.category.as_deref() {
            Some("A3") | Some("A4") | Some("A5") => AircraftKind::Airliner,
            Some("A7") => AircraftKind::Helicopter,
            // A2 ("small", 15,500-75,000 lbs) isn't purely "regional
            // turboprop" the way its DO-260 label suggests — real adsb.lol
            // data tags small business jets (e.g. a Learjet 35) as A2, not
            // A1, presumably because that weight class straddles both. So
            // the business-jet type check has to run for A2 too, not just
            // A1, or those get misclassified as Regional.
            Some(cat @ ("A1" | "A2")) => {
                if is_business_jet {
                    AircraftKind::BusinessJet
                } else if cat == "A1" {
                    AircraftKind::Private
                } else {
                    AircraftKind::Regional
                }
            }
            // No category reported (some feeds omit it) — best-effort off
            // the type code alone, since a blank icon reads worse than an
            // occasionally-wrong one.
            None => {
                if is_business_jet {
                    AircraftKind::BusinessJet
                } else if TURBOPROP_TYPE_PREFIXES
                    .iter()
                    .any(|p| type_code.starts_with(p))
                {
                    AircraftKind::Regional
                } else {
                    AircraftKind::Unknown
                }
            }
            _ => AircraftKind::Unknown,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct StatesResponse {
    pub ac: Vec<Aircraft>,
}
