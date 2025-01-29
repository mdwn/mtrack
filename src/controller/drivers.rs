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
use std::{error::Error, sync::Arc};

use tracing::error;

use crate::{config, controller, midi};

use super::Driver;

pub(super) fn driver_from_midi_config(
    config: &config::MidiController,
    midi_device: Option<Arc<dyn midi::Device>>,
) -> Result<controller::midi::Driver, Box<dyn Error>> {
    match midi_device {
        Some(midi_device) => Ok(controller::midi::Driver::new(
            midi_device,
            config.play()?,
            config.prev()?,
            config.next()?,
            config.stop()?,
            config.all_songs()?,
            config.playlist()?,
        )),
        None => Err("No MIDI device found for MIDI controller.".into()),
    }
}

/// Creates a controller driver from the config.
pub(super) fn driver(
    config: config::Controller,
    midi_device: Option<Arc<dyn midi::Device>>,
) -> Result<Arc<dyn Driver>, Box<dyn Error>> {
    match config {
        config::Controller::Midi(config) => match midi_device {
            Some(midi_device) => match driver_from_midi_config(&config, Some(midi_device)) {
                Ok(driver) => Ok(Arc::new(driver)),
                Err(error) => Err(error),
            },
            None => Err("No MIDI device found for MIDI controller.".into()),
        },
        config::Controller::Keyboard => Ok(Arc::new(controller::keyboard::Driver::new())),
        config::Controller::Multi(vec) => Ok(Arc::new(controller::multi::Driver::new(
            vec.iter()
                .filter_map(|d| match d {
                    (_key, config::Controller::Keyboard) => {
                        Some(controller::multi::SubDriver::Keyboard(Arc::new(
                            controller::keyboard::Driver::new(),
                        )))
                    }

                    (_key, config::Controller::Midi(midi_controller)) => {
                        let midi_driver_result =
                            driver_from_midi_config(midi_controller, midi_device.clone());
                        match midi_driver_result {
                            Ok(driver) => {
                                Some(controller::multi::SubDriver::Midi(Arc::new(driver)))
                            }
                            Err(_e) => None,
                        }
                    }

                    (_key, config::Controller::Multi(_vec)) => {
                        error!("Recursive multi controllers are not supported");
                        None
                    }
                })
                .collect::<Vec<_>>(),
        ))),
    }
}
