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
//! mtrack's beat grid, tempo map and metronome rules for the web UI, as a
//! WebAssembly module.
//!
//! The module is the player's own `src/tempo.rs`, `src/config/tempo.rs`,
//! `src/config/metronome.rs` and `src/audio/beat_grid.rs`, compiled as they
//! are, so the UI places measures, beats, signatures and accents exactly
//! where the player does. Nothing here does any arithmetic of its own; each
//! export only carries values across the boundary to one of the player's
//! functions. The TypeScript types of the arguments and results are
//! generated from the Rust structs (`tsify`), so the UI has no copy of them
//! to drift.
//!
//! Every export is stateless and synchronous: the UI hands over the grid or
//! config it holds and gets an answer, with nothing kept on this side. An
//! unanswerable question (a position off the grid, a config that makes no
//! tempo map) is `null`, never a trap.

// Each included source is named by its own `#[path]` here, one per line:
// the Makefile reads this list to know which player sources the web UI's
// build depends on, so it lives nowhere else.
#[path = "../../../src/tempo.rs"]
#[allow(dead_code)]
pub mod tempo;

/// The song's `tempo:` block (`src/config/tempo.rs`).
#[path = "../../../src/config/tempo.rs"]
#[allow(dead_code)]
pub mod tempo_config;

/// The song's `metronome:` block (`src/config/metronome.rs`).
#[path = "../../../src/config/metronome.rs"]
#[allow(dead_code)]
pub mod metronome_config;

/// The player's beat grid (`src/audio/beat_grid.rs`).
#[path = "../../../src/audio/beat_grid.rs"]
#[allow(dead_code)]
pub mod beat_grid;

/// What the included modules' own unit tests reach for in the player
/// (`crate::util::to_yaml_string`). Those tests are never run here (`[lib]
/// test = false`), but `cargo clippy --all-targets` still compiles them, so
/// the name has to resolve: JSON is YAML enough for a compile.
#[cfg(test)]
mod util {
    pub fn to_yaml_string<T: serde::Serialize>(
        value: &T,
    ) -> Result<String, Box<dyn std::error::Error>> {
        Ok(serde_json::to_string(value)?)
    }
}

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tsify_next::Tsify;
use wasm_bindgen::prelude::wasm_bindgen;

pub use beat_grid::BeatGrid;
pub use metronome_config::MetronomeConfig;
use tempo::TempoMap;
pub use tempo_config::TempoConfig;

// ── Beat grid ───────────────────────────────────────────────────────────────

/// A question to a beat grid: where a position is, or what position a time
/// is nearest.
#[derive(Deserialize, Tsify, Clone, Copy, Debug, PartialEq)]
#[tsify(from_wasm_abi)]
#[serde(untagged)]
pub enum GridQuery {
    /// A measure (1-based) and a possibly fractional beat (1-based).
    Position { measure: u32, beat: f64 },
    /// A time in seconds.
    Time { secs: f64 },
}

/// A position on a beat grid, with everything the UI asks about it.
#[derive(Serialize, Tsify, Clone, Copy, Debug, PartialEq)]
#[tsify(into_wasm_abi)]
pub struct GridPosition {
    /// The measure, 1-based.
    pub measure: u32,
    /// The beat within the measure, 1-based; whole for a time query.
    pub beat: f64,
    /// The time of the position in seconds.
    pub secs: f64,
    /// How many beats the measure holds.
    pub beats_in_measure: u32,
}

/// Answers a [`GridQuery`] with [`BeatGrid::beat_time`],
/// [`BeatGrid::position_at`] and [`BeatGrid::beats_in_measure`]; `None` for
/// a position the grid does not hold (off its end, past its measure's last
/// beat, before measure 1).
pub fn grid_position(grid: &BeatGrid, query: GridQuery) -> Option<GridPosition> {
    let (measure, beat) = match query {
        GridQuery::Position { measure, beat } => (measure.checked_sub(1)? as usize, beat),
        GridQuery::Time { secs } => {
            let (measure, beat) = grid.position_at(secs)?;
            (measure, beat as f64)
        }
    };
    let secs = grid.beat_time(measure, beat)?;
    Some(GridPosition {
        measure: u32::try_from(measure + 1).ok()?,
        beat,
        secs,
        beats_in_measure: u32::try_from(grid.beats_in_measure(measure)?).ok()?,
    })
}

