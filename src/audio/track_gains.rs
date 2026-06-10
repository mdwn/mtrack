// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//
//! Runtime-adjustable per-output-track gain.
//!
//! Gains are expressed in dB at the API/config level and stored alongside a
//! precomputed linear multiplier for the audio callback. Values are stored as
//! `f32` bit patterns in atomics so the callback can read them lock-free.
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};

use indexmap::IndexMap;
use tracing::warn;

/// Gains at or below this value are treated as -inf (linear 0.0).
pub const MIN_GAIN_DB: f32 = -60.0;
/// Maximum allowed boost.
pub const MAX_GAIN_DB: f32 = 12.0;

/// Clamps a dB gain to the supported range. NaN becomes 0.0 dB (unity).
pub fn clamp_db(db: f32) -> f32 {
    if db.is_nan() {
        return 0.0;
    }
    db.clamp(MIN_GAIN_DB, MAX_GAIN_DB)
}

/// Converts a dB gain to a linear multiplier. At or below `MIN_GAIN_DB` the
/// track is muted (0.0).
pub fn db_to_linear(db: f32) -> f32 {
    if db <= MIN_GAIN_DB {
        0.0
    } else {
        10.0f32.powf(db / 20.0)
    }
}

/// Error returned when setting the gain of a track that has no slot.
#[derive(Debug)]
pub struct UnknownTrackError(pub String);

impl fmt::Display for UnknownTrackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown track '{}'", self.0)
    }
}

impl std::error::Error for UnknownTrackError {}

/// Shared per-output-track gain state.
///
/// Built once at hardware init from the active profile's `track_mappings`
/// and `track_gains`, then shared (via `Arc`) between the player (set/get)
/// and the audio mixer (lock-free reads in the callback).
pub struct TrackGains {
    /// Track name -> slot index.
    slots: HashMap<String, usize>,
    /// Slot index -> track name, in config order (stable for reporting).
    names: Vec<String>,
    /// Gain in dB, stored as f32 bits.
    db_bits: Vec<AtomicU32>,
    /// Linear multiplier, stored as f32 bits. Hot-path reads.
    linear_bits: Vec<AtomicU32>,
}

impl TrackGains {
    /// Builds gain slots from the union of track mapping keys and configured
    /// gain keys. Tracks without a configured gain start at 0.0 dB (unity).
    /// Out-of-range configured values are clamped with a warning.
    pub fn from_config(
        track_mappings: &HashMap<String, Vec<u16>>,
        track_gains: Option<&IndexMap<String, f32>>,
    ) -> Self {
        // Deterministic order: configured gains first (config order), then
        // any remaining mapping keys sorted by name.
        let mut names: Vec<String> = Vec::new();
        if let Some(gains) = track_gains {
            names.extend(gains.keys().cloned());
        }
        let mut mapping_names: Vec<&String> = track_mappings
            .keys()
            .filter(|name| !names.iter().any(|n| n == *name))
            .collect();
        mapping_names.sort();
        names.extend(mapping_names.into_iter().cloned());

        let mut slots = HashMap::with_capacity(names.len());
        let mut db_bits = Vec::with_capacity(names.len());
        let mut linear_bits = Vec::with_capacity(names.len());
        for (slot, name) in names.iter().enumerate() {
            let configured = track_gains.and_then(|gains| gains.get(name)).copied();
            let db = match configured {
                Some(db) => {
                    let clamped = clamp_db(db);
                    if clamped != db {
                        warn!(
                            track = name.as_str(),
                            configured = db,
                            clamped,
                            "track gain out of range, clamping"
                        );
                    }
                    clamped
                }
                None => 0.0,
            };
            slots.insert(name.clone(), slot);
            db_bits.push(AtomicU32::new(db.to_bits()));
            linear_bits.push(AtomicU32::new(db_to_linear(db).to_bits()));
        }

        Self {
            slots,
            names,
            db_bits,
            linear_bits,
        }
    }

    /// Returns the slot index for a track name, if known.
    pub fn slot(&self, track: &str) -> Option<usize> {
        self.slots.get(track).copied()
    }

    /// Number of tracks with gain slots.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Whether there are no gain slots.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Sets the gain of a track in dB, returning the (clamped) applied value.
    pub fn set_db(&self, track: &str, db: f32) -> Result<f32, UnknownTrackError> {
        let slot = self
            .slot(track)
            .ok_or_else(|| UnknownTrackError(track.to_string()))?;
        let clamped = clamp_db(db);
        self.db_bits[slot].store(clamped.to_bits(), Ordering::Relaxed);
        self.linear_bits[slot].store(db_to_linear(clamped).to_bits(), Ordering::Relaxed);
        Ok(clamped)
    }

