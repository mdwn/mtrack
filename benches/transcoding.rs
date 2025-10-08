// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use hound::SampleFormat;
use mtrack::audio::{sample_source::AudioTranscoder, TargetFormat};
use std::time::Duration;

fn generate_test_audio(duration_seconds: f32, sample_rate: u32) -> Vec<f32> {
    let num_samples = (duration_seconds * sample_rate as f32) as usize;
    let mut samples = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        // Generate a complex signal with multiple frequencies
        let sample = 0.3 * (2.0 * std::f32::consts::PI * 440.0 * t).sin() +  // A4
                    0.2 * (2.0 * std::f32::consts::PI * 880.0 * t).sin() +  // A5
                    0.1 * (2.0 * std::f32::consts::PI * 1320.0 * t).sin(); // E6
        samples.push(sample);
    }

    samples
}

fn benchmark_resampling(c: &mut Criterion) {
    let mut group = c.benchmark_group("resampling");

    // Test different resampling ratios
    let test_cases = vec![
        ("48kHz_to_44.1kHz", 48000, 44100),
        ("44.1kHz_to_48kHz", 44100, 48000),
        ("96kHz_to_44.1kHz", 96000, 44100),
        ("44.1kHz_to_96kHz", 44100, 96000),
        ("48kHz_to_96kHz", 48000, 96000),
        ("96kHz_to_48kHz", 96000, 48000),
    ];

    for (name, source_rate, target_rate) in test_cases {
        let source_format = TargetFormat::new(source_rate, SampleFormat::Float, 32).unwrap();
        let target_format = TargetFormat::new(target_rate, SampleFormat::Float, 32).unwrap();
        let mut converter = AudioTranscoder::new(&source_format, &target_format, 1).unwrap();

        // Generate 1 second of test audio
        let input_samples = generate_test_audio(1.0, source_rate);

        group.bench_function(name, |b| {
            b.iter(|| {
                let result = converter.resample_block(black_box(&input_samples));
                black_box(result)
            })
        });
    }

    group.finish();
}

fn benchmark_multichannel_resampling(c: &mut Criterion) {
    let mut group = c.benchmark_group("multichannel_resampling");

    // Test different channel counts
    let channel_tests = vec![
        ("stereo_48kHz_to_44.1kHz", 2, 48000, 44100),
        ("quad_44.1kHz_to_48kHz", 4, 44100, 48000),
        ("stereo_96kHz_to_44.1kHz", 2, 96000, 44100),
    ];

    for (name, channels, source_rate, target_rate) in channel_tests {
        let source_format = TargetFormat::new(source_rate, SampleFormat::Float, 32).unwrap();
        let target_format = TargetFormat::new(target_rate, SampleFormat::Float, 32).unwrap();
        let mut converter = AudioTranscoder::new(&source_format, &target_format, channels).unwrap();

        // Generate 0.5 seconds of multichannel test audio
        let mut input_samples = Vec::new();
        for i in 0..((0.5 * source_rate as f32) as usize) {
            for ch in 0..channels {
                let t = i as f32 / source_rate as f32;
                let freq = 440.0 * (ch as f32 + 1.0); // Different frequency per channel
                let sample = 0.3 * (2.0 * std::f32::consts::PI * freq * t).sin();
                input_samples.push(sample);
            }
        }

        group.bench_function(name, |b| {
            b.iter(|| {
                let result = converter.resample_block(black_box(&input_samples));
                black_box(result)
            })
        });
    }

    group.finish();
}

fn benchmark_different_input_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("input_size_scaling");

    let source_format = TargetFormat::new(48000, SampleFormat::Float, 32).unwrap();
    let target_format = TargetFormat::new(44100, SampleFormat::Float, 32).unwrap();
    let mut converter = AudioTranscoder::new(&source_format, &target_format, 1).unwrap();

    // Test different input sizes
    let durations = vec![0.1, 0.5, 1.0, 2.0, 5.0]; // seconds

    for duration in durations {
        let input_samples = generate_test_audio(duration, 48000);

        group.bench_function(BenchmarkId::new("duration", duration), |b| {
            b.iter(|| {
                let result = converter.resample_block(black_box(&input_samples));
                black_box(result)
            })
        });
    }

    group.finish();
}

fn benchmark_format_conversion(c: &mut Criterion) {
    let mut group = c.benchmark_group("format_conversion");

    // Test different format conversions
    let format_tests = vec![
        (
            "int16_to_float32",
            SampleFormat::Int,
            16,
            SampleFormat::Float,
            32,
        ),
        (
            "int24_to_float32",
            SampleFormat::Int,
            24,
            SampleFormat::Float,
            32,
        ),
        (
            "int32_to_float32",
            SampleFormat::Int,
            32,
            SampleFormat::Float,
            32,
        ),
        (
            "float32_to_int16",
            SampleFormat::Float,
            32,
            SampleFormat::Int,
            16,
        ),
        (
            "float32_to_int24",
            SampleFormat::Float,
            32,
            SampleFormat::Int,
            24,
        ),
    ];

    for (name, source_format, source_bits, target_format, target_bits) in format_tests {
        let source = TargetFormat::new(44100, source_format, source_bits).unwrap();
        let target = TargetFormat::new(44100, target_format, target_bits).unwrap();
        let mut converter = AudioTranscoder::new(&source, &target, 1).unwrap();

        // Generate 1 second of test audio
        let input_samples = generate_test_audio(1.0, 44100);

        group.bench_function(name, |b| {
            b.iter(|| {
                let result = converter.resample_block(black_box(&input_samples));
                black_box(result)
            })
        });
    }

    group.finish();
}

fn benchmark_real_time_performance(c: &mut Criterion) {
    let mut group = c.benchmark_group("real_time_performance");

    // Test if we can process audio faster than real-time
    let source_format = TargetFormat::new(48000, SampleFormat::Float, 32).unwrap();
    let target_format = TargetFormat::new(44100, SampleFormat::Float, 32).unwrap();
    let mut converter = AudioTranscoder::new(&source_format, &target_format, 2).unwrap();

    // Generate 1 second of stereo audio (48kHz = 96,000 samples)
    let mut input_samples = Vec::new();
    for i in 0..48000 {
        let t = i as f32 / 48000.0;
        let left = 0.3 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
        let right = 0.3 * (2.0 * std::f32::consts::PI * 880.0 * t).sin();
        input_samples.push(left);
        input_samples.push(right);
    }

    group.bench_function("stereo_1_second", |b| {
        b.iter(|| {
            let result = converter.resample_block(black_box(&input_samples));
            black_box(result)
        })
    });

    // Test processing time vs real-time duration
    group.bench_function("real_time_ratio", |b| {
        b.iter(|| {
            let start = std::time::Instant::now();
            let result = converter.resample_block(black_box(&input_samples));
            let duration = start.elapsed();
            let real_time_duration = Duration::from_secs_f32(1.0);
            let ratio = duration.as_secs_f32() / real_time_duration.as_secs_f32();
            black_box((result, ratio))
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_resampling,
    benchmark_multichannel_resampling,
    benchmark_different_input_sizes,
    benchmark_format_conversion,
    benchmark_real_time_performance
);
criterion_main!(benches);
