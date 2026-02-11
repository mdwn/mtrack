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
// Shared context for audio playback (song and sample sources). Carries
// format, buffer sizes, and shared pools so call sites don't thread many
// separate arguments.
//

use std::sync::Arc;

use crate::audio::format::TargetFormat;
use crate::audio::sample_source::BufferFillPool;

/// Context passed into playback and source-creation paths so they can
/// obtain target format, buffer size, and shared resources (e.g. buffer
/// fill pool) without many separate parameters.
#[derive(Clone)]
pub struct PlaybackContext {
    /// Target sample rate, format, and bit depth for output.
    pub target_format: TargetFormat,
    /// Device buffer size in frames (used for BufferedSampleSource capacity
    /// and for file decode buffer size).
    pub buffer_size: usize,
    /// Shared pool for prefilling BufferedSampleSource. If None, sources
    /// are not wrapped in BufferedSampleSource.
    pub buffer_fill_pool: Option<Arc<BufferFillPool>>,
}

impl PlaybackContext {
    /// Builds a context from the given format, buffer size, and optional pool.
    pub fn new(
        target_format: TargetFormat,
        buffer_size: usize,
        buffer_fill_pool: Option<Arc<BufferFillPool>>,
    ) -> Self {
        Self {
            target_format,
            buffer_size,
            buffer_fill_pool,
        }
    }
}
