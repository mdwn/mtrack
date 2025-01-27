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
use serde::Deserialize;

const DEFAULT_AUDIO_PLAYBACK_DELAY: Duration = Duration::ZERO;

/// A YAML representation of the audio configuration.
#[derive(Deserialize, Clone)]
pub struct Audio {
    /// The audio device.
    device: String,

    /// Controls how long to wait before playback of an audio file starts.
    playback_delay: Option<String>,
}

impl Audio {
    /// New will create a new Audio configuration.
    pub fn new(device: String, playback_delay: Option<String>) -> Audio {
        Audio {
            device,
            playback_delay,
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
            None => Ok(DEFAULT_AUDIO_PLAYBACK_DELAY),
        }
    }
}
