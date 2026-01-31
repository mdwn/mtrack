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
use std::{error::Error, time::Duration};

use duration_string::DurationString;
use midly::{
    live::LiveEvent,
    num::{u14, u4, u7},
};
use serde::{Deserialize, Serialize};

const DEFAULT_MIDI_PLAYBACK_DELAY: Duration = Duration::ZERO;

/// A YAML representation of the MIDI configuration.
#[derive(Deserialize, Clone)]
pub struct Midi {
    /// The MIDI device.
    device: String,

    /// Controls how long to wait before playback of a MIDI file starts.
    playback_delay: Option<String>,

    /// MIDI to DMX passthrough configurations.
    midi_to_dmx: Option<Vec<MidiToDmx>>,
}

impl Midi {
    /// New will create a new MIDI configuration.
    pub fn new(device: &str, playback_delay: Option<String>) -> Midi {
        Midi {
            device: device.to_string(),
            playback_delay,
            midi_to_dmx: None,
        }
    }

    /// Returns the device from the configuration.
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Returns the playback delay from the configuration.
    pub fn playback_delay(&self) -> Result<Duration, Box<dyn Error>> {
        match &self.playback_delay {
            Some(playback_delay) => Ok(DurationString::from_string(playback_delay.clone())?.into()),
            None => Ok(DEFAULT_MIDI_PLAYBACK_DELAY),
        }
    }

    /// Returns the MIDI to DMX configuration.
    pub fn midi_to_dmx(&self) -> Vec<MidiToDmx> {
        self.midi_to_dmx.clone().unwrap_or_default()
    }
}

/// A YAML representation of the MIDI configuration.
#[derive(Deserialize, Clone)]
pub struct MidiToDmx {
    /// The MIDI channel to pass through to DMX.
    midi_channel: u8,

    /// The DMX universe to target.
    universe: String,

    /// Transformations to apply to the input to this mapping.
    transformers: Option<Vec<MidiTransformer>>,
}

impl MidiToDmx {
    /// The MIDI channel associated with this mapping.
    pub fn midi_channel(&self) -> Result<u7, Box<dyn Error>> {
        u7::try_from(self.midi_channel - 1).ok_or("error parsing MIDI channel".into())
    }

    /// The DMX universe to map the MIDI channel to.
    pub fn universe(&self) -> String {
        self.universe.clone()
    }

    /// The transformers to apply to the input MIDI.
    pub fn transformers(&self) -> Vec<MidiTransformer> {
        self.transformers.clone().unwrap_or_default()
    }
}

/// Implementers must convert to a MIDI live event.
pub trait ToMidiEvent {
    /// Converts the implementer to a MIDI live event.
    fn to_midi_event(&self) -> Result<LiveEvent<'static>, Box<dyn Error>>;
}

#[derive(Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MidiTransformer {
    NoteMapper(NoteMapper),
    ControlChangeMapper(ControlChangeMapper),
}

/// A YAML representation of the note mapper MIDI transformation.
#[derive(Deserialize, Clone)]
pub struct NoteMapper {
    input_note: u8,
    convert_to_notes: Vec<u8>,
}

impl NoteMapper {
    /// Gets the input note.
    pub fn input_note(&self) -> Result<u7, Box<dyn Error>> {
        u7::try_from(self.input_note).ok_or("input note cannot be converted to a u7".into())
    }

    /// Gets the notes to convert the input to.
    pub fn convert_to_notes(&self) -> Result<Vec<u7>, Box<dyn Error>> {
        self.convert_to_notes
            .iter()
            .map(|note| u7::try_from(*note).ok_or("unable to convert note to u7".into()))
            .collect()
    }
}

/// A YAML representation of the control change mapper MIDI transformation.
#[derive(Deserialize, Clone)]
pub struct ControlChangeMapper {
    input_controller: u8,
    convert_to_controllers: Vec<u8>,
}

impl ControlChangeMapper {
    /// Gets the input controller.
    pub fn input_controller(&self) -> Result<u7, Box<dyn Error>> {
        u7::try_from(self.input_controller)
            .ok_or("input controller cannot be converted to a u7".into())
    }

    /// Gets the controllers to convert the input to.
    pub fn convert_to_notes(&self) -> Result<Vec<u7>, Box<dyn Error>> {
        self.convert_to_controllers
            .iter()
            .map(|controller| {
                u7::try_from(*controller).ok_or("unable to convert controller to u7".into())
            })
            .collect()
    }
}

/// MIDI events that can be parsed from YAML.
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    NoteOff(NoteOff),
    NoteOn(NoteOn),
    Aftertouch(Aftertouch),
    ControlChange(ControlChange),
    ProgramChange(ProgramChange),
    ChannelAftertouch(ChannelAftertouch),
    PitchBend(PitchBend),
}

