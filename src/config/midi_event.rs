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

//! The MIDI events a config names (a song's, a trigger's, a controller's):
//! light, so the web UI's WebAssembly module compiles them as they are.
//! Turning one into the bytes sent is `midi`'s.

use serde::{Deserialize, Serialize};

/// MIDI events that can be parsed from YAML. `midi::Event` in the player;
/// named in full here for TypeScript, where `Event` is the DOM's.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MidiEvent {
    NoteOff(NoteOff),
    NoteOn(NoteOn),
    Aftertouch(Aftertouch),
    ControlChange(ControlChange),
    ProgramChange(ProgramChange),
    ChannelAftertouch(ChannelAftertouch),
    PitchBend(PitchBend),
}

/// A NoteOff event.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct NoteOff {
    /// The channel the MIDI event belongs to.
    pub(super) channel: u8,
    /// The key for the note off event.
    pub(super) key: u8,
    /// The velocity of the note off event.
    /// Optional for trigger matching; defaults to 0.
    #[serde(default)]
    pub(super) velocity: u8,
}

/// A NoteOn event.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct NoteOn {
    /// The channel the MIDI event belongs to.
    pub(super) channel: u8,
    /// The key of the note on event.
    pub(super) key: u8,
    /// The velocity of the note on event.
    /// Optional for trigger matching; defaults to 0.
    #[serde(default)]
    pub(super) velocity: u8,
}

/// An Aftertouch event.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct Aftertouch {
    /// The channel the MIDI event belongs to.
    pub(super) channel: u8,
    /// The key value of the aftertouch event.
    pub(super) key: u8,
    /// The velocity value of the aftertouch event.
    pub(super) velocity: u8,
}

/// A ControlChange event.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct ControlChange {
    /// The channel the MIDI event belongs to.
    pub(super) channel: u8,
    /// Controller is the controller for a control_change event.
    pub(super) controller: u8,
    /// Value is the control_change value.
    pub(super) value: u8,
}

/// A ProgramChange event.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct ProgramChange {
    /// The channel the MIDI event belongs to.
    pub(super) channel: u8,
    /// Program is the program value for program_change events.
    pub(super) program: u8,
}

/// A ChannelAftertouch event.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct ChannelAftertouch {
    /// The channel the MIDI event belongs to.
    pub(super) channel: u8,
    /// The velocity of the channel aftertouch event.
    pub(super) velocity: u8,
}

/// A PitchBend event.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct PitchBend {
    /// The channel the MIDI event belongs to.
    pub(super) channel: u8,
    /// The pitchbend event.
    pub(super) bend: u16,
}
