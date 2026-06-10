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
//! Benchmarks for the audio mixer hot path (`AudioMixer::process_into_output`).
//!
//! These measure the per-callback mixing cost for representative live rigs.
//! Compare against the saved baseline when touching the mixer:
//!
//! ```sh
//! cargo bench --bench mixer -- --save-baseline pre-gain   # before changes
//! cargo bench --bench mixer -- --baseline pre-gain        # after changes
//! ```
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};

use mtrack::audio::mixer::{ActiveSource, AudioMixer};
use mtrack::audio::sample_source::{
    ChannelMappedSampleSource, ChannelMappedSource, MemorySampleSource,
};
use mtrack::playsync::CancelHandle;

/// Frames per simulated device callback.
const BUF_FRAMES: usize = 512;
const SAMPLE_RATE: u32 = 48000;

/// Describes one source in a benchmark scenario: its channel count and the
/// (label, output channels) mapping for each source channel.
struct SourceSpec {
    channels: u16,
    mappings: Vec<(String, Vec<u16>)>,
}

fn build_source(spec: &SourceSpec, frames: usize) -> ActiveSource {
    let samples: Vec<f32> = (0..frames * spec.channels as usize)
        .map(|i| ((i % 97) as f32 / 97.0) - 0.5)
        .collect();
    let memory =
        MemorySampleSource::from_shared(Arc::new(samples), spec.channels, SAMPLE_RATE, 1.0);
    let labels: Vec<Vec<String>> = spec
        .mappings
        .iter()
        .map(|(label, _)| vec![label.clone()])
        .collect();
    let source: Box<dyn ChannelMappedSampleSource> = Box::new(ChannelMappedSource::new(
        Box::new(memory),
        labels,
        spec.channels,
    ));
    let track_mappings: HashMap<String, Vec<u16>> = spec.mappings.iter().cloned().collect();

    ActiveSource {
        id: 0,
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
    }
}

fn build_mixer(specs: &[SourceSpec], output_channels: u16) -> AudioMixer {
    let mixer = AudioMixer::new(output_channels, SAMPLE_RATE);
    for (id, spec) in specs.iter().enumerate() {
        let mut source = build_source(spec, BUF_FRAMES);
        source.id = id as u64;
        mixer.add_source(source);
    }
    mixer
}

/// 8 stereo sources routed to a 16-channel interface (typical live rig).
fn specs_8_stereo_16ch() -> Vec<SourceSpec> {
    (0..8u16)
        .map(|i| SourceSpec {
            channels: 2,
            mappings: vec![
                (format!("t{i}-l"), vec![i * 2 + 1]),
                (format!("t{i}-r"), vec![i * 2 + 2]),
            ],
        })
        .collect()
}

/// One 16-channel source (single multichannel song file) to 16 outputs.
fn specs_1x16ch_16ch() -> Vec<SourceSpec> {
    vec![SourceSpec {
        channels: 16,
        mappings: (0..16u16).map(|i| (format!("t{i}"), vec![i + 1])).collect(),
    }]
}

/// 32 mono sources to 32 outputs (stress).
fn specs_32_mono_32ch() -> Vec<SourceSpec> {
    (0..32u16)
        .map(|i| SourceSpec {
            channels: 1,
            mappings: vec![(format!("t{i}"), vec![i + 1])],
        })
        .collect()
}

fn bench_scenario(c: &mut Criterion, name: &str, specs: Vec<SourceSpec>, output_channels: u16) {
    let mut group = c.benchmark_group("mixer");
    group.throughput(Throughput::Elements(BUF_FRAMES as u64));
    group.bench_function(name, |b| {
        b.iter_batched(
            || {
                let mixer = build_mixer(&specs, output_channels);
                let output = vec![0.0f32; BUF_FRAMES * output_channels as usize];
                (mixer, output)
            },
            |(mixer, mut output)| {
                mixer.process_into_output(&mut output, BUF_FRAMES);
                output
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn mixer_benches(c: &mut Criterion) {
    bench_scenario(c, "8_stereo_sources_16ch_out", specs_8_stereo_16ch(), 16);
    bench_scenario(c, "1_source_16ch_16ch_out", specs_1x16ch_16ch(), 16);
    bench_scenario(c, "32_mono_sources_32ch_out", specs_32_mono_32ch(), 32);
}

criterion_group!(benches, mixer_benches);
criterion_main!(benches);