/// Creates a note on MIDI event.
#[cfg(test)]
pub fn note_on(channel: u8, key: u8, velocity: u8) -> Event {
    Event::NoteOn(NoteOn {
        channel,
        key,
        velocity,
    })
}

impl ToMidiEvent for Event {
    fn to_midi_event(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        match self {
            Event::NoteOff(e) => e.to_midi_event(),
            Event::NoteOn(e) => e.to_midi_event(),
            Event::Aftertouch(e) => e.to_midi_event(),
            Event::ControlChange(e) => e.to_midi_event(),
            Event::ProgramChange(e) => e.to_midi_event(),
            Event::ChannelAftertouch(e) => e.to_midi_event(),
            Event::PitchBend(e) => e.to_midi_event(),
        }
    }
}

/// A NoteOff event.
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct NoteOff {
    /// The channel the MIDI event belongs to.
    channel: u8,
    /// The key for the note off event.
    key: u8,
    /// The velocity of the note off event.
    /// Optional for trigger matching; defaults to 0.
    #[serde(default)]
    velocity: u8,
}

#[cfg(test)]
impl NoteOff {
    /// Gets the channel (1-indexed).
    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// Gets the key.
    pub fn key(&self) -> u8 {
        self.key
    }
}

impl ToMidiEvent for NoteOff {
    fn to_midi_event(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        Ok(LiveEvent::Midi {
            channel: parse_channel(self.channel)?,
            message: midly::MidiMessage::NoteOff {
                key: parse_u7(self.key)?,
                vel: parse_u7(self.velocity)?,
            },
        })
    }
}

/// A NoteOn event.
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct NoteOn {
    /// The channel the MIDI event belongs to.
    channel: u8,
    /// The key of the note on event.
    key: u8,
    /// The velocity of the note on event.
    /// Optional for trigger matching; defaults to 0.
    #[serde(default)]
    velocity: u8,
}

#[cfg(test)]
impl NoteOn {
    /// Gets the channel (1-indexed).
    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// Gets the key.
    pub fn key(&self) -> u8 {
        self.key
    }
}

impl ToMidiEvent for NoteOn {
    fn to_midi_event(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        Ok(LiveEvent::Midi {
            channel: parse_channel(self.channel)?,
            message: midly::MidiMessage::NoteOn {
                key: parse_u7(self.key)?,
                vel: parse_u7(self.velocity)?,
            },
        })
    }
}

/// An Aftertouch event.
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct Aftertouch {
    /// The channel the MIDI event belongs to.
    channel: u8,
    /// The key value of the aftertouch event.
    key: u8,
    /// The velocity value of the aftertouch event.
    velocity: u8,
}

impl ToMidiEvent for Aftertouch {
    fn to_midi_event(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        Ok(LiveEvent::Midi {
            channel: parse_channel(self.channel)?,
            message: midly::MidiMessage::Aftertouch {
                key: parse_u7(self.key)?,
                vel: parse_u7(self.velocity)?,
            },
        })
    }
}

/// A ControlChange event.
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct ControlChange {
    /// The channel the MIDI event belongs to.
    channel: u8,
    /// Controller is the controller for a control_change event.
    controller: u8,
    /// Value is the control_change value.
    value: u8,
}

impl ToMidiEvent for ControlChange {
    fn to_midi_event(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        Ok(LiveEvent::Midi {
            channel: parse_channel(self.channel)?,
            message: midly::MidiMessage::Controller {
                controller: parse_u7(self.controller)?,
                value: parse_u7(self.value)?,
            },
        })
    }
}

/// A ProgramChange event.
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct ProgramChange {
    /// The channel the MIDI event belongs to.
    channel: u8,
    /// Program is the program value for program_change events.
    program: u8,
}

impl ToMidiEvent for ProgramChange {
    fn to_midi_event(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        Ok(LiveEvent::Midi {
            channel: parse_channel(self.channel)?,
            message: midly::MidiMessage::ProgramChange {
                program: parse_u7(self.program)?,
            },
        })
    }
}

/// A ChannelAftertouch event.
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct ChannelAftertouch {
    /// The channel the MIDI event belongs to.
    channel: u8,
    /// The velocity of the channel aftertouch event.
    velocity: u8,
}

impl ToMidiEvent for ChannelAftertouch {
    fn to_midi_event(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        Ok(LiveEvent::Midi {
            channel: parse_channel(self.channel)?,
            message: midly::MidiMessage::ChannelAftertouch {
                vel: parse_u7(self.velocity)?,
            },
        })
    }
}

/// A PitchBend event.
#[derive(Deserialize, Clone, Serialize, Debug, PartialEq, Eq)]
pub struct PitchBend {
    /// The channel the MIDI event belongs to.
    channel: u8,
    /// The pitchbend event.
    bend: u16,
}

