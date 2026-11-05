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
//! mtrack's beat grid, tempo map, metronome, DMX patch and light show
//! rules for the web UI, as a WebAssembly module.
//!
//! The module is the player's own `src/tempo.rs`, `src/config/tempo.rs`,
//! `src/config/metronome.rs`, `src/audio/beat_grid.rs`,
//! `src/lighting/patch.rs` and the light show parser (`src/lighting/parser/*`
//! with the effect types it builds), compiled as they are, so the UI places
//! measures, beats, signatures, accents and cues exactly where the player
//! does, refuses the patch overlaps the player refuses, and reads a `.light`
//! file by the one grammar. No rule lives here: each export carries values
//! across the boundary to one of the player's functions. What little logic
//! it holds is the boundary's own -- reading the page's values leniently,
//! refusing magnitudes no song has (see [`bounds`]), milliseconds to and
//! from seconds, which of the player's two signature lookups a tempo point
//! reports (the bar's, by its time, and the one the changes have set), and
//! the beats-in-measure of a count-in. The TypeScript types of the
//! arguments and results are generated from the Rust structs (`tsify`), so
//! the UI has no copy of them to drift; so are the types of what the UI
//! reads from the player's HTTP API and websocket (`src/webui/wire`), which
//! are compiled here for their types alone.
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

/// The player's light show parser, with the effect types it builds: the
/// modules of `src/lighting/effects` and `src/lighting/parser` the parser
/// needs, under the paths they have in the player (`crate::lighting::…`),
/// so each compiles as it is. See `lighting.rs`.
#[allow(dead_code)]
pub mod lighting;

/// A transition's fade as status reports it (`src/config/fade.rs`).
#[path = "../../../src/config/fade.rs"]
mod config_fade;

/// A song's pilot hints (`src/config/pilot.rs`).
#[path = "../../../src/config/pilot.rs"]
#[allow(dead_code)]
pub mod pilot_config;

/// A song's sections (`src/config/section.rs`).
#[path = "../../../src/config/section.rs"]
mod config_section;

/// What the audio output is doing (`src/audio/output_status.rs`).
#[path = "../../../src/audio/output_status.rs"]
pub mod output_status;

/// The hardware and controllers as status reports them
/// (`src/player/hardware_status.rs`).
#[path = "../../../src/player/hardware_status.rs"]
mod hardware_status;

/// The audio and MIDI devices and the trigger calibration as the web UI is
/// told of them (`src/device_info.rs`).
#[path = "../../../src/device_info.rs"]
pub mod device_info;

/// A tempo map guessed for a song (`src/audio/guessed_tempo.rs`).
#[path = "../../../src/audio/guessed_tempo.rs"]
pub mod guessed_tempo;

/// Ambient light's mode (`src/dmx/engine/ambient_mode.rs`).
#[path = "../../../src/dmx/engine/ambient_mode.rs"]
mod ambient_mode;

/// Whose ambient settings are up (`src/playlist/ambient_source.rs`).
#[path = "../../../src/playlist/ambient_source.rs"]
mod ambient_source;

/// Where a mover points (`src/state/pose.rs`).
#[path = "../../../src/state/pose.rs"]
mod pose;

// What the web UI reads from the player (`src/webui/wire`), whose
// TypeScript types are generated here and nowhere else.

/// The devices API's payloads.
#[path = "../../../src/webui/wire/devices.rs"]
pub mod wire_devices;
/// The fixture test's payloads.
#[path = "../../../src/webui/wire/fixture_test.rs"]
pub mod wire_fixture_test;
/// The fixture type, GDTF and venue listing payloads.
#[path = "../../../src/webui/wire/fixture_types.rs"]
pub mod wire_fixture_types;
/// The websocket's messages.
#[path = "../../../src/webui/wire/messages.rs"]
pub mod wire_messages;
/// A song's shows evaluated offline (the 3D view's Preview mode).
#[path = "../../../src/webui/wire/preview.rs"]
pub mod wire_preview;
/// The lighting readiness facts.
#[path = "../../../src/webui/wire/readiness.rs"]
pub mod wire_readiness;
/// The songs API's payloads.
#[path = "../../../src/webui/wire/songs.rs"]
pub mod wire_songs;
/// The status API's payloads.
#[path = "../../../src/webui/wire/status.rs"]
pub mod wire_status;
/// The venue API's payloads.
#[path = "../../../src/webui/wire/venue.rs"]
pub mod wire_venue;

