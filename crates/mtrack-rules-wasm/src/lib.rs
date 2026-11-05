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
//! mtrack's beat grid, tempo map, metronome and DMX patch rules for the web
//! UI, as a WebAssembly module.
//!
//! The module is the player's own `src/tempo.rs`, `src/config/tempo.rs`,
//! `src/config/metronome.rs`, `src/audio/beat_grid.rs` and
//! `src/lighting/patch.rs`, compiled as they are, so the UI places measures,
//! beats, signatures and accents exactly where the player does, and refuses
//! the patch overlaps the player refuses. No rule lives here: each export
//! carries values across the boundary to one of the player's functions.
//! What little logic it holds is the boundary's own -- reading the page's
//! values leniently, refusing magnitudes no song has (see [`bounds`]),
//! milliseconds to and from seconds, which of the player's two signature
//! lookups a tempo point reports (the bar's, by its time, and the one the
//! changes have set), and the beats-in-measure of a count-in. The
//! TypeScript types of the arguments and results are generated from the
//! Rust structs (`tsify`), so the UI has no copy of them to drift.
//!
//! Every export is stateless and synchronous: the UI hands over the grid or
//! config it holds and gets an answer, with nothing kept on this side. An
//! unanswerable question (a position off the grid, a config that makes no
//! tempo map, a value that is not what its type says -- a measure of 1.5,
//! a DMX address of -1) is `null`, never a trap.

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

/// The player's DMX patch rules (`src/lighting/patch.rs`): which fixtures
/// overlap, which gang, which run past the universe.
#[path = "../../../src/lighting/patch.rs"]
#[allow(dead_code)]
pub mod patch;

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
pub use patch::{PatchOverlap, PatchOverrun, PatchSpan};
use tempo::TempoMap;
pub use tempo_config::TempoConfig;

/// Reads a value the page handed over. One that is not what its type says
/// (a measure of 1.5 or -1, a bpm that is not a number, a list where an
/// object should be) is an error here, never a trap: the export answers
/// `None`, or [`tempo_validate`] says the value is not a tempo config.
fn read<T: DeserializeOwned>(value: JsValue) -> Option<T> {
    serde_wasm_bindgen::from_value(value).ok()
}

/// The magnitudes past which a question is refused rather than asked: no
/// song is longer than [`bounds::MAX_SECS`] (about 31 years), has more than
/// [`bounds::MAX_MEASURES`] bars, or counts beats past [`bounds::MAX_BEAT`];
/// a `tempo:` block's start, measures, beats and transition lengths are
/// held to the same. Past them the player's own functions would walk bar
/// by bar for minutes, or turn a time into a `Duration` that cannot hold
/// it, so the answer is `None` (or the verdict) before they are reached.
pub mod bounds {
    pub const MAX_SECS: f64 = 1.0e9;
    pub const MAX_MEASURES: u32 = 1_000_000;
    pub const MAX_BEAT: f64 = 1.0e6;
    /// The most beats an accent pattern is asked for at once.
    pub const MAX_PATTERN_BEATS: u32 = 64;

    pub fn secs(secs: f64) -> bool {
        secs.is_finite() && secs.abs() <= MAX_SECS
    }

    pub fn beat(beat: f64) -> bool {
        beat.is_finite() && beat.abs() <= MAX_BEAT
    }

    /// Why a tempo config is beyond any song, or `None` when it is within
    /// bounds (which says nothing about whether it makes a map).
    pub fn tempo_config(config: &crate::TempoConfig) -> Option<String> {
        if let Some(start) = config.start {
            if !secs(start) {
                return Some(format!("tempo start {start:e} is beyond any song"));
            }
        }
        for change in &config.changes {
            if change.measure > MAX_MEASURES {
                return Some(format!(
                    "tempo change at measure {} is beyond any song",
                    change.measure
                ));
            }
            if change.beat.is_some_and(|b| !beat(b)) {
                return Some(format!(
                    "tempo change at measure {} has a beat beyond any bar",
                    change.measure
                ));
            }
            let length = match change.transition {
                Some(crate::tempo_config::TransitionConfig::Beats(n))
                | Some(crate::tempo_config::TransitionConfig::Measures(n)) => n,
                None => 0.0,
            };
            if !beat(length) {
                return Some(format!(
                    "tempo change at measure {} has a transition beyond any song",
                    change.measure
                ));
            }
        }
        None
    }

    /// Whether a grid's times are all within bounds.
    pub fn grid(grid: &crate::BeatGrid) -> bool {
        grid.beats.iter().all(|b| secs(*b))
    }
}

type BeatGridJs = <BeatGrid as Tsify>::JsType;
type GridQueryJs = <GridQuery as Tsify>::JsType;
type TempoSourceJs = <TempoSource as Tsify>::JsType;
type TempoQueryJs = <TempoQuery as Tsify>::JsType;
type TempoConfigJs = <TempoConfig as Tsify>::JsType;
type MetronomeConfigJs = <MetronomeConfig as Tsify>::JsType;
type PatchSpanJs = <PatchSpan as Tsify>::JsType;
type PatchSpansJs = <PatchSpans as Tsify>::JsType;

