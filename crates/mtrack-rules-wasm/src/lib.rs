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
//! functions, and the one conversion it makes is between the UI's 1-based
//! measures and the grid's 0-based ones. The TypeScript types of the
//! arguments and results are generated from the Rust structs (`tsify`), so
//! the UI has no copy of them to drift.
//!
//! Every export is stateless and synchronous: the UI hands over the grid or
//! config it holds and gets an answer, with nothing kept on this side. An
//! unanswerable question (a position off the grid, a config that makes no
//! tempo map, a value that is not what its type says -- a measure of 1.5)
//! is `null`, never a trap.

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

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tsify_next::Tsify;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;

pub use beat_grid::BeatGrid;
pub use metronome_config::{MetronomeConfig, Subdivision};
use tempo::TempoMap;
pub use tempo_config::TempoConfig;

/// Reads a value the page handed over. One that is not what its type says
/// (a measure of 1.5 or -1, a bpm that is not a number) is an error here,
/// never a trap: the export answers `None`, or hands the message on as a
/// verdict.
fn read<T: DeserializeOwned>(value: JsValue) -> Result<T, String> {
    serde_wasm_bindgen::from_value(value).map_err(|error| error.to_string())
}

type BeatGridJs = <BeatGrid as Tsify>::JsType;
type GridQueryJs = <GridQuery as Tsify>::JsType;
type TempoSourceJs = <TempoSource as Tsify>::JsType;
type TempoQueryJs = <TempoQuery as Tsify>::JsType;
type TempoConfigJs = <TempoConfig as Tsify>::JsType;
type MetronomeConfigJs = <MetronomeConfig as Tsify>::JsType;

// ── Beat grid ───────────────────────────────────────────────────────────────

/// A question to a beat grid: where a position is, or what position a time
/// is at.
#[derive(Deserialize, Tsify, Clone, Copy, Debug, PartialEq)]
#[serde(untagged)]
pub enum GridQuery {
    /// A measure (1-based) and a possibly fractional beat (1-based).
    Position { measure: u32, beat: f64 },
    /// A time in seconds: its fractional position, or with `snap`, the
    /// nearest beat on that step (1 = whole beats, 0.5 = half beats), as
    /// the player snaps a time to one (`BeatGrid::snapped_position_at`).
    Time {
        secs: f64,
        #[serde(default)]
        #[tsify(optional)]
        snap: Option<f64>,
    },
}

/// A position on a beat grid, with everything the UI asks about it.
#[derive(Serialize, Tsify, Clone, Copy, Debug, PartialEq)]
#[tsify(into_wasm_abi)]
pub struct GridPosition {
    /// The measure, 1-based.
    pub measure: u32,
    /// The beat within the measure, 1-based; fractional for a time query
    /// without `snap`.
    pub beat: f64,
    /// The time of the position in seconds.
    pub secs: f64,
    /// How many beats the measure holds.
    pub beats_in_measure: u32,
    /// The index into `beats` of the whole beat the position is on (the one
    /// a fractional beat is counted from): what the accent levels the
    /// backend sends are indexed by.
    pub beat_index: u32,
}

impl GridQuery {
    /// The grid's answer, by `BeatGrid::beat_time`, `position_at`,
    /// `snapped_position_at` and `beats_in_measure`; `None` for a position
    /// the grid does not hold (off its end, past its measure's last beat,
    /// before measure 1).
    pub fn answer(&self, grid: &BeatGrid) -> Option<GridPosition> {
        let (measure, beat) = match *self {
            GridQuery::Position { measure, beat } => (measure.checked_sub(1)? as usize, beat),
            GridQuery::Time { secs, snap: None } => grid.position_at(secs)?,
            GridQuery::Time {
                secs,
                snap: Some(step),
            } => grid.snapped_position_at(secs, step)?,
        };
        let secs = grid.beat_time(measure, beat)?;
        let beat_index = grid.measure_starts.get(measure)? + (beat - 1.0).floor() as usize;
        Some(GridPosition {
            measure: u32::try_from(measure + 1).ok()?,
            beat,
            secs,
            beats_in_measure: u32::try_from(grid.beats_in_measure(measure)?).ok()?,
            beat_index: u32::try_from(beat_index).ok()?,
        })
    }
}

