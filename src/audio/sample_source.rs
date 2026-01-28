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
pub mod audio;
pub mod channel_mapped;
pub mod error;
pub mod factory;
pub mod memory;
pub mod traits;
pub mod transcoder;

#[cfg(test)]
mod tests;

// Re-exports for use by other modules
pub use channel_mapped::create_channel_mapped_sample_source;
#[cfg(test)]
pub use channel_mapped::ChannelMappedSource;
pub use factory::create_sample_source_from_file;
pub use traits::ChannelMappedSampleSource;

#[cfg(test)]
pub use memory::MemorySampleSource;
