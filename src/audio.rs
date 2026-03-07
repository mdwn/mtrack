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
use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{error::Error, fmt, sync::Arc, time::Duration};

use crate::config;
use crate::playsync::CancelHandle;
use crate::songs::Song;
use std::collections::HashMap;
use std::sync::Barrier;

pub mod context;
pub mod cpal;
pub mod format;
pub mod mixer;
pub mod mock;
pub mod sample_source;
mod thread_priority;

// Re-export the format types for backward compatibility
pub use context::PlaybackContext;
pub use format::{SampleFormat, TargetFormat};

/// Global source ID counter shared by song playback and sample triggers so IDs are unique.
static SOURCE_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Returns the next unique source ID for the mixer. Used by both song play_from and sample engine.
pub fn next_source_id() -> u64 {
    SOURCE_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Type alias for the channel sender used to add sources to the mixer.
pub type SourceSender = crossbeam_channel::Sender<mixer::ActiveSource>;

pub trait Device: Any + fmt::Display + std::marker::Send + std::marker::Sync {
    /// Plays the given song through the audio interface, starting from a specific time.
    fn play_from(
        &self,
        song: Arc<Song>,
        mappings: &HashMap<String, Vec<u16>>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
        start_time: Duration,
    ) -> Result<(), Box<dyn Error>>;

    /// Gets the mixer for adding triggered samples.
    /// Returns None if the device doesn't support triggered samples.
    fn mixer(&self) -> Option<Arc<mixer::AudioMixer>> {
        None
    }

    /// Gets the source sender for adding triggered samples without lock contention.
    /// Returns None if the device doesn't support triggered samples.
    fn source_sender(&self) -> Option<SourceSender> {
        None
    }

    #[cfg(test)]
    fn to_mock(&self) -> Result<Arc<mock::Device>, Box<dyn Error>>;
}

/// Finds a cpal input device by name, searching all available hosts.
pub(crate) fn find_input_device(name: &str) -> Result<::cpal::Device, Box<dyn Error>> {
    use ::cpal::traits::{DeviceTrait, HostTrait};

    for host_id in ::cpal::available_hosts() {
        let host = ::cpal::host_from_id(host_id)?;
        let devices = match host.input_devices() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(
                    host = host_id.name(),
                    error = %e,
                    "Failed to list input devices for host"
                );
                continue;
            }
        };

        for device in devices {
            let device_id = match device.id() {
                Ok(id) => id.to_string(),
                Err(_) => continue,
            };
            if device_id.trim() == name.trim() {
                return Ok(device);
            }
        }
    }

    Err(format!("No input device found with name '{}'", name).into())
}

/// Lists devices known to cpal.
pub fn list_devices() -> Result<Vec<Box<dyn Device>>, Box<dyn Error>> {
    cpal::Device::list()
}

/// Gets a device with the given name.
pub fn get_device(config: Option<config::Audio>) -> Result<Arc<dyn Device>, Box<dyn Error>> {
    let config = match config {
        Some(config) => config,
        None => return Err("there must be an audio device specified".into()),
    };

    let device = config.device();
    if device.starts_with("mock") {
        return Ok(Arc::new(mock::Device::get(device)));
    };

    Ok(Arc::new(cpal::Device::get(config)?))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn get_device_none_returns_error() {
        let result = get_device(None);
        match result {
            Err(e) => assert!(e.to_string().contains("audio device specified")),
            Ok(_) => panic!("expected error for None config"),
        }
    }

    #[test]
    fn get_device_mock_returns_ok() {
        let config = config::Audio::new("mock-device");
        let result = get_device(Some(config));
        assert!(result.is_ok());
    }

    #[test]
    fn default_mixer_returns_none() {
        struct Dummy;
        impl fmt::Display for Dummy {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "dummy")
            }
        }
        impl Device for Dummy {
            fn play_from(
                &self,
                _song: Arc<Song>,
                _mappings: &HashMap<String, Vec<u16>>,
                _cancel_handle: CancelHandle,
                _play_barrier: Arc<Barrier>,
                _start_time: Duration,
            ) -> Result<(), Box<dyn Error>> {
                Ok(())
            }
            fn to_mock(&self) -> Result<Arc<mock::Device>, Box<dyn Error>> {
                Err("not a mock".into())
            }
        }
        let d = Dummy;
        assert!(d.mixer().is_none());
        assert!(d.source_sender().is_none());
    }

    #[test]
    fn next_source_id_increments() {
        let id1 = next_source_id();
        let id2 = next_source_id();
        assert!(id2 > id1);
    }
}
