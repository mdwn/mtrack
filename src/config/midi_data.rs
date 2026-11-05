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

//! A profile's `midi:` section as written: light, so the web UI's WebAssembly module compiles it as it is
//! and the UI edits it by the generated types. What it means is `midi`'s.

use serde::{Deserialize, Serialize};

/// A YAML representation of the MIDI configuration.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone)]
pub struct Midi {
    /// The MIDI device.
    pub(super) device: String,

    /// Controls how long to wait before playback of a MIDI file starts.
    #[serde(default)]
    pub(super) playback_delay: Option<String>,

    /// Enable MIDI beat clock output (24 ppqn timing clock).
    #[serde(default)]
    pub(super) beat_clock: Option<bool>,

    /// Keep the beat clock free-running at the last known tempo once a song
    /// stops, until the next song starts. Only meaningful when `beat_clock` is
    /// enabled.
    #[serde(default)]
    pub(super) persist_tempo: Option<bool>,

    /// What the beat clock does where a loop goes back (a section loop's
    /// jump, a looping song's next pass). Only meaningful when `beat_clock`
    /// is enabled.
    #[serde(default)]
    pub(super) beat_clock_seams: Option<BeatClockSeams>,
}

/// What the beat clock does where a loop goes back.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BeatClockSeams {
    /// Its pulses run straight on through the seam, from the first in the
    /// stretch gone back to; nothing else is sent. Gear counting pulses
    /// counts on past the loop.
    #[default]
    RunThrough,
    /// Stop, then a Song Position Pointer to where the loop goes back to (the
    /// nearest 16th note) and Continue, or Start at the top of the song, so
    /// gear that follows song position goes back with it. Some gear treats
    /// Continue as Start, or cuts its notes at Stop.
    Relocate,
}
