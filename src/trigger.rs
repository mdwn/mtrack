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

//! Audio trigger detection for piezo drum triggers.
//!
//! Captures audio input via cpal, detects transient hits using per-channel
//! state machines, and produces source-agnostic `TriggerAction` events.

mod detector;
mod engine;
mod filter;

pub use engine::TriggerEngine;

/// Converts milliseconds to samples, rounding up.
fn ms_to_samples(ms: u32, sample_rate: u32) -> u32 {
    ((ms as f64) * (sample_rate as f64) / 1000.0).ceil() as u32
}