// The player's module paths the included sources name each other by
// (`crate::audio::beat_grid`, `crate::config::Section`, …), so each
// compiles as it is.
pub mod audio {
    pub use crate::{beat_grid, guessed_tempo, output_status};
}

pub mod config {
    pub use crate::config_fade::{FadeCurve, FadeStatus};
    pub use crate::config_section::Section;
}

pub mod dmx {
    pub mod engine {
        pub use crate::ambient_mode::AmbientMode;
    }
}

pub mod player {
    pub use crate::hardware_status::*;
}

pub mod playlist {
    pub use crate::ambient_source::AmbientSource;
}

pub mod state {
    pub use crate::pose::PoseSnapshot;
}

pub mod webui {
    pub mod wire {
        pub use crate::wire_messages as messages;
    }
}

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

use std::path::Path;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tsify_next::Tsify;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;

pub use beat_grid::BeatGrid;
pub use lighting::parser::document::{
    CueEffect, Document, DocumentResult, EffectParams, SequenceRef, TempoSection,
};
pub use lighting::parser::edit::{EffectEdit, EffectView};
pub use lighting::parser::layout::Layout;
use lighting::parser::Sources;
pub use metronome_config::{MetronomeConfig, Subdivision};
pub use patch::{PatchOverlap, PatchOverrun, PatchSpan};
use tempo::TempoMap;
pub use tempo_config::{TempoConfig, TransitionConfig};

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
    ///
    /// Not the player's validity rule (`tempo::check_bpm` and the rest,
    /// which `TempoConfig::to_tempo_map` walks the fields with), and
    /// deliberately its own walk: it holds other fields (the start, a
    /// change's measure and beat, which the player takes at any size) to
    /// magnitudes, in its own words, and must refuse before
    /// `to_tempo_map` is reached, as past these bounds the player's
    /// functions would walk bar by bar for minutes.
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
type LayoutRequestJs = <LayoutRequest as Tsify>::JsType;
type PassEndsJs = <PassEnds as Tsify>::JsType;
type DocumentJs = <Document as Tsify>::JsType;
type TempoSectionJs = <TempoSection as Tsify>::JsType;
type CueEffectJs = <CueEffect as Tsify>::JsType;
type EffectParamsJs = <EffectParams as Tsify>::JsType;
type EffectEditJs = <EffectEdit as Tsify>::JsType;
type SequenceRefJs = <SequenceRef as Tsify>::JsType;
type TransitionConfigJs = <TransitionConfig as Tsify>::JsType;

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

/// Where a tempo map comes from: a song's `tempo:` block, the beat grid
/// of a song without one (`BeatGrid::to_tempo_map`, the map the player
/// cues such a song's light show against), or a light show's `tempo {}`
/// block as DSL text (the last file-level block, as the parser takes it).
#[derive(Deserialize, Tsify, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum TempoSource {
    /// The song's `tempo:` block.
    Config { config: TempoConfig },
    /// The song's beat grid, from click analysis.
    Grid { grid: BeatGrid },
    /// A light show's `tempo { }` block, as written.
    Dsl { dsl: String },
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
            TempoSource::Dsl { dsl } => lighting::parser::file_items(dsl, None)
                .map_err(|e| e.to_string())?
                .tempo_map
                .ok_or_else(|| "the source has no tempo block".to_string()),
        }
    }
}

