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
//! Reading audio files and what a song builds from them: sample sources,
//! beat grids, crossfades, the metronome and the pilot. Audio devices and the
//! mixer are the player's.

pub mod beat_grid;
pub mod click_analysis;
pub mod context;
pub mod crossfade;
pub mod format;
pub mod metronome;
pub mod monoclip;
pub mod pilot;
pub mod sample_source;
pub mod track_gains;

pub use context::PlaybackContext;
pub use format::{SampleFormat, TargetFormat};

/// How many samples `duration` lasts at `sample_rate`, to the nearest one
/// (half a sample rounds up). Every conversion of a time to a sample count
/// goes through this, so two that should agree, such as a loop boundary and
/// the crossfade scheduled on it, never differ by a sample from rounding
/// one way here and the other there.
pub fn duration_to_samples(duration: std::time::Duration, sample_rate: u32) -> u64 {
    (duration.as_secs_f64() * sample_rate as f64).round() as u64
}
