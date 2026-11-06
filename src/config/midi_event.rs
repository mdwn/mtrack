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
/// its lowest. The web UI's event editor offers what this says, so a new
/// kind reaches it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(into_wasm_abi))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MidiEventShapes {
    pub kinds: Vec<MidiEvent>,
}

impl MidiEventShapes {
    pub fn new() -> Self {
        let kinds: Vec<MidiEvent> = (0..).map_while(MidiEvent::kind_at).collect();
        MidiEventShapes { kinds }
    }
}

impl Default for MidiEventShapes {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiEvent {
    /// The kind at `index` in declaration order, its fields at their
    /// lowest; `None` past the last. The match below names every kind, so
    /// adding one does not compile until it is listed here too.
    fn kind_at(index: usize) -> Option<MidiEvent> {
        let kind = match index {
            0 => MidiEvent::NoteOff(NoteOff {
                channel: 1,
                key: 0,
                velocity: 0,
            }),
            1 => MidiEvent::NoteOn(NoteOn {
                channel: 1,
                key: 0,
                velocity: 0,
            }),
            2 => MidiEvent::Aftertouch(Aftertouch {
                channel: 1,
                key: 0,
                velocity: 0,
            }),
            3 => MidiEvent::ControlChange(ControlChange {
                channel: 1,
                controller: 0,
                value: 0,
            }),
            4 => MidiEvent::ProgramChange(ProgramChange {
                channel: 1,
                program: 0,
            }),
            5 => MidiEvent::ChannelAftertouch(ChannelAftertouch {
                channel: 1,
                velocity: 0,
            }),
            6 => MidiEvent::PitchBend(PitchBend {
                channel: 1,
                bend: 0,
            }),
            _ => return None,
        };
        // Exhaustive: a kind missing above fails to compile here.
        let listed = match &kind {
            MidiEvent::NoteOff(_) => 0,
            MidiEvent::NoteOn(_) => 1,
            MidiEvent::Aftertouch(_) => 2,
            MidiEvent::ControlChange(_) => 3,
            MidiEvent::ProgramChange(_) => 4,
            MidiEvent::ChannelAftertouch(_) => 5,
            MidiEvent::PitchBend(_) => 6,
        };
        debug_assert_eq!(listed, index);
        Some(kind)
    }
}