/// A question to a tempo map: what is at a time, where a position is, or
/// where a duration written as the DSL writes one ends.
#[derive(Deserialize, Tsify, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum TempoQuery {
    /// A time in milliseconds of song time.
    Time { ms: f64 },
    /// A measure (1-based) and a possibly fractional beat (1-based, in the
    /// map's quarter-note units).
    Position { measure: u32, beat: f64 },
    /// Where a duration written as an effect's is (`2beats`, `1measure`,
    /// `500ms`, `1.5s`, a bare number of seconds) ends from a time, as the
    /// parser reads it there (`parse_duration_string`): through the tempo
    /// changes and ramps it runs over.
    Span { from_ms: f64, duration: String },
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
    /// before measure 1, or the question is beyond [`bounds`]. With no
    /// source at all, only a span in seconds or milliseconds has an answer,
    /// and its bar, beat and tempo are those of a bare clock (bar 1 at 0,
    /// the default tempo and metre).
    pub fn answer(&self, source: Option<&TempoSource>) -> Option<TempoPoint> {
        let map = match source {
            Some(source) => Some(source.to_tempo_map().ok()?),
            None => None,
        };
        let (at, measure, beat, label, sig) = match self {
            TempoQuery::Time { ms } => {
                let map = map.as_ref()?;
                if *ms < 0.0 || !bounds::secs(ms / 1000.0) {
                    return None;
                }
                let at = Duration::from_secs_f64(ms / 1000.0);
                let (measure, beat) = map.time_to_measure_beat(at);
                let sig = map.time_signature_at_time(at, 0.0);
                (at, measure, beat, map.position_label(at), sig)
            }
            TempoQuery::Position { measure, beat } => {
                let map = map.as_ref()?;
                if *measure > bounds::MAX_MEASURES || !bounds::beat(*beat) {
                    return None;
                }
                let at = map.measure_to_time_with_offset(*measure, *beat, 0, 0.0)?;
                // The bar the point is in, by its time: a beat past its
                // measure's last (beat 5 of a 4/4 bar) is in the next bar.
                let sig = map.time_signature_at_time(at, 0.0);
                (
                    at,
                    *measure,
                    *beat,
                    tempo::position_label_for(*measure, *beat),
                    sig,
                )
            }
            TempoQuery::Span { from_ms, duration } => {
                if *from_ms < 0.0 || !bounds::secs(from_ms / 1000.0) {
                    return None;
                }
                let from = Duration::from_secs_f64(from_ms / 1000.0);
                let length =
                    lighting::parser::utils::parse_duration_string(duration, &map, Some(from), 0.0)
                        .ok()?;
                let at = from.saturating_add(length);
                let Some(map) = map.as_ref() else {
                    return Some(TempoPoint::on_a_bare_clock(at));
                };
                let (measure, beat) = map.time_to_measure_beat(at);
                let sig = map.time_signature_at_time(at, 0.0);
                (at, measure, beat, map.position_label(at), sig)
            }
        };
        let map = map.as_ref()?;
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

impl TempoPoint {
    /// A time with no tempo map to place it on: a span in seconds ends
    /// here, on a bare clock at the default tempo and metre.
    fn on_a_bare_clock(at: Duration) -> TempoPoint {
        let sig = tempo::DEFAULT_TIME_SIGNATURE;
        TempoPoint {
            ms: at.as_secs_f64() * 1000.0,
            measure: 1,
            beat: 1.0,
            label: tempo::position_label_for(1, 1.0),
            bpm: tempo::DEFAULT_BPM,
            sig: (sig.numerator, sig.denominator),
            sig_set: (sig.numerator, sig.denominator),
        }
    }
}

/// [`TempoQuery::answer`], for the web UI. `source` may be `null` for a
/// show with no tempo at all, whose spans are in seconds alone.
#[wasm_bindgen]
pub fn tempo_query(source: Option<TempoSourceJs>, query: TempoQueryJs) -> Option<TempoPoint> {
    let source: Option<TempoSource> = match source {
        Some(source) => Some(read(JsValue::from(source))?),
        None => None,
    };
    let query: TempoQuery = read(JsValue::from(query))?;
    query.answer(source.as_ref())
}

/// The distance between two times on a tempo map, as the position walk
/// counts it: in milliseconds, in quarter-note beats
/// (`TempoMap::beats_between`) and in bars of the metres between
/// (`TempoMap::measures_between`, fractional within a bar). What the
/// timeline names a dragged length by.
#[derive(Serialize, Tsify, Clone, Copy, Debug, PartialEq)]
#[tsify(into_wasm_abi)]
pub struct TempoDistance {
    pub ms: f64,
    pub beats: f64,
    pub measures: f64,
}

impl TempoDistance {
    /// The distance from `from_ms` to `to_ms` on `source`'s map; `None`
    /// when the source makes no map or a time is not one.
    pub fn between(source: &TempoSource, from_ms: f64, to_ms: f64) -> Option<TempoDistance> {
        if from_ms < 0.0
            || to_ms < 0.0
            || !bounds::secs(from_ms / 1000.0)
            || !bounds::secs(to_ms / 1000.0)
        {
            return None;
        }
        let map = source.to_tempo_map().ok()?;
        let from = Duration::from_secs_f64(from_ms / 1000.0);
        let to = Duration::from_secs_f64(to_ms / 1000.0);
        Some(TempoDistance {
            ms: to_ms - from_ms,
            beats: map.beats_between(from, to),
            measures: map.measures_between(from, to),
        })
    }
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

// ── Light shows ─────────────────────────────────────────────────────────────

/// A `.light` file to lay out, with everything the parser would read off
/// the disk for it: the sources of the files it includes (fetched by the
/// UI), and the song's tempo map for a file with no tempo block.
#[derive(Deserialize, Tsify, Clone, Debug, PartialEq)]
#[tsify(from_wasm_abi)]
pub struct LayoutRequest {
    pub source: String,
    /// The file's path (as the lighting API names it), which its
    /// `include`s resolve against; `None` for a draft that is no file yet.
    #[serde(default)]
    #[tsify(optional)]
    pub path: Option<String>,
    /// The song's tempo map, for a file with no `tempo` block.
    #[serde(default)]
    #[tsify(optional)]
    pub song_tempo: Option<TempoSource>,
    /// The included files, by the path [`Document`]'s `includes` resolved
    /// them to.
    #[serde(default)]
    pub includes: Vec<IncludeSource>,
    /// How far the timeline reaches at least: the song's length.
    pub until_ms: f64,
}

/// An included file as the UI fetched it, or why it could not.
#[derive(Deserialize, Tsify, Clone, Debug, PartialEq)]
pub struct IncludeSource {
    pub path: String,
    #[serde(default)]
    #[tsify(optional)]
    pub source: Option<String>,
    /// Why the file could not be fetched; the include that names it
    /// reports as the player would, with this where the player has the
    /// file system's reason.
    #[serde(default)]
    #[tsify(optional)]
    pub error: Option<String>,
}

impl LayoutRequest {
    /// The layout, by the player's own parse of the file
    /// (`parser::layout::layout`).
    pub fn answer(&self) -> Layout {
        let mut sources = Sources::none();
        for include in &self.includes {
            match (&include.source, &include.error) {
                (Some(source), _) => sources.add(&include.path, source.clone()),
                (None, Some(error)) => sources.add_missing(&include.path, error.clone()),
                (None, None) => {}
            }
        }
        let external = match self.song_tempo.as_ref().map(TempoSource::to_tempo_map) {
            Some(Ok(map)) => Some(map),
            Some(Err(error)) => {
                return Layout {
                    error: Some(format!("the song's tempo makes no tempo map: {error}")),
                    ..Layout::default()
                }
            }
            None => None,
        };
        let until = if self.until_ms > 0.0 && bounds::secs(self.until_ms / 1000.0) {
            Duration::from_secs_f64(self.until_ms / 1000.0)
        } else {
            Duration::ZERO
        };
        lighting::parser::layout::layout(
            &self.source,
            self.path.as_deref().map(Path::new),
            external.as_ref(),
            &sources,
            until,
        )
    }
}

/// [`LayoutRequest::answer`], for the web UI: a request that is not one
/// (no source) lays out as an error.
#[wasm_bindgen]
pub fn layout(request: LayoutRequestJs) -> Layout {
    match read::<LayoutRequest>(JsValue::from(request)) {
        Some(request) => request.answer(),
        None => Layout {
            error: Some("the layout request is not one".to_string()),
            ..Layout::default()
        },
    }
}

/// A `.light` file as written (`parser::document::document`): the editing
/// model, read off the one grammar. `path` is the file's path, which its
/// `include`s resolve against.
#[wasm_bindgen]
pub fn document(source: &str, path: Option<String>) -> DocumentResult {
    lighting::parser::document::document(source, path.as_deref().map(Path::new))
}

/// The document written back as DSL text (`writer::write`): what a save
/// writes and what the layout reads. `None` for a value that is not a
/// document, or one holding a number that is none (NaN, an infinity).
#[wasm_bindgen]
pub fn document_write(document: DocumentJs) -> Option<String> {
    let document: Document = read(JsValue::from(document))?;
    // A number the grammar has no spelling for is refused, never written.
    document
        .numbers_finite()
        .then(|| lighting::parser::writer::write(&document))
}

/// A `tempo { }` block alone, written as [`document_write`] writes it
/// (`writer::write_tempo`): the `{ dsl }` tempo source of a show whose
/// block is being edited. `None` for a value that is not a tempo block.
#[wasm_bindgen]
pub fn tempo_write(tempo: TempoSectionJs) -> Option<String> {
    let tempo: TempoSection = read(JsValue::from(tempo))?;
    tempo
        .numbers_finite()
        .then(|| lighting::parser::writer::write_tempo(&tempo))
}

/// An effect line with a form's edit made (`CueEffect::edited`): the
/// value written as the grammar wants it, only the edited text changed,
/// a removed parameter's comments kept in place. `None` for a value that
/// is not an effect or an edit.
#[wasm_bindgen]
pub fn effect_edit(effect: CueEffectJs, edit: EffectEditJs) -> Option<CueEffect> {
    let effect: CueEffect = read(JsValue::from(effect))?;
    let edit: EffectEdit = read(JsValue::from(edit))?;
    Some(effect.edited(&edit))
}

/// An effect's parameters as its form shows them (`EffectParams::view`).
#[wasm_bindgen]
pub fn effect_view(effect: EffectParamsJs) -> Option<EffectView> {
    let effect: EffectParams = read(JsValue::from(effect))?;
    Some(effect.view())
}

/// The effect a click on a timeline lane adds (`edit::new_effect`).
#[wasm_bindgen]
pub fn effect_new(layer: Option<String>, by_bars: bool) -> CueEffect {
    lighting::parser::edit::new_effect(layer.as_deref(), by_bars)
}

/// A sequence reference set to play `count` passes, as its block dragged
/// to a length says (`SequenceRef::with_loop_count`).
#[wasm_bindgen]
pub fn reference_loop(reference: SequenceRefJs, count: f64) -> Option<SequenceRef> {
    let reference: SequenceRef = read(JsValue::from(reference))?;
    reference.with_loop_count(count)
}

/// The duration an effect dragged to `ms` long from `at_ms` is written as
/// (`writer::duration_text`): whole measures or beats of the tempo map
/// there when it is some, else time.
pub fn length_name(source: Option<&TempoSource>, at_ms: f64, ms: f64) -> String {
    let span = source
        .and_then(|s| TempoDistance::between(s, at_ms, at_ms + ms))
        .map(|d| (d.measures, d.beats));
    lighting::parser::writer::duration_text(span, ms)
}

/// [`length_name`], for the web UI.
#[wasm_bindgen]
pub fn duration_text(source: Option<TempoSourceJs>, at_ms: f64, ms: f64) -> String {
    let source = source.and_then(|s| read::<TempoSource>(JsValue::from(s)));
    length_name(source.as_ref(), at_ms, ms)
}

/// A length dragged from `at_ms`, snapped to whole measures (`step`
/// `None`) or to whole steps of a beat, in milliseconds as the parser
/// reads that length there. `None` when the source makes no tempo map or
/// a time is not one.
pub fn snapped_length(
    source: &TempoSource,
    at_ms: f64,
    duration_ms: f64,
    step: Option<f64>,
) -> Option<f64> {
    if step.is_some_and(|step| !(step.is_finite() && step > 0.0)) {
        return None;
    }
    let span = TempoDistance::between(source, at_ms, at_ms + duration_ms)?;
    let map = source.to_tempo_map().ok()?;
    let round = |n: f64| (n + 0.5).floor();
    let from = Duration::from_secs_f64(at_ms / 1000.0);
    // The player's own length of that many bars or beats from there.
    let length = match step {
        None => map.measures_to_duration(round(span.measures).max(1.0), from, 0.0),
        Some(step) => map.beats_to_duration((round(span.beats / step) * step).max(step), from, 0.0),
    };
    // A length past any song (a step so fine its count overflows) is none.
    let end = from.checked_add(length)?.as_secs_f64();
    bounds::secs(end).then_some(end * 1000.0 - at_ms)
}

/// [`snapped_length`], for the web UI.
#[wasm_bindgen]
pub fn duration_snap(
    source: TempoSourceJs,
    at_ms: f64,
    duration_ms: f64,
    step: Option<f64>,
) -> Option<f64> {
    let source = read::<TempoSource>(JsValue::from(source))?;
    snapped_length(&source, at_ms, duration_ms, step)
}

/// A song tempo change's transition as a light show's `transition:`
/// value (`writer::transition_text`): `4` beats, `2m` measures.
#[wasm_bindgen]
pub fn transition_text(transition: TransitionConfigJs) -> Option<String> {
    use tempo::{TempoTransition, TransitionCurve};
    let transition = match read::<TransitionConfig>(JsValue::from(transition))? {
        TransitionConfig::Beats(n) => TempoTransition::Beats(n, TransitionCurve::Linear),
        TransitionConfig::Measures(n) => TempoTransition::Measures(n, TransitionCurve::Linear),
    };
    Some(lighting::parser::writer::transition_text(&transition))
}

/// A light show's `transition:` value as a song tempo change's
/// transition, as the player reads it (`document::transition`); `None`
/// for `snap` and for text that is not a transition.
#[wasm_bindgen]
pub fn transition_read(text: &str) -> Option<TransitionConfig> {
    use tempo::TempoTransition;
    match lighting::parser::document::transition(text)? {
        TempoTransition::Snap => None,
        TempoTransition::Beats(n, _) => Some(TransitionConfig::Beats(n)),
        TempoTransition::Measures(n, _) => Some(TransitionConfig::Measures(n)),
    }
}

/// `value` in `from` (`s` or `ms`) written in `to` instead, the same time
/// as the player reads it (`document::time_in`): what a tempo block's
/// start becomes when its unit is switched.
#[wasm_bindgen]
pub fn time_in(value: f64, from: &str, to: &str) -> Option<f64> {
    lighting::parser::document::time_in(value, from, to)
}

/// A sequence reference's `PassLayout::pass_ends`, as the page hands them
/// back.
#[derive(Deserialize, Tsify, Clone, Debug, PartialEq)]
#[serde(transparent)]
pub struct PassEnds(pub Vec<f64>);

/// How many whole passes of a sequence reference fit before `until_ms`,
/// from its layout's `pass_ends` (laid by the player to the end of the
/// timeline): the count its block dragged to end there stands for
/// (`layout::passes_by`). `None` for a value that is not a list of times.
#[wasm_bindgen]
pub fn passes_by(pass_ends: PassEndsJs, until_ms: f64) -> Option<u32> {
    let ends: PassEnds = read(JsValue::from(pass_ends))?;
    Some(lighting::parser::layout::passes_by(&ends.0, until_ms))
}

/// What a light show's tempo block says when it says nothing: the
/// player's default tempo and metre (`DEFAULT_BPM`,
/// `DEFAULT_TIME_SIGNATURE`): what the editor's new tempo block starts
/// at.
#[derive(Serialize, Tsify, Clone, Copy, Debug, PartialEq)]
#[tsify(into_wasm_abi)]
pub struct TempoDefaults {
    pub bpm: f64,
    pub time_signature: (u32, u32),
}

/// [`TempoDefaults`], for the web UI.
#[wasm_bindgen]
pub fn tempo_defaults() -> TempoDefaults {
    let sig = tempo::DEFAULT_TIME_SIGNATURE;
    TempoDefaults {
        bpm: tempo::DEFAULT_BPM,
        time_signature: (sig.numerator, sig.denominator),
    }
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
