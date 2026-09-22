use crate::data::aircraft::Aircraft;
use crate::geometry::bearing_to_xy;
use std::collections::{HashMap, HashSet, VecDeque};

/// Trail length in fetch cycles, not seconds — at the ~10s poll interval this
/// is roughly two minutes of history.
const MAX_TRAIL_POINTS: usize = 12;

#[derive(Default)]
pub struct TrailStore {
    trails: HashMap<String, VecDeque<(f64, f64)>>,
}

impl TrailStore {
    /// Appends each aircraft's current position to its trail and drops
    /// trails for aircraft no longer in the fetched list.
    pub fn update(&mut self, aircraft: &[Aircraft]) {
        let mut seen = HashSet::new();
        for ac in aircraft {
            let (Some(dst), Some(dir)) = (ac.dst, ac.dir) else {
                continue;
            };
            seen.insert(ac.hex.clone());
            let trail = self.trails.entry(ac.hex.clone()).or_default();
            trail.push_back(bearing_to_xy(dst, dir));
            while trail.len() > MAX_TRAIL_POINTS {
                trail.pop_front();
            }
        }
        self.trails.retain(|hex, _| seen.contains(hex));
    }

    pub fn get(&self, hex: &str) -> Option<&VecDeque<(f64, f64)>> {
        self.trails.get(hex)
    }
}
