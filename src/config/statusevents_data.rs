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

//! A profile's `status_events:` as written: light, so the web UI's WebAssembly module compiles it as it is
//! and the UI edits it by the generated types. What it means is `statusevents`'s.

use super::midi_event::MidiEvent;
use serde::{Deserialize, Serialize};

/// The configuration for emitting status events.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone)]
pub struct StatusEvents {
    /// The events to emit to clear the status.
    pub(super) off_events: Vec<MidiEvent>,
    /// The events to emit to indicate that the player is idling and waiting for input.
    pub(super) idling_events: Vec<MidiEvent>,
    /// The events to emit to indicate that the player is currently playing.
    pub(super) playing_events: Vec<MidiEvent>,
    /// The events to emit, in place of the idling events, while an auto
    /// play waits to start its song. None configured: the idling events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) auto_play_events: Vec<MidiEvent>,
}
