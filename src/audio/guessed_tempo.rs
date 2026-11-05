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

//! A tempo map guessed for a song, as the web UI is sent it: light, so the
//! UI's WebAssembly module compiles it as it is. The guessing is
//! `midi_tempo`'s.

use serde::Serialize;

/// A guessed tempo map: what the web UI suggests for a song's `tempo:`
/// block or a show's `tempo {}` block.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GuessedTempo {
    pub start_seconds: f64,
    pub bpm: f64,
    pub time_signature: [u32; 2],
    pub changes: Vec<GuessedTempoChange>,
    /// RMS alignment error in milliseconds between MIDI-predicted beats and the
    /// click-track beat grid, measured after cross-correlation offset detection.
    /// `None` when no beat grid was available for comparison.
    /// Values above ~15ms suggest the MIDI may not match the recorded audio.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment_rms_ms: Option<f64>,
}

/// A single tempo or time-signature change.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GuessedTempoChange {
    pub measure: u32,
    pub beat: u32,
    pub bpm: f64,
    pub time_signature: [u32; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition_beats: Option<u32>,
}
