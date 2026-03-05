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
use std::error::Error;

use midly::live::LiveEvent;
use serde::{Deserialize, Serialize};

use self::midi::ToMidiEvent;

use super::midi;

/// The configuration for emitting status events.
#[derive(Deserialize, Serialize, Clone)]
pub struct StatusEvents {
    /// The events to emit to clear the status.
    off_events: Vec<midi::Event>,
    /// The events to emit to indicate that the player is idling and waiting for input.
    idling_events: Vec<midi::Event>,
    /// The events to emit to indicate that the player is currently playing.
    playing_events: Vec<midi::Event>,
}

impl StatusEvents {
    /// Gets the off events.
    pub fn off_events(&self) -> Result<Vec<LiveEvent<'static>>, Box<dyn Error>> {
        self.off_events
            .iter()
            .map(|event| event.to_midi_event())
            .collect()
    }

    /// Gets the idling events.
    pub fn idling_events(&self) -> Result<Vec<LiveEvent<'static>>, Box<dyn Error>> {
        self.idling_events
            .iter()
            .map(|event| event.to_midi_event())
            .collect()
    }

    /// Gets the playing events.
    pub fn playing_events(&self) -> Result<Vec<LiveEvent<'static>>, Box<dyn Error>> {
        self.playing_events
            .iter()
            .map(|event| event.to_midi_event())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use config::{Config, File, FileFormat};

    use super::*;

    fn make_status_events(yaml: &str) -> StatusEvents {
        Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap()
    }

    #[test]
    fn off_events_converts_to_midi() {
        let se = make_status_events(
            r#"
            off_events:
              - type: note_on
                channel: 1
                key: 60
                velocity: 0
            idling_events: []
            playing_events: []
            "#,
        );
        let events = se.off_events().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LiveEvent::Midi { message, .. } => match message {
                midly::MidiMessage::NoteOn { key, vel } => {
                    assert_eq!(u8::from(*key), 60);
                    assert_eq!(u8::from(*vel), 0);
                }
                other => panic!("expected NoteOn, got {:?}", other),
            },
            other => panic!("expected Midi event, got {:?}", other),
        }
    }

    #[test]
    fn idling_events_converts_to_midi() {
        let se = make_status_events(
            r#"
            off_events: []
            idling_events:
              - type: control_change
                channel: 1
                controller: 7
                value: 100
            playing_events: []
            "#,
        );
        let events = se.idling_events().unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn playing_events_converts_to_midi() {
        let se = make_status_events(
            r#"
            off_events: []
            idling_events: []
            playing_events:
              - type: program_change
                channel: 2
                program: 5
            "#,
        );
        let events = se.playing_events().unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn multiple_events() {
        let se = make_status_events(
            r#"
            off_events:
              - type: note_on
                channel: 1
                key: 60
                velocity: 0
              - type: note_on
                channel: 1
                key: 61
                velocity: 0
              - type: note_on
                channel: 1
                key: 62
                velocity: 0
            idling_events: []
            playing_events: []
            "#,
        );
        assert_eq!(se.off_events().unwrap().len(), 3);
    }

    #[test]
    fn empty_events() {
        let se = make_status_events(
            r#"
            off_events: []
            idling_events: []
            playing_events: []
            "#,
        );
        assert!(se.off_events().unwrap().is_empty());
        assert!(se.idling_events().unwrap().is_empty());
        assert!(se.playing_events().unwrap().is_empty());
    }
}