/// [`GridQuery::answer`], for the web UI.
#[wasm_bindgen]
pub fn grid_query(grid: BeatGridJs, query: GridQueryJs) -> Option<GridPosition> {
    let grid: BeatGrid = read(JsValue::from(grid)).ok()?;
    let query: GridQuery = read(JsValue::from(query)).ok()?;
    query.answer(&grid)
}

// ── Tempo map ───────────────────────────────────────────────────────────────

/// Where a tempo map comes from: a song's `tempo:` block, or the beat grid
/// of a song without one (`BeatGrid::to_tempo_map`, the map the player
/// cues such a song's light show against). A light show's `tempo {}` block
/// is a later variant.
#[derive(Deserialize, Tsify, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum TempoSource {
    /// The song's `tempo:` block.
    Config { config: TempoConfig },
    /// The song's beat grid, from click analysis.
    Grid { grid: BeatGrid },
}

impl TempoSource {
    /// The tempo map the source makes, or why it makes none.
    pub fn to_tempo_map(&self) -> Result<TempoMap, String> {
        match self {
            TempoSource::Config { config } => config.to_tempo_map(),
            TempoSource::Grid { grid } => grid
                .to_tempo_map()
                .ok_or_else(|| "the beat grid holds no complete measure".to_string()),
        }
    }
}

/// A question to a tempo map: what is at a time, or where a position is.
#[derive(Deserialize, Tsify, Clone, Copy, Debug, PartialEq)]
#[serde(untagged)]
pub enum TempoQuery {
    /// A time in milliseconds of song time.
    Time { ms: f64 },
    /// A measure (1-based) and a possibly fractional beat (1-based, in the
    /// map's quarter-note units).
    Position { measure: u32, beat: f64 },
}

/// A point on a tempo map: its time, its position, and the tempo and
/// signatures in force there.
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
    /// The tempo at that time, in quarter-note beats per minute: inside a
    /// gradual transition, the ramp's value there.
    pub bpm: f64,
    /// The time signature of the bar the point is in, as `[numerator,
    /// denominator]`: what the beat grid counts the bar in. A change
    /// anchored mid-bar governs from the next bar.
    pub sig: (u32, u32),
    /// The time signature the changes up to and including this point have
    /// set: for a marker that changes the metre mid-bar, the metre it sets
    /// (what its accent pads are for), where `sig` is still the bar's.
    pub sig_set: (u32, u32),
}

impl TempoQuery {
    /// The map's answer, by the player's `TempoMap`; `None` when the source
    /// makes no tempo map (see [`tempo_validate`]) or the position lies
    /// before measure 1.
    pub fn answer(&self, source: &TempoSource) -> Option<TempoPoint> {
        let map = source.to_tempo_map().ok()?;
        let (at, measure, beat, label, sig) = match *self {
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
                // The bar the point is in, by its time: a beat past its
                // measure's last (beat 5 of a 4/4 bar) is in the next bar.
                let sig = map.time_signature_at_time(at, 0.0);
                (
                    at,
                    measure,
                    beat,
                    tempo::position_label_for(measure, beat),
                    sig,
                )
            }
        };
        let sig_set = map.time_signature_set_at(at);
        Some(TempoPoint {
            ms: at.as_secs_f64() * 1000.0,
            measure,
            beat,
            label,
            bpm: map.bpm_at_time(at, 0.0),
            sig: (sig.numerator, sig.denominator),
            sig_set: (sig_set.numerator, sig_set.denominator),
        })
    }
}

