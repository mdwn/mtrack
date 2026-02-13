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
pub(crate) fn next_source_id() -> u64 {
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