impl ToMidiEvent for PitchBend {
    fn to_midi_event(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        Ok(LiveEvent::Midi {
            channel: parse_channel(self.channel)?,
            message: midly::MidiMessage::PitchBend {
                bend: midly::PitchBend(parse_u14(self.bend)?),
            },
        })
    }
}

/// Parses a channel from the config. Input is expected to be [1, 16].
fn parse_channel(channel: u8) -> Result<u4, Box<dyn Error>> {
    match u4::try_from(channel - 1) {
        Some(val) => Ok(val),
        None => Err(format!("error parsing channel: {} is invalid", channel).into()),
    }
}

/// Parses a raw u7 value.
fn parse_u7(raw: u8) -> Result<u7, Box<dyn Error>> {
    match u7::try_from(raw) {
        Some(val) => Ok(val),
        None => Err(format!("error parsing u7 value: {} is invalid", raw).into()),
    }
}

// Parses a raw u14 value.
fn parse_u14(raw: u16) -> Result<u14, Box<dyn Error>> {
    match u14::try_from(raw) {
        Some(val) => Ok(val),
        None => Err(format!("error parsing u14 value: {} is invalid", raw).into()),
    }
}

#[cfg(test)]
mod test {
    use std::error::Error;

    use config::{Config, File, FileFormat};
    use midly::{
        live::LiveEvent,
        num::{u14, u4, u7},
    };

    use crate::config::midi::ToMidiEvent;

    #[test]
    fn note_off() -> Result<(), Box<dyn Error>> {
        assert_yaml_matches_midi(
            r#"
            type: note_off
            channel: 7
            key: 5
            velocity: 28
        "#
            .into(),
            LiveEvent::Midi {
                channel: u4::from(6),
                message: midly::MidiMessage::NoteOff {
                    key: u7::from(5),
                    vel: u7::from(28),
                },
            },
        )
    }

    #[test]
    fn note_on() -> Result<(), Box<dyn Error>> {
        assert_yaml_matches_midi(
            r#"
            type: note_on
            channel: 7
            key: 5
            velocity: 28
        "#
            .into(),
            LiveEvent::Midi {
                channel: u4::from(6),
                message: midly::MidiMessage::NoteOn {
                    key: u7::from(5),
                    vel: u7::from(28),
                },
            },
        )
    }

    #[test]
    fn aftertouch() -> Result<(), Box<dyn Error>> {
        assert_yaml_matches_midi(
            r#"
            type: aftertouch
            channel: 7
            key: 5
            velocity: 28
        "#
            .into(),
            LiveEvent::Midi {
                channel: u4::from(6),
                message: midly::MidiMessage::Aftertouch {
                    key: u7::from(5),
                    vel: u7::from(28),
                },
            },
        )
    }

    #[test]
    fn control_change() -> Result<(), Box<dyn Error>> {
        assert_yaml_matches_midi(
            r#"
            type: control_change
            channel: 7
            controller: 5
            value: 28
        "#
            .into(),
            LiveEvent::Midi {
                channel: u4::from(6),
                message: midly::MidiMessage::Controller {
                    controller: u7::from(5),
                    value: u7::from(28),
                },
            },
        )
    }

    #[test]
    fn program_change() -> Result<(), Box<dyn Error>> {
        assert_yaml_matches_midi(
            r#"
            type: program_change
            channel: 7
            program: 5
        "#
            .into(),
            LiveEvent::Midi {
                channel: u4::from(6),
                message: midly::MidiMessage::ProgramChange {
                    program: u7::from(5),
                },
            },
        )
    }

    #[test]
    fn channel_aftertouch() -> Result<(), Box<dyn Error>> {
        assert_yaml_matches_midi(
            r#"
            type: channel_aftertouch
            channel: 7
            velocity: 5
        "#
            .into(),
            LiveEvent::Midi {
                channel: u4::from(6),
                message: midly::MidiMessage::ChannelAftertouch { vel: u7::from(5) },
            },
        )
    }

    #[test]
    fn pitch_bend() -> Result<(), Box<dyn Error>> {
        assert_yaml_matches_midi(
            r#"
            type: pitch_bend
            channel: 7
            bend: 200
        "#
            .into(),
            LiveEvent::Midi {
                channel: u4::from(6),
                message: midly::MidiMessage::PitchBend {
                    bend: midly::PitchBend(u14::from(200)),
                },
            },
        )
    }

    fn assert_yaml_matches_midi(
        yaml: String,
        expected_event: midly::live::LiveEvent,
    ) -> Result<(), Box<dyn Error>> {
        let event = Config::builder()
            .add_source(File::from_str(&yaml, FileFormat::Yaml))
            .build()?
            .try_deserialize::<super::Event>()?
            .to_midi_event()?;

        if expected_event == event {
            Ok(())
        } else {
            Err("expected event did not match".into())
        }
    }
}
