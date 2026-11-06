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

//! Sources for the mixer's and the output streams' tests.

use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc},
};

use crate::{
    audio::{
        mixer::ActiveSource,
        sample_source::{ChannelMappedSampleSource, ChannelMappedSource, MemorySampleSource},
    },
    playsync::CancelHandle,
};

/// `samples` (interleaved, `channels` of them) at 44.1kHz, each channel
/// carrying the tracks `labels` gives it.
pub fn memory_source(
    samples: Vec<f32>,
    channels: u16,
    labels: Vec<Vec<String>>,
) -> Box<dyn ChannelMappedSampleSource + Send + Sync> {
    Box::new(ChannelMappedSource::new(
        Box::new(MemorySampleSource::new(samples, channels, 44100)),
        labels,
        channels,
    ))
}

/// `source` as the mixer plays it, under a fresh id, its channel count
/// cached as adding it caches it.
pub fn active_source(
    source: Box<dyn ChannelMappedSampleSource + Send + Sync>,
    track_mappings: HashMap<String, Vec<u16>>,
) -> ActiveSource {
    let cached = source.source_channel_count();
    ActiveSource {
        cached_source_channel_count: cached,
        ..active_source_uncached(crate::audio::next_source_id(), source, track_mappings)
    }
}

/// `source` as the mixer plays it, under `id`, its channel count not yet
/// cached (0).
pub fn active_source_uncached(
    id: u64,
    source: Box<dyn ChannelMappedSampleSource + Send + Sync>,
    track_mappings: HashMap<String, Vec<u16>>,
) -> ActiveSource {
    ActiveSource {
        id,
        source,
        track_mappings,
        channel_mappings: Vec::new(),
        cached_source_channel_count: 0,
        is_finished: Arc::new(AtomicBool::new(false)),
        cancel_handle: CancelHandle::new(),
        start_at_sample: None,
        cancel_at_sample: None,
        gain: Default::default(),
        gain_envelope: None,
        break_envelope: None,
    }
}
