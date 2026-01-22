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
use midly::num::u7;

#[derive(Clone)]
pub enum MidiTransformer {
    NoteMapper(NoteMapper),
    ControlChangeMapper(ControlChangeMapper),
}

impl MidiTransformer {
    pub fn can_process(&self, midi_message: &midly::MidiMessage) -> bool {
        match self {
            MidiTransformer::NoteMapper(note_mapper) => note_mapper.can_process(midi_message),
            MidiTransformer::ControlChangeMapper(control_change_mapper) => {
                control_change_mapper.can_process(midi_message)
            }
        }
    }

    pub fn transform(&self, midi_message: &midly::MidiMessage) -> Vec<midly::MidiMessage> {
        match self {
            MidiTransformer::NoteMapper(note_mapper) => note_mapper.transform(midi_message),
            MidiTransformer::ControlChangeMapper(control_change_mapper) => {
                control_change_mapper.transform(midi_message)
            }
        }
    }
}

/// NoteMapper will map an incoming note and convert it into several notes.
#[derive(Clone)]
pub struct NoteMapper {
    input_note: u7,
    convert_to_notes: Vec<u7>,
}

impl NoteMapper {
    /// Creates a new note mapper.
    pub fn new(input_note: u7, convert_to_notes: Vec<u7>) -> NoteMapper {
        NoteMapper {
            input_note,
            convert_to_notes,
        }
    }
}

impl NoteMapper {
    fn can_process(&self, midi_message: &midly::MidiMessage) -> bool {
        matches!(
            midi_message,
            midly::MidiMessage::NoteOn { .. } | midly::MidiMessage::NoteOff { .. }
        )
    }

    fn transform(&self, midi_message: &midly::MidiMessage) -> Vec<midly::MidiMessage> {
        match midi_message {
            midly::MidiMessage::NoteOn { key, vel } => {
                if *key == self.input_note {
                    self.convert_to_notes
                        .iter()
                        .map(|key| midly::MidiMessage::NoteOn {
                            key: *key,
                            vel: *vel,
                        })
                        .collect()
                } else {
                    vec![*midi_message]
                }
            }
            midly::MidiMessage::NoteOff { key, vel } => {
                if *key == self.input_note {
                    self.convert_to_notes
                        .iter()
                        .map(|key| midly::MidiMessage::NoteOff {
                            key: *key,
                            vel: *vel,
                        })
                        .collect()
                } else {
                    vec![*midi_message]
                }
            }
            _ => vec![*midi_message],
        }
    }
}

/// ControlChangeMapper will map an incoming control change and convert it into several control change messages.
#[derive(Clone)]
pub struct ControlChangeMapper {
    input_controller: u7,
    convert_to_controllers: Vec<u7>,
}

impl ControlChangeMapper {
    /// Creates a new note mapper.
    pub fn new(input_controller: u7, convert_to_controllers: Vec<u7>) -> ControlChangeMapper {
        ControlChangeMapper {
            input_controller,
            convert_to_controllers,
        }
    }
}

impl ControlChangeMapper {
    fn can_process(&self, midi_message: &midly::MidiMessage) -> bool {
        matches!(midi_message, midly::MidiMessage::Controller { .. })
    }

    fn transform(&self, midi_message: &midly::MidiMessage) -> Vec<midly::MidiMessage> {
        match midi_message {
            midly::MidiMessage::Controller { controller, value } => {
                if *controller == self.input_controller {
                    self.convert_to_controllers
                        .iter()
                        .map(|controller| midly::MidiMessage::Controller {
                            controller: *controller,
                            value: *value,
                        })
                        .collect()
                } else {
                    vec![*midi_message]
                }
            }
            _ => vec![*midi_message],
        }
    }
}

#[cfg(test)]
mod test {
    use std::error::Error;

    use midly::num::u7;

    use crate::midi::transform::{ControlChangeMapper, MidiTransformer};

    use super::NoteMapper;

    fn note_on(key: u8, vel: u8) -> midly::MidiMessage {
        midly::MidiMessage::NoteOn {
            key: u7::from_int_lossy(key),
            vel: u7::from_int_lossy(vel),
        }
    }

    fn note_off(key: u8, vel: u8) -> midly::MidiMessage {
        midly::MidiMessage::NoteOff {
            key: u7::from_int_lossy(key),
            vel: u7::from_int_lossy(vel),
        }
    }

    fn control_change(controller: u8, value: u8) -> midly::MidiMessage {
        midly::MidiMessage::Controller {
            controller: u7::from_int_lossy(controller),
            value: u7::from_int_lossy(value),
        }
    }

    #[test]
    fn note_mapper_note_on() -> Result<(), Box<dyn Error>> {
        let mapper = MidiTransformer::NoteMapper(NoteMapper::new(
            u7::from_int_lossy(1),
            u7::slice_from_int(&[2, 3, 4, 5]).to_vec(),
        ));

        assert!(!mapper.can_process(&control_change(1, 27)));
        assert!(mapper.can_process(&note_on(1, 27)));
        let results = mapper.transform(&note_on(1, 27));

        assert_eq!(
            vec![
                note_on(2, 27),
                note_on(3, 27),
                note_on(4, 27),
                note_on(5, 27),
            ],
            results
        );

        Ok(())
    }

    #[test]
    fn note_mapper_note_off() -> Result<(), Box<dyn Error>> {
        let mapper = MidiTransformer::NoteMapper(NoteMapper::new(
            u7::from_int_lossy(1),
            u7::slice_from_int(&[2, 3, 4, 5]).to_vec(),
        ));

        assert!(!mapper.can_process(&control_change(1, 27)));
        assert!(mapper.can_process(&note_off(1, 27)));
        let results = mapper.transform(&note_off(1, 27));

        assert_eq!(
            vec![
                note_off(2, 27),
                note_off(3, 27),
                note_off(4, 27),
                note_off(5, 27),
            ],
            results
        );

        Ok(())
    }

    #[test]
    fn control_change_mapper() -> Result<(), Box<dyn Error>> {
        let mapper = MidiTransformer::ControlChangeMapper(ControlChangeMapper::new(
            u7::from_int_lossy(1),
            u7::slice_from_int(&[2, 3, 4, 5]).to_vec(),
        ));

        assert!(!mapper.can_process(&note_on(1, 0)));
        assert!(mapper.can_process(&control_change(1, 0)));
        let results = mapper.transform(&control_change(1, 0));

        assert_eq!(
            vec![
                control_change(2, 0),
                control_change(3, 0),
                control_change(4, 0),
                control_change(5, 0),
            ],
            results
        );

        Ok(())
    }
}