/// [`grid_position`], for the web UI.
#[wasm_bindgen]
pub fn grid_query(grid: BeatGrid, query: GridQuery) -> Option<GridPosition> {
    grid_position(&grid, query)
}

// ── Tempo map ───────────────────────────────────────────────────────────────

/// Where a tempo map comes from. A song's `tempo:` block today; a light
/// show's `tempo {}` block is a later variant.
#[derive(Deserialize, Tsify, Clone, Debug, PartialEq)]
#[tsify(from_wasm_abi)]
#[serde(untagged)]
pub enum TempoSource {
    /// The song's `tempo:` block.
    Config { config: TempoConfig },
}

impl TempoSource {
    /// The tempo map the source makes, or why it makes none.
    pub fn to_tempo_map(&self) -> Result<TempoMap, String> {
        match self {
            TempoSource::Config { config } => config.to_tempo_map(),
        }
    }
}

/// A question to a tempo map: what is at a time, or where a position is.
#[derive(Deserialize, Tsify, Clone, Copy, Debug, PartialEq)]
#[tsify(from_wasm_abi)]
#[serde(untagged)]
pub enum TempoQuery {
    /// A time in milliseconds of song time.
    Time { ms: f64 },
    /// A measure (1-based) and a possibly fractional beat (1-based, in the
    /// map's quarter-note units).
    Position { measure: u32, beat: f64 },
}

/// A point on a tempo map: its time, its position, and the tempo and
/// signature in force there.
#[derive(Serialize, Tsify, Clone, Debug, PartialEq)]
#[tsify(into_wasm_abi)]
pub struct TempoPoint {
    /// Song time in milliseconds.
    pub ms: f64,
    /// The measure, 1-based.
    pub measure: u32,
    /// The beat within the measure, 1-based.
    pub beat: f64,
    /// The position as reports show it: `@3/1.00`.
    pub label: String,
    /// The tempo at that time, in quarter-note beats per minute.
    pub bpm: f64,
    /// The time signature of the bar the point is in, as `[numerator,
    /// denominator]`.
    pub sig: (u32, u32),
}

/// Answers a [`TempoQuery`] with the player's `TempoMap`; `None` when the
/// source makes no tempo map (see [`tempo_verdict`]) or the position lies
/// before measure 1.
pub fn tempo_point(source: &TempoSource, query: TempoQuery) -> Option<TempoPoint> {
    let map = source.to_tempo_map().ok()?;
    let (at, measure, beat, label, sig) = match query {
        TempoQuery::Time { ms } => {
            if !ms.is_finite() || ms < 0.0 {
                return None;
            }
            let at = Duration::from_secs_f64(ms / 1000.0);
            let (measure, beat) = map.time_to_measure_beat(at);
            let sig = map.time_signature_at_time(at, 0.0);
            (at, measure, beat, map.position_label(at), sig)
        }
        TempoQuery::Position { measure, beat } => {
            if !beat.is_finite() {
                return None;
            }
            let at = map.measure_to_time_with_offset(measure, beat, 0, 0.0)?;
            let sig = map.time_signature_at_measure(measure);
            (
                at,
                measure,
                beat,
                tempo::position_label_for(measure, beat),
                sig,
            )
        }
    };
    Some(TempoPoint {
        ms: at.as_secs_f64() * 1000.0,
        measure,
        beat,
        label,
        bpm: map.bpm_at_time(at, 0.0),
        sig: (sig.numerator, sig.denominator),
    })
}

/// [`tempo_point`], for the web UI.
#[wasm_bindgen]
pub fn tempo_query(source: TempoSource, query: TempoQuery) -> Option<TempoPoint> {
    tempo_point(&source, query)
}