// ── Beat grid ───────────────────────────────────────────────────────────────

/// How a position snaps to a beat: to the nearest `step` of a beat (1 for
/// whole beats, 0.5 for halves), rolling into the next measure when the
/// rounding leaves this one -- or, `within_bar`, staying at the highest
/// step this measure holds (`BeatGrid::snap`).
#[derive(Deserialize, Tsify, Clone, Copy, Debug, PartialEq)]
pub struct Snap {
    pub step: f64,
    #[serde(default)]
    #[tsify(optional)]
    pub within_bar: bool,
}

/// A question to a beat grid: where a position is, or what position a time
/// is at; either snapped to a beat on request.
#[derive(Deserialize, Tsify, Clone, Copy, Debug, PartialEq)]
#[serde(untagged)]
pub enum GridQuery {
    /// A measure (1-based; 0 the count-in before bar 1) and a possibly
    /// fractional beat (1-based).
    Position {
        measure: u32,
        beat: f64,
        #[serde(default)]
        #[tsify(optional)]
        snap: Option<Snap>,
    },
    /// A time in seconds: its fractional position, or snapped.
    Time {
        secs: f64,
        #[serde(default)]
        #[tsify(optional)]
        snap: Option<Snap>,
    },
}

/// A position on a beat grid, with everything the UI asks about it.
#[derive(Serialize, Tsify, Clone, Copy, Debug, PartialEq)]
#[tsify(into_wasm_abi)]
pub struct GridPosition {
    /// The measure, 1-based; 0 for the count-in before bar 1, whose clicks
    /// the click track plays but no section or marker can name.
    pub measure: u32,
    /// The beat within the measure (click within the count-in), 1-based;
    /// fractional for a time query without `snap`.
    pub beat: f64,
    /// The time of the position in seconds.
    pub secs: f64,
    /// How many beats the measure holds (clicks, for the count-in).
    pub beats_in_measure: u32,
    /// The index into `beats` of the whole beat the position is on (the one
    /// a fractional beat is counted from): what the accent levels the
    /// backend sends are indexed by.
    pub beat_index: u32,
}

impl GridQuery {
    /// The grid's answer, by `BeatGrid::point`, `position_at`, `snap`,
    /// `time_of` and `beats_in`; `None` for a position the grid does not
    /// hold (off its end, past its measure's last beat) or a question
    /// beyond [`bounds`].
    pub fn answer(&self, grid: &BeatGrid) -> Option<GridPosition> {
        if !bounds::grid(grid) {
            return None;
        }
        let in_bounds = |snap: Option<Snap>| {
            snap.is_none_or(|s| s.step.is_finite() && s.step > 0.0 && s.step <= bounds::MAX_BEAT)
        };
        let point = match *self {
            GridQuery::Position {
                measure,
                beat,
                snap,
            } => {
                if measure > bounds::MAX_MEASURES || !bounds::beat(beat) || !in_bounds(snap) {
                    return None;
                }
                match snap {
                    None => grid.point(measure as usize, beat)?,
                    Some(snap) => grid.snap(measure as usize, beat, snap.step, snap.within_bar)?,
                }
            }
            GridQuery::Time { secs, snap } => {
                if !bounds::secs(secs) || !in_bounds(snap) {
                    return None;
                }
                match snap {
                    None => grid.position_at(secs)?,
                    Some(snap) => grid.snapped_position_at(secs, snap.step, snap.within_bar)?,
                }
            }
        };
        Some(GridPosition {
            measure: u32::try_from(point.measure).ok()?,
            beat: point.beat,
            secs: grid.time_of(&point)?,
            beats_in_measure: u32::try_from(grid.beats_in(point.measure)?).ok()?,
            beat_index: u32::try_from(point.index).ok()?,
        })
    }
}