    /// Gets the gain of a track in dB.
    pub fn get_db(&self, track: &str) -> Option<f32> {
        self.slot(track)
            .map(|slot| f32::from_bits(self.db_bits[slot].load(Ordering::Relaxed)))
    }

    /// Gets the linear multiplier for a slot. Hot path: single relaxed load.
    pub fn linear(&self, slot: usize) -> f32 {
        f32::from_bits(self.linear_bits[slot].load(Ordering::Relaxed))
    }

    /// Snapshots all gains as (name, dB) pairs in slot order.
    pub fn snapshot_db(&self) -> Vec<(String, f32)> {
        self.names
            .iter()
            .enumerate()
            .map(|(slot, name)| {
                (
                    name.clone(),
                    f32::from_bits(self.db_bits[slot].load(Ordering::Relaxed)),
                )
            })
            .collect()
    }

    /// Snapshots gains for persistence, omitting unity (0.0 dB) entries so
    /// the serialized config stays minimal.
    pub fn snapshot_db_map(&self) -> IndexMap<String, f32> {
        self.snapshot_db()
            .into_iter()
            .filter(|(_, db)| *db != 0.0)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mappings(names: &[&str]) -> HashMap<String, Vec<u16>> {
        names
            .iter()
            .enumerate()
            .map(|(i, name)| (name.to_string(), vec![i as u16 + 1]))
            .collect()
    }

    #[test]
    fn db_to_linear_conversions() {
        assert_eq!(db_to_linear(0.0), 1.0);
        assert!((db_to_linear(-6.0) - 0.5012).abs() < 0.001);
        assert_eq!(db_to_linear(-60.0), 0.0);
        assert_eq!(db_to_linear(-80.0), 0.0);
        assert!((db_to_linear(6.0) - 1.9953).abs() < 0.001);
    }

    #[test]
    fn clamp_db_range() {
        assert_eq!(clamp_db(0.0), 0.0);
        assert_eq!(clamp_db(20.0), MAX_GAIN_DB);
        assert_eq!(clamp_db(-100.0), MIN_GAIN_DB);
        assert_eq!(clamp_db(f32::NAN), 0.0);
        assert_eq!(clamp_db(f32::INFINITY), MAX_GAIN_DB);
        assert_eq!(clamp_db(f32::NEG_INFINITY), MIN_GAIN_DB);
    }

    #[test]
    fn set_get_round_trip() {
        let gains = TrackGains::from_config(&mappings(&["click", "keys"]), None);
        assert_eq!(gains.get_db("click"), Some(0.0));

        let applied = gains.set_db("click", -6.0).unwrap();
        assert_eq!(applied, -6.0);
        assert_eq!(gains.get_db("click"), Some(-6.0));
        let slot = gains.slot("click").unwrap();
        assert!((gains.linear(slot) - 0.5012).abs() < 0.001);

        // Clamped on the way in.
        let applied = gains.set_db("keys", 20.0).unwrap();
        assert_eq!(applied, MAX_GAIN_DB);
        assert_eq!(gains.get_db("keys"), Some(MAX_GAIN_DB));
    }

    #[test]
    fn unknown_track_errors() {
        let gains = TrackGains::from_config(&mappings(&["click"]), None);
        assert!(gains.set_db("nope", 0.0).is_err());
        assert_eq!(gains.get_db("nope"), None);
        assert_eq!(gains.slot("nope"), None);
    }

    #[test]
    fn union_of_mappings_and_configured_gains() {
        let configured: IndexMap<String, f32> =
            IndexMap::from([("click".to_string(), -6.0), ("extra".to_string(), 3.0)]);
        let gains = TrackGains::from_config(&mappings(&["click", "keys"]), Some(&configured));

        assert_eq!(gains.len(), 3);
        assert_eq!(gains.get_db("click"), Some(-6.0));
        assert_eq!(gains.get_db("extra"), Some(3.0));
        assert_eq!(gains.get_db("keys"), Some(0.0));
    }

    #[test]
    fn out_of_range_config_clamped() {
        let configured: IndexMap<String, f32> = IndexMap::from([("click".to_string(), -120.0)]);
        let gains = TrackGains::from_config(&mappings(&["click"]), Some(&configured));
        assert_eq!(gains.get_db("click"), Some(MIN_GAIN_DB));
        assert_eq!(gains.linear(gains.slot("click").unwrap()), 0.0);
    }

    #[test]
    fn snapshot_map_omits_unity() {
        let gains = TrackGains::from_config(&mappings(&["click", "keys", "cue"]), None);
        gains.set_db("click", -6.0).unwrap();
        let map = gains.snapshot_db_map();
        assert_eq!(map.len(), 1);
        assert_eq!(map["click"], -6.0);

        // Full snapshot includes everything.
        assert_eq!(gains.snapshot_db().len(), 3);
    }
}
