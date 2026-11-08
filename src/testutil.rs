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

mod audio;
mod examples;
pub mod lighting;
pub mod markdown;
pub mod mixer;
pub mod net;
pub mod player;
mod polling;

/// How long a test waits on what takes no time at all, but may on a loaded
/// runner: a bound that only stops a test hanging, never one it should come
/// near. Every such wait uses this one.
pub const PATIENCE: std::time::Duration = std::time::Duration::from_secs(30);

pub use audio::{audio_test_utils, write_wav, write_wav_with_bits};
pub use polling::{eventually, eventually_async};