/// [`GridQuery::answer`], for the web UI.
#[wasm_bindgen]
pub fn grid_query(grid: BeatGridJs, query: GridQueryJs) -> Option<GridPosition> {
    let grid: BeatGrid = read(JsValue::from(grid))?;
    let query: GridQuery = read(JsValue::from(query))?;
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
    /// The tempo map the source makes, or why it makes none -- including a
    /// source beyond [`bounds`], which is never handed to the player.
    pub fn to_tempo_map(&self) -> Result<TempoMap, String> {
        match self {
            TempoSource::Config { config } => match bounds::tempo_config(config) {
                Some(beyond) => Err(beyond),
                None => config.to_tempo_map(),
            },
            TempoSource::Grid { grid } => {
                if !bounds::grid(grid) {
                    return Err("the beat grid's times are beyond any song".to_string());
                }
                grid.to_tempo_map()
                    .ok_or_else(|| "the beat grid holds no complete measure".to_string())
            }
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
    /// makes no tempo map (see [`tempo_validate`]), the position lies
    /// before measure 1, or the question is beyond [`bounds`].
    pub fn answer(&self, source: &TempoSource) -> Option<TempoPoint> {
        let map = source.to_tempo_map().ok()?;
        let (at, measure, beat, label, sig) = match *self {
            TempoQuery::Time { ms } => {
                if ms < 0.0 || !bounds::secs(ms / 1000.0) {
                    return None;
                }
                let at = Duration::from_secs_f64(ms / 1000.0);
                let (measure, beat) = map.time_to_measure_beat(at);
                let sig = map.time_signature_at_time(at, 0.0);
                (at, measure, beat, map.position_label(at), sig)
            }
            TempoQuery::Position { measure, beat } => {
                if measure > bounds::MAX_MEASURES || !bounds::beat(beat) {
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
    let source: TempoSource = read(JsValue::from(source))?;
    let query: TempoQuery = read(JsValue::from(query))?;
    query.answer(&source)
}

/// Why a tempo config cannot be saved (`TempoConfig::validate`, the verdict
/// a save gets; a value beyond [`bounds`] or not a tempo config at all is
/// said so), or `None` when it can.
#[wasm_bindgen]
pub fn tempo_validate(config: TempoConfigJs) -> Option<String> {
    match read::<TempoConfig>(JsValue::from(config)) {
        Some(config) => bounds::tempo_config(&config).or_else(|| config.validate().err()),
        None => Some("not a tempo config".to_string()),
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
    let config: TempoConfig = read(JsValue::from(config))?;
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
/// a signature the player accepts, or not a string.
#[wasm_bindgen]
pub fn signature_parse(raw: Option<String>) -> Option<Signature> {
    tempo_config::parse_time_signature(&raw?)
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
/// bar by bar. `None` for a value that is not a metronome config, or a
/// measure or beat count beyond [`bounds`].
#[wasm_bindgen]
pub fn accent_pattern(
    metronome: MetronomeConfigJs,
    measure: u32,
    beats: u32,
) -> Option<AccentPattern> {
    if measure > bounds::MAX_MEASURES || beats > bounds::MAX_PATTERN_BEATS {
        return None;
    }
    let metronome: MetronomeConfig = read(JsValue::from(metronome))?;
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
    if measure > bounds::MAX_MEASURES {
        return None;
    }
    let metronome: MetronomeConfig = read(JsValue::from(metronome))?;
    Some(metronome.subdivision_at(measure).0.clone())
}

// ── DMX patch ───────────────────────────────────────────────────────────────

/// The spans of a venue's fixtures, as the UI holds them.
#[derive(Deserialize, Tsify, Clone, Debug, PartialEq)]
#[serde(transparent)]
pub struct PatchSpans(pub Vec<PatchSpan>);

/// The gangs a candidate patch would partially overlap.
#[derive(Serialize, Tsify, Clone, Debug, PartialEq)]
#[tsify(into_wasm_abi)]
#[serde(transparent)]
pub struct PatchOverlaps(pub Vec<PatchOverlap>);

/// The spans that run past the end of their universe.
#[derive(Serialize, Tsify, Clone, Debug, PartialEq)]
#[tsify(into_wasm_abi)]
#[serde(transparent)]
pub struct PatchOverruns(pub Vec<PatchOverrun>);

/// `patch::overlaps_with`, for the web UI: the gangs of `spans` a candidate
/// address or mode would partially overlap, by address then name -- the
/// rule the loader and the readiness report apply. A span with the
/// candidate's name is the fixture being moved and is skipped; spans
/// identical to the candidate's gang with it. `None` for a value that is
/// not a list of spans (an address or footprint that is not a whole number
/// in 0..=65535).
#[wasm_bindgen]
pub fn patch_overlaps(spans: PatchSpansJs, candidate: PatchSpanJs) -> Option<PatchOverlaps> {
    let spans: PatchSpans = read(JsValue::from(spans))?;
    let candidate: PatchSpan = read(JsValue::from(candidate))?;
    Some(PatchOverlaps(patch::overlaps_with(&spans.0, &candidate)))
}

/// `PatchSpan::last`, for the web UI: the last address a span uses,
/// inclusive; `None` for a zero footprint or a value that is not a span.
#[wasm_bindgen]
pub fn patch_last(span: PatchSpanJs) -> Option<u32> {
    read::<PatchSpan>(JsValue::from(span))?.last()
}

/// `patch::venue_overruns`, for the web UI: every span that runs past
/// address 512, by universe, address, name. `None` for a value that is not
/// a list of spans.
#[wasm_bindgen]
pub fn patch_overruns(spans: PatchSpansJs) -> Option<PatchOverruns> {
    let spans: PatchSpans = read(JsValue::from(spans))?;
    Some(PatchOverruns(patch::venue_overruns(&spans.0)))
}
