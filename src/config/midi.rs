// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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
use serde::Deserialize;

const DEFAULT_MIDI_PLAYBACK_DELAY: Duration = Duration::ZERO;

/// A YAML representation of the MIDI configuration.
#[derive(Deserialize)]
pub(crate) struct Midi {
    /// The MIDI device.
    device: String,

    /// Controls how long to wait before playback of a DMX lighting file starts.
    playback_delay: Option<String>,
}

impl Midi {
    /// New will create a new MIDI configuration.
    pub fn new(device: String, playback_delay: Option<String>) -> Midi {
        Midi {
            device,
            playback_delay,
        }
    }

    /// Returns the device from the configuration.
    pub fn device(&self) -> String {
        self.device.clone()
    }

    /// Returns the playback delay from the configuration.
    pub fn playback_delay(&self) -> Result<Duration, Box<dyn Error>> {
        match &self.playback_delay {
            Some(playback_delay) => Ok(DurationString::from_string(playback_delay.clone())?.into()),
            None => Ok(DEFAULT_MIDI_PLAYBACK_DELAY),
        }
    }
}

/// Implementers must convert to a MIDI live event.
pub(super) trait ToMidiEvent {
    /// Converts the implementer to a MIDI live event.
    fn to_midi_event(&self) -> Result<LiveEvent<'static>, Box<dyn Error>>;
}

/// MIDI events that can be parsed from YAML.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Event {
    NoteOff(NoteOff),
    NoteOn(NoteOn),
    Aftertouch(Aftertouch),
    ControlChange(ControlChange),
    ProgramChange(ProgramChange),
    ChannelAftertouch(ChannelAftertouch),
    PitchBend(PitchBend),
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
#[derive(Deserialize)]
pub(super) struct NoteOff {
    /// The channel the MIDI event belongs to.
    channel: u8,
    /// The key for the note off event.
    key: u8,
    /// The velocity of the note off event.
    velocity: u8,
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
#[derive(Deserialize)]
pub(super) struct NoteOn {
    /// The channel the MIDI event belongs to.
    channel: u8,
    /// The key of the note on event.
    key: u8,
    /// The velocity of the note on event.
    velocity: u8,
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
#[derive(Deserialize)]
pub(super) struct Aftertouch {
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
#[derive(Deserialize)]
pub(super) struct ControlChange {
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
#[derive(Deserialize)]
pub(super) struct ProgramChange {
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
#[derive(Deserialize)]
pub(super) struct ChannelAftertouch {
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
#[derive(Deserialize)]
pub(super) struct PitchBend {
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
        let event = serde_yaml::from_str::<super::Event>(&yaml)?.to_midi_event()?;

        if expected_event == event {
            Ok(())
        } else {
            Err("expected event did not match".into())
        }
    }
}
