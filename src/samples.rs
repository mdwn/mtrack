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

//! MIDI-triggered sample playback system.
//!
//! This module provides:
//! - Sample loading and caching (in-memory for zero-latency playback)
//! - MIDI event to sample trigger matching
//! - Voice management with polyphony limits
//! - Integration with the audio mixer

mod engine;
mod loader;
mod voice;

pub use engine::SampleEngine;

// These types are exported for potential external use and testing
#[allow(unused_imports)]
pub use loader::{LoadedSample, SampleLoader};
#[allow(unused_imports)]
pub use voice::{Voice, VoiceManager};