/// [`TempoQuery::answer`], for the web UI.
#[wasm_bindgen]
pub fn tempo_query(source: TempoSourceJs, query: TempoQueryJs) -> Option<TempoPoint> {
    let source: TempoSource = read(JsValue::from(source)).ok()?;
    let query: TempoQuery = read(JsValue::from(query)).ok()?;
    query.answer(&source)
}

/// Why a tempo config cannot be saved (`TempoConfig::validate`, the verdict
/// a save gets -- or, for a value that is not a tempo config at all, what
/// is wrong with it), or `None` when it can.
#[wasm_bindgen]
pub fn tempo_validate(config: TempoConfigJs) -> Option<String> {
    match read::<TempoConfig>(JsValue::from(config)) {
        Ok(config) => config.validate().err(),
        Err(error) => Some(error),
    }
}

/// A tempo config put in the order a save requires, and how many changes
/// that cost.
#[derive(Serialize, Tsify, Clone, Debug, PartialEq)]
#[tsify(into_wasm_abi)]
pub struct NormalizedTempo {
    /// The config with its changes in bar order and each position held
    /// once.
    pub config: TempoConfig,
    /// How many changes were dropped for repeating another's position.
    pub duplicates: u32,
}

/// `TempoConfig::normalized`, for the web UI: the changes in the order
/// `to_tempo_map` requires, the last change at a position winning. `None`
/// for a value that is not a tempo config.
#[wasm_bindgen]
pub fn tempo_normalize(config: TempoConfigJs) -> Option<NormalizedTempo> {
    let config: TempoConfig = read(JsValue::from(config)).ok()?;
    let (config, duplicates) = config.normalized();
    Some(NormalizedTempo {
        config,
        duplicates: u32::try_from(duplicates).ok()?,
    })
}

/// A time signature as `[numerator, denominator]`.
#[derive(Serialize, Tsify, Clone, Copy, Debug, PartialEq)]
#[tsify(into_wasm_abi)]
#[serde(transparent)]
pub struct Signature(pub (u32, u32));

/// A `4/4`-style string as `[numerator, denominator]`, by the rule a song's
/// `tempo:` block parses it (`parse_time_signature`); `None` when it is not
/// a signature the player accepts.
#[wasm_bindgen]
pub fn signature_parse(raw: &str) -> Option<Signature> {
    tempo_config::parse_time_signature(raw)
        .ok()
        .map(|sig| Signature((sig.numerator, sig.denominator)))
}

// ── Metronome ───────────────────────────────────────────────────────────────

/// Per-beat accent levels: 0 silent, 1 normal, 2 half, 3 accent.
#[derive(Serialize, Tsify, Clone, Debug, PartialEq)]
#[tsify(into_wasm_abi)]
#[serde(transparent)]
pub struct AccentPattern(pub Vec<u8>);

/// The accent pattern a measure (1-based) of `beats` beats plays, by
/// `MetronomeConfig::accent_pattern_at`: the rule the click renderer applies
/// bar by bar. `None` for a value that is not a metronome config.
#[wasm_bindgen]
pub fn accent_pattern(
    metronome: MetronomeConfigJs,
    measure: u32,
    beats: u32,
) -> Option<AccentPattern> {
    let metronome: MetronomeConfig = read(JsValue::from(metronome)).ok()?;
    Some(AccentPattern(
        metronome.accent_pattern_at(measure, beats as usize),
    ))
}

/// The subdivision a measure (1-based) plays, by
/// `MetronomeConfig::subdivision_at`: the base one, swapped by the last
/// change at or before the measure. `None` for a value that is not a
/// metronome config.
#[wasm_bindgen]
pub fn subdivision_at(metronome: MetronomeConfigJs, measure: u32) -> Option<Subdivision> {
    let metronome: MetronomeConfig = read(JsValue::from(metronome)).ok()?;
    Some(metronome.subdivision_at(measure).0.clone())
}