/// Why a tempo config makes no tempo map (`TempoConfig::to_tempo_map`'s
/// verdict, the one a save gets), or `None` when it does.
pub fn tempo_verdict(config: &TempoConfig) -> Option<String> {
    config.to_tempo_map().err()
}

/// [`tempo_verdict`], for the web UI.
#[wasm_bindgen]
pub fn tempo_validate(config: TempoConfig) -> Option<String> {
    tempo_verdict(&config)
}

/// A tempo config put in the order `TempoConfig::to_tempo_map` requires,
/// and how many changes that cost.
#[derive(Serialize, Tsify, Clone, Debug, PartialEq)]
#[tsify(into_wasm_abi)]
pub struct NormalizedTempo {
    /// The config with its changes in bar order and each position held
    /// once.
    pub config: TempoConfig,
    /// How many changes were dropped for repeating another's position.
    pub duplicates: u32,
}

/// Puts a tempo config's changes in the bar order `to_tempo_map` requires
/// and keeps one change per position: the last, since `TempoMap::new`
/// applies changes in order and that is the one that was in effect. An
/// editor that adds a change out of turn, or an import whose changes share
/// a position, goes through here before a save.
pub fn tempo_normalized(config: &TempoConfig) -> NormalizedTempo {
    let mut changes = config.changes.clone();
    changes.sort_by(|a, b| {
        tempo::measure_beat_order(
            (a.measure, a.beat.unwrap_or(1.0)),
            (b.measure, b.beat.unwrap_or(1.0)),
        )
    });
    let before = changes.len();
    let mut kept: Vec<tempo_config::TempoChangeConfig> = Vec::with_capacity(before);
    for change in changes {
        let position = (change.measure, change.beat.unwrap_or(1.0));
        if let Some(last) = kept.last() {
            if (last.measure, last.beat.unwrap_or(1.0)) == position {
                kept.pop();
            }
        }
        kept.push(change);
    }
    NormalizedTempo {
        duplicates: (before - kept.len()) as u32,
        config: TempoConfig {
            changes: kept,
            ..config.clone()
        },
    }
}

/// [`tempo_normalized`], for the web UI.
#[wasm_bindgen]
pub fn tempo_normalize(config: TempoConfig) -> NormalizedTempo {
    tempo_normalized(&config)
}

/// A time signature as `[numerator, denominator]`.
#[derive(Serialize, Tsify, Clone, Copy, Debug, PartialEq)]
#[tsify(into_wasm_abi)]
#[serde(transparent)]
pub struct Signature(pub (u32, u32));

/// A `4/4`-style string as `[numerator, denominator]`, by the rule a song's
/// `tempo:` block parses it (`parse_time_signature`); `None` when it is not
/// a signature the player accepts.
pub fn signature(raw: &str) -> Option<Signature> {
    tempo_config::parse_time_signature(raw)
        .ok()
        .map(|sig| Signature((sig.numerator, sig.denominator)))
}

/// [`signature`], for the web UI.
#[wasm_bindgen]
pub fn signature_parse(raw: &str) -> Option<Signature> {
    signature(raw)
}

// ── Metronome ───────────────────────────────────────────────────────────────

/// Per-beat accent levels: 0 silent, 1 normal, 2 half, 3 accent.
#[derive(Serialize, Tsify, Clone, Debug, PartialEq)]
#[tsify(into_wasm_abi)]
#[serde(transparent)]
pub struct AccentPattern(pub Vec<u8>);

/// The accent pattern a measure (1-based) of `beats` beats plays, by
/// `MetronomeConfig::accent_pattern_at`: the rule the click renderer applies
/// bar by bar.
#[wasm_bindgen]
pub fn accent_pattern(metronome: MetronomeConfig, measure: u32, beats: u32) -> AccentPattern {
    AccentPattern(metronome.accent_pattern_at(measure, beats as usize))
}
