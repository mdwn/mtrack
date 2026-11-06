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

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// MIDI events that can be parsed from YAML. `midi::Event` in the player;
/// named in full here for TypeScript, where `Event` is the DOM's.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
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
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
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
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
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
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
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
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
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
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct ProgramChange {
    /// The channel the MIDI event belongs to.
    pub(super) channel: u8,
    /// Program is the program value for program_change events.
    pub(super) program: u8,
}

/// A ChannelAftertouch event.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct ChannelAftertouch {
    /// The channel the MIDI event belongs to.
    pub(super) channel: u8,
    /// The velocity of the channel aftertouch event.
    pub(super) velocity: u8,
}

/// A PitchBend event.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct PitchBend {
    /// The channel the MIDI event belongs to.
    pub(super) channel: u8,
    /// The pitchbend event.
    pub(super) bend: u16,
}

/// What a config's MIDI events may hold: one of each kind, every field at
/// its lowest, and the range each field takes as the player reads it
/// (`midi`'s parsing, which a test holds to these). The web UI's event
/// editor offers what this says, so a new kind reaches it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(into_wasm_abi, hashmap_as_object))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MidiEventShapes {
    pub kinds: Vec<MidiEvent>,
    /// `[lowest, highest]`, by field name.
    pub ranges: BTreeMap<String, (u16, u16)>,
}

impl MidiEventShapes {
    pub fn new() -> Self {
        let kinds = MidiEvent::kinds();
        let seven_bit = (0, 127);
        let ranges = [
            ("channel", (1, 16)),
            ("key", seven_bit),
            ("velocity", seven_bit),
            ("controller", seven_bit),
            ("value", seven_bit),
            ("program", seven_bit),
            ("bend", (0, 16383)),
        ]
        .into_iter()
        .map(|(name, range)| (name.to_string(), range))
        .collect();
        MidiEventShapes { kinds, ranges }
    }
}

impl Default for MidiEventShapes {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiEvent {
    /// One event of each kind, in declaration order, every field at its
    /// lowest. A test counts the kinds by a match of its own, so a kind
    /// missing here fails it.
    fn kinds() -> Vec<MidiEvent> {
        vec![
            MidiEvent::NoteOff(NoteOff {
                channel: 1,
                key: 0,
                velocity: 0,
            }),
            MidiEvent::NoteOn(NoteOn {
                channel: 1,
                key: 0,
                velocity: 0,
            }),
            MidiEvent::Aftertouch(Aftertouch {
                channel: 1,
                key: 0,
                velocity: 0,
            }),
            MidiEvent::ControlChange(ControlChange {
                channel: 1,
                controller: 0,
                value: 0,
            }),
            MidiEvent::ProgramChange(ProgramChange {
                channel: 1,
                program: 0,
            }),
            MidiEvent::ChannelAftertouch(ChannelAftertouch {
                channel: 1,
                velocity: 0,
            }),
            MidiEvent::PitchBend(PitchBend {
                channel: 1,
                bend: 0,
            }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind of `MidiEvent` is in `kinds`, each once. The match is
    /// exhaustive: a new kind does not compile until it has an index here,
    /// and then fails until `kinds` lists it.
    #[test]
    fn kinds_lists_every_kind_once() {
        const KINDS: usize = 7;
        let index = |kind: &MidiEvent| match kind {
            MidiEvent::NoteOff(_) => 0,
            MidiEvent::NoteOn(_) => 1,
            MidiEvent::Aftertouch(_) => 2,
            MidiEvent::ControlChange(_) => 3,
            MidiEvent::ProgramChange(_) => 4,
            MidiEvent::ChannelAftertouch(_) => 5,
            MidiEvent::PitchBend(_) => 6,
        };
        let listed: Vec<usize> = MidiEvent::kinds().iter().map(index).collect();
        assert_eq!(listed, (0..KINDS).collect::<Vec<_>>());
    }
}
