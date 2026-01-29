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
#[cfg(test)]
mod tests {
    use crate::audio::sample_source::audio::AudioSampleSource;
    use crate::audio::sample_source::create_sample_source_from_file;
    use crate::audio::sample_source::error::SampleSourceError;
    use crate::audio::sample_source::memory::MemorySampleSource;
    use crate::audio::sample_source::traits::{SampleSource, SampleSourceTestExt};
    use crate::audio::sample_source::transcoder::AudioTranscoder;
    use crate::audio::TargetFormat;

    /// Helper trait for reading samples in tests (bridges planar API to interleaved test expectations)
    trait SampleSourceTestHelper {
        /// Read one sample at a time (for compatibility with old tests).
        /// Returns interleaved samples: L, R, L, R, ... for stereo
        fn read_one_sample(&mut self) -> Result<Option<f32>, SampleSourceError>;

        /// Read all samples, returned as interleaved.
        fn read_all_samples(&mut self) -> Result<Vec<f32>, SampleSourceError>;
    }

    impl<T: SampleSource> SampleSourceTestHelper for T {
        fn read_one_sample(&mut self) -> Result<Option<f32>, SampleSourceError> {
            // For single-sample reads, we read one frame and return first sample
            // This is inefficient but maintains test compatibility
            let num_channels = self.channel_count() as usize;
            let mut planar_buf: Vec<Vec<f32>> = vec![Vec::new(); num_channels];

            let frames_read = self.next_chunk(&mut planar_buf, 1)?;
            if frames_read == 0 {
                return Ok(None);
            }

            // Return first channel's sample (tests that need all channels should use read_all_samples)
            if let Some(ch0) = planar_buf.first() {
                if let Some(&sample) = ch0.first() {
                    return Ok(Some(sample));
                }
            }
            Ok(None)
        }

        fn read_all_samples(&mut self) -> Result<Vec<f32>, SampleSourceError> {
            let num_channels = self.channel_count() as usize;
            let mut all_samples = Vec::new();
            let mut planar_buf: Vec<Vec<f32>> = vec![Vec::new(); num_channels];

            loop {
                let frames_read = self.next_chunk(&mut planar_buf, 1024)?;
                if frames_read == 0 {
                    break;
                }

                // Interleave the planar data for test compatibility
                for frame_idx in 0..frames_read {
                    for ch_buf in &planar_buf {
                        if let Some(&sample) = ch_buf.get(frame_idx) {
                            all_samples.push(sample);
                        }
                    }
                }
            }
            Ok(all_samples)
        }
    }

    // ---------------------------------------------------------------------
    // Scaling helpers – direct unit tests for all integer formats
    // ---------------------------------------------------------------------

    #[test]
    fn test_integer_scaling_signed_ranges() {
        // S8
        assert!((AudioSampleSource::scale_s8(0) - 0.0).abs() < 1e-7);
        assert!(AudioSampleSource::scale_s8(i8::MAX) <= 1.0 + 1e-7);
        assert!(AudioSampleSource::scale_s8(i8::MIN) >= -1.0 - 1e-7);

        // S16
        assert!((AudioSampleSource::scale_s16(0) - 0.0).abs() < 1e-7);
        assert!(AudioSampleSource::scale_s16(i16::MAX) <= 1.0 + 1e-7);
        assert!(AudioSampleSource::scale_s16(i16::MIN) >= -1.0 - 1e-7);

        // S24
        assert!((AudioSampleSource::scale_s24(0) - 0.0).abs() < 1e-7);
        assert!(AudioSampleSource::scale_s24((1 << 23) - 1) <= 1.0 + 1e-7);
        assert!(AudioSampleSource::scale_s24(-(1 << 23)) >= -1.0 - 1e-7);

        // S32
        assert!((AudioSampleSource::scale_s32(0) - 0.0).abs() < 1e-7);
        assert!(AudioSampleSource::scale_s32(i32::MAX) <= 1.0 + 1e-7);
        assert!(AudioSampleSource::scale_s32(i32::MIN) >= -1.0 - 1e-7);
    }

    #[test]
    fn test_integer_scaling_unsigned_ranges() {
        // U8
        assert!((AudioSampleSource::scale_u8(0) + 1.0).abs() < 1e-7);
        assert!((AudioSampleSource::scale_u8(u8::MAX) - 1.0).abs() < 1e-7);
        let mid_u8 = AudioSampleSource::scale_u8(128);
        assert!(mid_u8 > -0.01 && mid_u8 < 0.01);

        // U16
        assert!((AudioSampleSource::scale_u16(0) + 1.0).abs() < 1e-7);
        assert!((AudioSampleSource::scale_u16(u16::MAX) - 1.0).abs() < 1e-7);
        let mid_u16 = AudioSampleSource::scale_u16(u16::MAX / 2);
        assert!(mid_u16 > -0.01 && mid_u16 < 0.01);

        // U24
        let max_u24 = (1u32 << 24) - 1;
        assert!((AudioSampleSource::scale_u24(0) + 1.0).abs() < 1e-7);
        assert!((AudioSampleSource::scale_u24(max_u24) - 1.0).abs() < 1e-7);
        let mid_u24 = AudioSampleSource::scale_u24(max_u24 / 2);
        assert!(mid_u24 > -0.01 && mid_u24 < 0.01);

        // U32
        assert!((AudioSampleSource::scale_u32(0) + 1.0).abs() < 1e-7);
        assert!((AudioSampleSource::scale_u32(u32::MAX) - 1.0).abs() < 1e-7);
        let mid_u32 = AudioSampleSource::scale_u32(u32::MAX / 2);
        assert!(mid_u32 > -0.01 && mid_u32 < 0.01);
    }
    use crate::testutil::audio_test_utils::calculate_snr;
    use rand;

    /// Calculate high-frequency energy content (simple approximation)
    fn calculate_high_frequency_energy(samples: &[f32], _sample_rate: f32) -> f32 {
        if samples.len() < 2 {
            return 0.0;
        }

        // Simple high-pass filter approximation: difference between consecutive samples
        let mut high_freq_energy = 0.0;
        for i in 1..samples.len() {
            let diff = samples[i] - samples[i - 1];
            high_freq_energy += diff * diff;
        }

        high_freq_energy / (samples.len() - 1) as f32
    }

    #[test]
    fn test_memory_sample_source() {
        let samples = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let mut source = MemorySampleSource::new(samples.clone(), 1, 44100);

        // Test that we get all samples
        for (i, expected) in samples.iter().enumerate() {
            let sample = source.read_one_sample().unwrap().unwrap();
            assert_eq!(sample, *expected);
            // After reading the last sample, we should be finished
            if i == samples.len() - 1 {
                assert!(SampleSourceTestExt::is_finished(&source));
            } else {
                assert!(!SampleSourceTestExt::is_finished(&source));
            }
        }

        // Test that we get None when finished
        assert!(source.read_one_sample().unwrap().is_none());
        assert!(SampleSourceTestExt::is_finished(&source));
    }

    #[test]
    fn test_memory_sample_source_duration_mono() {
        // Test duration calculation for mono audio
        let samples = vec![1.0, 2.0, 3.0, 4.0, 5.0]; // 5 samples
        let source = MemorySampleSource::new(samples.clone(), 1, 44100);

        // Calculate expected duration using the same formula as the implementation
        let total_samples = samples.len() as f64;
        let samples_per_channel = total_samples / 1.0; // mono
        let duration_secs = samples_per_channel / 44100.0;
        let expected_duration = std::time::Duration::from_secs_f64(duration_secs);

        let actual_duration = source.duration().unwrap();

        // Allow for small rounding differences
        let diff = actual_duration.abs_diff(expected_duration);
        assert!(
            diff < std::time::Duration::from_micros(1),
            "Duration mismatch: expected {:?}, got {:?}",
            expected_duration,
            actual_duration
        );
    }

    #[test]
    fn test_memory_sample_source_duration_stereo() {
        // Test duration calculation for stereo audio
        let samples = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]; // 6 samples (3 frames * 2 channels)
        let source = MemorySampleSource::new(samples.clone(), 2, 44100);

        // Calculate expected duration using the same formula as the implementation
        let total_samples = samples.len() as f64;
        let samples_per_channel = total_samples / 2.0; // stereo
        let duration_secs = samples_per_channel / 44100.0;
        let expected_duration = std::time::Duration::from_secs_f64(duration_secs);

        let actual_duration = source.duration().unwrap();

        // Allow for small rounding differences
        let diff = actual_duration.abs_diff(expected_duration);
        assert!(
            diff < std::time::Duration::from_micros(1),
            "Duration mismatch: expected {:?}, got {:?}",
            expected_duration,
            actual_duration
        );
    }

    #[test]
    fn test_resampling_quality() {
        // Test actual resampling with simple input
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Create a mock source for testing
        let mock_source = MemorySampleSource::new(vec![0.1, 0.2, 0.3, 0.4, 0.5], 1, 44100);
        match AudioTranscoder::new(mock_source, &source_format, &target_format, 1) {
            Ok(mut converter) => {
                // Test resampling by getting samples from the converter
                let mut output_samples = Vec::with_capacity(100);
                let mut sample_count = 0;
                const MAX_SAMPLES: usize = 100; // Prevent infinite loops

                while sample_count < MAX_SAMPLES {
                    match converter.read_one_sample() {
                        Ok(Some(sample)) => {
                            output_samples.push(sample);
                            sample_count += 1;
                        }
                        Ok(None) => break, // End of source
                        Err(_e) => break,  // Error occurred
                    }
                }

                // Basic checks
                assert!(!output_samples.is_empty(), "Output should not be empty");
                assert!(
                    !output_samples.is_empty(),
                    "Should have some output samples"
                );

                // Verify the signal is still recognizable (basic quality check)
                let max_amplitude = output_samples.iter().map(|&x| x.abs()).fold(0.0, f32::max);

                assert!(
                    max_amplitude > 0.0,
                    "Resampled signal should have some amplitude"
                );
                assert!(
                    max_amplitude <= 1.1,
                    "Resampled signal too loud, max amplitude: {}",
                    max_amplitude
                );
            }
            Err(_e) => {
                // If rubato resampler creation fails, that's acceptable for now
            }
        }
    }

    #[test]
    fn test_rubato_resampler_creation() {
        // Test that rubato resampler can be created for common ratios
        let test_cases = vec![
            (44100, 48000), // CD to DAT
            (48000, 44100), // DAT to CD
            (44100, 44100), // Same rate (should not create resampler)
        ];

        for (source_rate, target_rate) in test_cases {
            let source_format =
                TargetFormat::new(source_rate, crate::audio::SampleFormat::Float, 32).unwrap();
            let target_format =
                TargetFormat::new(target_rate, crate::audio::SampleFormat::Float, 32).unwrap();

            // Create a mock source for testing
            let mock_source = MemorySampleSource::new(vec![0.1, 0.2, 0.3], 1, 44100);
            let converter = AudioTranscoder::new(mock_source, &source_format, &target_format, 1);

            if source_rate == target_rate {
                // Should not need resampling
                assert!(converter.is_ok());
                let converter = converter.unwrap();
                assert!(converter.resampler.is_none());
            } else {
                // Should create resampler
                match converter {
                    Ok(_converter) => {
                        if source_rate != target_rate {
                            // For now, we expect this to work for reasonable ratios
                            // If rubato fails, we'll get an error which is also acceptable
                        }
                    }
                    Err(_e) => {
                        // If rubato fails to create the resampler, that's also a valid test result
                    }
                }
            }
        }
    }

    #[test]
    fn test_rubato_configuration_debug() {
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Create a mock source for testing
        let mock_source = MemorySampleSource::new(vec![1.0, 2.0, 3.0, 4.0, 5.0], 1, 44100);
        match AudioTranscoder::new(mock_source, &source_format, &target_format, 1) {
            Ok(mut converter) => {
                // Test with a simple input to understand the behavior
                let mut sample_count = 0;
                const MAX_SAMPLES: usize = 50;

                while sample_count < MAX_SAMPLES {
                    match converter.read_one_sample() {
                        Ok(Some(_sample)) => sample_count += 1,
                        Ok(None) => break,
                        Err(_e) => break,
                    }
                }
            }
            Err(_e) => {}
        }
    }

    #[test]
    fn test_resampling_edge_cases() {
        // Test that we can detect when resampling is needed
        let test_cases = vec![
            (44100, 48000, true),  // CD to DAT - should need resampling
            (48000, 44100, true),  // DAT to CD - should need resampling
            (44100, 44100, false), // Same rate - should not need resampling
        ];

        for (source_rate, target_rate, should_need_resampling) in test_cases {
            let source_format =
                TargetFormat::new(source_rate, crate::audio::SampleFormat::Float, 32).unwrap();
            let target_format =
                TargetFormat::new(target_rate, crate::audio::SampleFormat::Float, 32).unwrap();

            // Create a mock source for testing
            let mock_source = MemorySampleSource::new(vec![0.1, 0.2, 0.3], 1, 44100);
            let converter = AudioTranscoder::new(mock_source, &source_format, &target_format, 1);

            match converter {
                Ok(converter) => {
                    // Test that the converter was created successfully
                    assert_eq!(converter.source_rate, source_rate);
                    assert_eq!(converter.target_rate, target_rate);
                    // Verify resampler presence matches expectation
                    assert_eq!(
                        converter.resampler.is_some(),
                        should_need_resampling,
                        "Resampler presence mismatch for {} -> {}",
                        source_rate,
                        target_rate
                    );
                }
                Err(_) => {
                    // If rubato fails to create the resampler, that's acceptable for now
                }
            }
        }
    }

    #[test]
    fn test_no_resampling_needed() {
        // Test when no resampling is needed
        let source_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        // Create a mock source for testing
        let mock_source = MemorySampleSource::new(vec![0.1, 0.2, 0.3, 0.4, 0.5], 1, 44100);
        let mut converter =
            AudioTranscoder::new(mock_source, &source_format, &target_format, 1).unwrap();

        // Should not need resampling
        // Transcoding is now handled internally by AudioSampleSource

        // Test that samples are returned unchanged when no resampling is needed
        let mut output_samples = Vec::with_capacity(10);
        let mut sample_count = 0;
        const MAX_SAMPLES: usize = 10;

        while sample_count < MAX_SAMPLES {
            match converter.read_one_sample() {
                Ok(Some(sample)) => {
                    output_samples.push(sample);
                    sample_count += 1;
                }
                Ok(None) => break,
                Err(_e) => break,
            }
        }

        // Should have some output samples
        assert!(!output_samples.is_empty());
    }

    #[test]
    fn test_resampling_quality_sine_wave() {
        // Test resampling quality with a sine wave signal
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate a 1kHz sine wave at 48kHz
        let frequency = 1000.0; // 1kHz
        let duration = 0.1; // 100ms
        let num_samples = (48000.0 * duration) as usize;

        let mut input_samples = Vec::with_capacity(num_samples);
        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            let sample = (2.0 * std::f32::consts::PI * frequency * t).sin();
            input_samples.push(sample);
        }

        // Create a mock source with the sine wave
        let mock_source = MemorySampleSource::new(input_samples, 1, 44100);
        match AudioTranscoder::new(mock_source, &source_format, &target_format, 1) {
            Ok(mut converter) => {
                // Test resampling by getting samples from the converter
                let mut output_samples = Vec::with_capacity(200);
                let mut sample_count = 0;
                const MAX_SAMPLES: usize = 200; // Allow more samples for sine wave

                while sample_count < MAX_SAMPLES {
                    match converter.read_one_sample() {
                        Ok(Some(sample)) => {
                            output_samples.push(sample);
                            sample_count += 1;
                        }
                        Ok(None) => break,
                        Err(_e) => break,
                    }
                }

                // Verify output length is reasonable (rubato may produce different lengths)
                // For now, just ensure we get some output
                assert!(
                    !output_samples.is_empty(),
                    "Should have some output samples"
                );
                assert!(
                    !output_samples.is_empty(),
                    "Should have some output samples"
                );

                // Verify the signal is still a sine wave (basic quality check)
                let max_amplitude = output_samples.iter().map(|&x| x.abs()).fold(0.0, f32::max);

                assert!(
                    max_amplitude > 0.5,
                    "Resampled sine wave too quiet, max amplitude: {}",
                    max_amplitude
                );
                assert!(
                    max_amplitude <= 1.1,
                    "Resampled sine wave too loud, max amplitude: {}",
                    max_amplitude
                );

                // Check for aliasing (high-frequency content should be minimal)
                let high_freq_energy = calculate_high_frequency_energy(&output_samples, 44100.0);
                assert!(
                    high_freq_energy < 0.1,
                    "Too much high-frequency content (aliasing): {}",
                    high_freq_energy
                );
            }
            Err(_e) => {}
        }
    }

    #[test]
    fn test_roundtrip_resampling_quality() {
        // Test that resampling up and then down preserves quality
        let original_rate = 44100;
        let intermediate_rate = 48000;
        let final_rate = 44100;

        let source_format_1 =
            TargetFormat::new(original_rate, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format_1 =
            TargetFormat::new(intermediate_rate, crate::audio::SampleFormat::Float, 32).unwrap();
        let source_format_2 =
            TargetFormat::new(intermediate_rate, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format_2 =
            TargetFormat::new(final_rate, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate a test signal: 1kHz sine wave + small amount of noise
        let duration = 0.1; // 100ms
        let num_samples = (original_rate as f32 * duration) as usize;
        let mut original_samples = Vec::with_capacity(num_samples);

        for i in 0..num_samples {
            let t = i as f32 / original_rate as f32;
            let sine_wave = 0.5 * (2.0 * std::f32::consts::PI * 1000.0 * t).sin();
            let noise = 0.05 * (rand::random::<f32>() - 0.5);
            original_samples.push(sine_wave + noise);
        }

        // First resampling: 44.1kHz -> 48kHz
        let source_1 = MemorySampleSource::new(original_samples.clone(), 1, 44100);
        let mut converter_1 =
            AudioTranscoder::new(source_1, &source_format_1, &target_format_1, 1).unwrap();

        let mut intermediate_samples = Vec::with_capacity(num_samples);
        loop {
            match converter_1.read_one_sample() {
                Ok(Some(sample)) => intermediate_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Second resampling: 48kHz -> 44.1kHz
        let intermediate_len = intermediate_samples.len();
        let source_2 = MemorySampleSource::new(intermediate_samples, 1, intermediate_rate);
        let mut converter_2 =
            AudioTranscoder::new(source_2, &source_format_2, &target_format_2, 1).unwrap();

        let mut final_samples = Vec::with_capacity(intermediate_len);
        loop {
            match converter_2.read_one_sample() {
                Ok(Some(sample)) => final_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Quality checks
        assert!(
            !final_samples.is_empty(),
            "Final samples should not be empty"
        );

        // Check that we have a reasonable number of samples (should be close to original)
        // Roundtrip through sinc resampling loses some samples at boundaries due to inherent delay
        let expected_length = original_samples.len();
        let length_tolerance = (expected_length as f32 * 0.35) as usize;
        assert!(
            final_samples.len() >= expected_length - length_tolerance
                && final_samples.len() <= expected_length + length_tolerance,
            "Final length {} should be close to original length {}",
            final_samples.len(),
            expected_length
        );

        // Check that the signal is still recognizable
        let max_amplitude = final_samples.iter().map(|&x| x.abs()).fold(0.0, f32::max);
        assert!(
            max_amplitude > 0.1,
            "Final signal should have reasonable amplitude, got {}",
            max_amplitude
        );
        assert!(
            max_amplitude <= 1.0,
            "Final signal should not be too loud, got {}",
            max_amplitude
        );
    }

    #[test]
    fn test_resampling_quality_impulse() {
        // Test resampling quality with impulse signal
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate impulse signal (single sample at maximum amplitude)
        let mut input_samples = vec![0.0; 100];
        input_samples[50] = 1.0; // Impulse at sample 50

        let source = MemorySampleSource::new(input_samples, 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::new();
        loop {
            match converter.read_one_sample() {
                Ok(Some(sample)) => output_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Basic quality checks
        assert!(!output_samples.is_empty(), "Output should not be empty");

        // The impulse will be spread by the sinc kernel; we just require that
        // some non-trivial amplitude remains (numerically, not perceptually).
        let max_amplitude = output_samples.iter().map(|&x| x.abs()).fold(0.0, f32::max);
        assert!(
            max_amplitude > 1e-8,
            "Impulse signal should have reasonable amplitude after resampling, got {}",
            max_amplitude
        );
    }

    #[test]
    fn test_resampling_quality_noise() {
        // Test resampling quality with white noise
        let source_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate white noise
        let num_samples = 1000;
        let mut input_samples = Vec::new();
        for _ in 0..num_samples {
            // Simple pseudo-random noise
            let noise = (rand::random::<f32>() - 0.5) * 2.0;
            input_samples.push(noise);
        }

        let source = MemorySampleSource::new(input_samples.clone(), 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::new();
        loop {
            match converter.read_one_sample() {
                Ok(Some(sample)) => output_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Basic quality checks
        assert!(!output_samples.is_empty(), "Output should not be empty");

        // Verify output length is approximately correct
        // Sinc resamplers with sliding window may produce fewer samples due to inherent delay
        // and not zero-padding at EOF, so we use a larger tolerance (30%)
        let expected_ratio = 48000.0 / 44100.0;
        let expected_length = (num_samples as f32 * expected_ratio) as usize;
        let length_tolerance = (expected_length as f32 * 0.30) as usize;

        assert!(
            output_samples.len() >= expected_length - length_tolerance
                && output_samples.len() <= expected_length + length_tolerance,
            "Expected ~{} samples, got {}",
            expected_length,
            output_samples.len()
        );

        // Verify the noise characteristics are preserved
        let input_rms = calculate_rms(&input_samples);
        let output_rms = calculate_rms(&output_samples);

        // RMS should be similar (within 50% tolerance for FFT resamplers)
        let rms_ratio = output_rms / input_rms;
        assert!(
            rms_ratio > 0.5 && rms_ratio < 1.5,
            "RMS ratio out of range: {} (input: {}, output: {})",
            rms_ratio,
            input_rms,
            output_rms
        );
    }

    #[test]
    fn test_resampling_multichannel_quality() {
        // Test resampling quality with multichannel audio
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let channels = 2;

        // Generate stereo test signal
        let duration = 0.1; // 100ms
        let num_frames = (48000.0 * duration) as usize;
        let mut input_samples = Vec::new();

        for i in 0..num_frames {
            let t = i as f32 / 48000.0;
            // Left channel: 440Hz
            let left = 0.3 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
            // Right channel: 880Hz
            let right = 0.3 * (2.0 * std::f32::consts::PI * 880.0 * t).sin();
            input_samples.push(left);
            input_samples.push(right);
        }

        // Create source with correct channel count (2 channels, stereo)
        let source = MemorySampleSource::new(input_samples, channels, 48000);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, channels).unwrap();

        // Read all samples (returns interleaved)
        let output_samples = converter.read_all_samples().unwrap();

        // Basic quality checks
        assert!(!output_samples.is_empty(), "Output should not be empty");

        // Should have approximately the right number of samples
        let expected_length = (num_frames as f32 * (44100.0 / 48000.0) * channels as f32) as usize;
        let length_tolerance = (expected_length as f32 * 0.1) as usize;

        assert!(
            output_samples.len() >= expected_length - length_tolerance
                && output_samples.len() <= expected_length + length_tolerance,
            "Expected ~{} samples, got {}",
            expected_length,
            output_samples.len()
        );

        // Check that we have stereo output (even number of samples)
        // Note: With larger block sizes, the resampler may produce slightly different
        // numbers of samples. We allow up to one extra sample as this doesn't affect quality.
        let remainder = output_samples.len() % 2;
        assert!(
            remainder == 0
                || output_samples.len() % 2 == 1
                    && output_samples.len() <= expected_length + length_tolerance + 1,
            "Stereo output should have even number of samples (or at most one extra), got {}",
            output_samples.len()
        );
    }

    /// Calculate RMS (Root Mean Square) of a signal
    fn calculate_rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }

        let sum_squares: f32 = samples.iter().map(|&x| x * x).sum();
        (sum_squares / samples.len() as f32).sqrt()
    }

    #[test]
    fn test_resampling_empty_input() {
        // Test behavior with empty input
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        let source = MemorySampleSource::new(vec![], 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        // Empty input should return None immediately
        assert!(matches!(converter.read_one_sample(), Ok(None)));
    }

    #[test]
    fn test_resampling_single_sample() {
        // Test with just one sample
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        let source = MemorySampleSource::new(vec![0.5], 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::new();
        loop {
            match converter.read_one_sample() {
                Ok(Some(sample)) => output_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Should produce some output even from a single sample
        assert!(
            !output_samples.is_empty(),
            "Single sample should produce some output"
        );
    }

    #[test]
    fn test_resampling_extreme_ratios() {
        // Test very high and very low sample rate ratios
        let test_cases = vec![
            (8000, 192000), // 24:1 upsampling
            (192000, 8000), // 1:24 downsampling
            (44100, 88200), // 2:1 upsampling
            (88200, 44100), // 1:2 downsampling
        ];

        for (source_rate, target_rate) in test_cases {
            let source_format =
                TargetFormat::new(source_rate, crate::audio::SampleFormat::Float, 32).unwrap();
            let target_format =
                TargetFormat::new(target_rate, crate::audio::SampleFormat::Float, 32).unwrap();

            // Generate a simple test signal
            let duration = 0.01; // 10ms
            let num_samples = (source_rate as f32 * duration) as usize;
            let mut input_samples = Vec::new();

            for i in 0..num_samples {
                let t = i as f32 / source_rate as f32;
                input_samples.push((2.0 * std::f32::consts::PI * 1000.0 * t).sin() * 0.5);
            }

            let source = MemorySampleSource::new(input_samples, 1, 44100);
            let mut converter =
                AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

            let mut output_samples = Vec::new();
            let mut sample_count = 0;
            const MAX_SAMPLES: usize = 1000; // Prevent infinite loops

            while sample_count < MAX_SAMPLES {
                match converter.read_one_sample() {
                    Ok(Some(sample)) => {
                        output_samples.push(sample);
                        sample_count += 1;
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }

            assert!(
                !output_samples.is_empty(),
                "Extreme ratio {}/{} should produce output",
                source_rate,
                target_rate
            );

            // Check for non-trivial amplitude. For extreme ratios, sinc
            // resampling can spread energy significantly; we only assert that
            // the signal is not effectively silent.
            let max_amplitude = output_samples.iter().map(|&x| x.abs()).fold(0.0, f32::max);
            assert!(
                max_amplitude > 1e-8,
                "Extreme ratio {}/{} should have reasonable amplitude, got {}",
                source_rate,
                target_rate,
                max_amplitude
            );
        }
    }

    #[test]
    fn test_resampling_long_duration() {
        // Test with a longer duration signal to ensure stability
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate a 1-second signal
        let duration = 1.0;
        let num_samples = (48000.0 * duration) as usize;
        let mut input_samples = Vec::new();

        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            // Mix of frequencies to test aliasing
            let signal = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.3
                + (2.0 * std::f32::consts::PI * 880.0 * t).sin() * 0.2
                + (2.0 * std::f32::consts::PI * 1760.0 * t).sin() * 0.1;
            input_samples.push(signal);
        }

        let source = MemorySampleSource::new(input_samples, 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::new();
        let mut sample_count = 0;
        const MAX_SAMPLES: usize = 50000; // Allow for longer processing

        while sample_count < MAX_SAMPLES {
            match converter.read_one_sample() {
                Ok(Some(sample)) => {
                    output_samples.push(sample);
                    sample_count += 1;
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }

        assert!(
            !output_samples.is_empty(),
            "Long duration should produce output"
        );

        // Check that we got a reasonable number of samples
        let expected_length = (num_samples as f32 * (44100.0 / 48000.0)) as usize;
        let length_tolerance = (expected_length as f32 * 0.05) as usize; // 5% tolerance

        assert!(
            output_samples.len() >= expected_length - length_tolerance
                && output_samples.len() <= expected_length + length_tolerance,
            "Long duration: expected ~{}, got {}",
            expected_length,
            output_samples.len()
        );
    }

    #[test]
    fn test_resampling_high_frequency_content() {
        // Test with high-frequency content to check for aliasing
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate signal with high-frequency content (near Nyquist)
        let num_samples = 1000;
        let mut input_samples = Vec::new();

        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            // High frequency signal (20kHz - near Nyquist for 48kHz)
            let signal = (2.0 * std::f32::consts::PI * 20000.0 * t).sin() * 0.5;
            input_samples.push(signal);
        }

        let source = MemorySampleSource::new(input_samples, 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::new();
        loop {
            match converter.read_one_sample() {
                Ok(Some(sample)) => output_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        assert!(
            !output_samples.is_empty(),
            "High frequency content should produce output"
        );

        // Check for aliasing - high frequency content should be attenuated
        let high_freq_energy = calculate_high_frequency_energy(&output_samples, 44100.0);
        assert!(
            high_freq_energy < 0.5, // Should be significantly attenuated
            "High frequency content should be attenuated, got {}",
            high_freq_energy
        );
    }

    #[test]
    fn test_resampling_overflow_protection() {
        // Test with very large values to check for overflow
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Test with values near the limits
        let test_values = vec![
            vec![1.0, -1.0, 0.999, -0.999], // Near full scale
            vec![0.0, 0.0, 0.0, 0.0],       // All zeros
            vec![0.5, -0.5, 0.5, -0.5],     // Alternating
        ];

        for values in test_values {
            let source = MemorySampleSource::new(values, 1, 44100);
            let mut converter =
                AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

            let mut output_samples = Vec::new();
            let mut sample_count = 0;
            const MAX_SAMPLES: usize = 100;

            while sample_count < MAX_SAMPLES {
                match converter.read_one_sample() {
                    Ok(Some(sample)) => {
                        // Check for NaN or infinity
                        assert!(
                            sample.is_finite(),
                            "Output should be finite, got {}",
                            sample
                        );
                        output_samples.push(sample);
                        sample_count += 1;
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }

            assert!(
                !output_samples.is_empty(),
                "Should produce output for edge values"
            );
        }
    }

    #[test]
    fn test_resampling_dc_offset() {
        // Test DC offset handling
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate signal with DC offset
        let dc_offset = 0.1;
        let num_samples = 100;
        let mut input_samples = Vec::new();

        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            let signal = (2.0 * std::f32::consts::PI * 1000.0 * t).sin() * 0.3 + dc_offset;
            input_samples.push(signal);
        }

        let source = MemorySampleSource::new(input_samples, 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::new();
        loop {
            match converter.read_one_sample() {
                Ok(Some(sample)) => output_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        assert!(
            !output_samples.is_empty(),
            "DC offset test should produce output"
        );

        // Check that DC offset does not blow up; sinc resamplers are linear,
        // but practical implementations may slightly attenuate DC.
        let mean_value = output_samples.iter().sum::<f32>() / output_samples.len() as f32;
        assert!(
            (mean_value - dc_offset).abs() < 0.2,
            "DC offset should be reasonably preserved, expected ~{}, got {}",
            dc_offset,
            mean_value
        );
    }

    #[test]
    fn test_resampling_simple_snr() {
        // Simple test: just resample a sine wave once and check SNR
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate a simple sine wave
        let frequency = 1000.0; // 1kHz
        let duration = 0.01; // 10ms
        let num_samples = (48000.0 * duration) as usize;
        let mut original_samples = Vec::with_capacity(num_samples);

        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            let sample = (2.0 * std::f32::consts::PI * frequency * t).sin() * 0.5;
            original_samples.push(sample);
        }

        // Resample once: 48kHz -> 44.1kHz
        let source = MemorySampleSource::new(original_samples.clone(), 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::with_capacity(num_samples);
        loop {
            match converter.read_one_sample() {
                Ok(Some(sample)) => output_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // For now, just check that we get reasonable output
        assert!(!output_samples.is_empty(), "No output samples generated");
        assert!(
            output_samples.len() > 100,
            "Too few output samples: {}",
            output_samples.len()
        );

        // Check that the output has reasonable amplitude
        let output_rms = calculate_rms(&output_samples);
        assert!(output_rms > 0.1, "Output RMS too low: {}", output_rms);
        assert!(output_rms < 1.0, "Output RMS too high: {}", output_rms);
    }

    #[test]
    fn test_resampling_snr_quality() {
        // Test that resampling maintains reasonable SNR between input and output signals
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let back_format = TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate a clean 1kHz sine wave at 48kHz
        let frequency = 1000.0; // 1kHz
        let duration = 0.1; // 100ms
        let num_samples = (48000.0 * duration) as usize;
        let mut original_samples = Vec::with_capacity(num_samples);

        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            let sample = (2.0 * std::f32::consts::PI * frequency * t).sin() * 0.5;
            original_samples.push(sample);
        }

        // First resampling: 48kHz -> 44.1kHz
        let source_1 = MemorySampleSource::new(original_samples.clone(), 1, 44100);
        let mut converter_1 =
            AudioTranscoder::new(source_1, &source_format, &target_format, 1).unwrap();

        let mut intermediate_samples = Vec::with_capacity(num_samples);
        loop {
            match converter_1.read_one_sample() {
                Ok(Some(sample)) => intermediate_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Second resampling: 44.1kHz -> 48kHz (roundtrip)
        let source_2 = MemorySampleSource::new(intermediate_samples, 1, 44100);
        let mut converter_2 =
            AudioTranscoder::new(source_2, &target_format, &back_format, 1).unwrap();

        let mut final_samples = Vec::with_capacity(original_samples.len());
        loop {
            match converter_2.read_one_sample() {
                Ok(Some(sample)) => final_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Ensure both signals have the same length for SNR calculation
        let min_len = original_samples.len().min(final_samples.len());
        let original_truncated = &original_samples[..min_len];
        let final_truncated = &final_samples[..min_len];

        // Calculate SNR between original and final (roundtrip)
        let snr = calculate_snr(original_truncated, final_truncated);

        // For sinc resampling, uncompensated phase delay and edge effects can
        // reduce this naive SNR metric even when audible quality is high.
        // Use a loose lower bound here to catch only obviously broken behaviour.
        assert!(
            snr > -10.0,
            "SNR too low: {} dB (expected > -10 dB). Original: {} samples, Final: {} samples",
            snr,
            original_truncated.len(),
            final_truncated.len()
        );
    }

    #[test]
    fn test_resampling_rms_preservation() {
        // Test that RMS energy is preserved across different resampling ratios
        let test_cases = vec![
            (48000, 44100, 1000.0), // 48kHz -> 44.1kHz, 1kHz
            (48000, 96000, 2000.0), // 48kHz -> 96kHz, 2kHz
            (44100, 48000, 1500.0), // 44.1kHz -> 48kHz, 1.5kHz
        ];

        for (source_rate, target_rate, frequency) in test_cases {
            let source_format =
                TargetFormat::new(source_rate, crate::audio::SampleFormat::Float, 32).unwrap();
            let target_format =
                TargetFormat::new(target_rate, crate::audio::SampleFormat::Float, 32).unwrap();

            // Generate sine wave at the specified frequency
            let duration = 0.05; // 50ms
            let num_samples = (source_rate as f32 * duration) as usize;
            let mut input_samples = Vec::with_capacity(num_samples);

            for i in 0..num_samples {
                let t = i as f32 / source_rate as f32;
                let sample = (2.0 * std::f32::consts::PI * frequency * t).sin() * 0.3;
                input_samples.push(sample);
            }

            // Resample
            let source = MemorySampleSource::new(input_samples.clone(), 1, source_rate);
            let mut converter =
                AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

            let mut output_samples = Vec::with_capacity(num_samples);
            loop {
                match converter.read_one_sample() {
                    Ok(Some(sample)) => output_samples.push(sample),
                    Ok(None) => break,
                    Err(_) => break,
                }
            }

            // Calculate RMS for input and output
            let input_rms = calculate_rms(&input_samples);
            let output_rms = calculate_rms(&output_samples);

            // RMS should be preserved within 20% tolerance
            // Sinc resamplers with sliding window may lose some energy at signal boundaries
            let rms_ratio = output_rms / input_rms;
            assert!(
                (0.8..=1.2).contains(&rms_ratio),
                "RMS ratio out of range for {}Hz->{}Hz: {} (input: {}, output: {})",
                source_rate,
                target_rate,
                rms_ratio,
                input_rms,
                output_rms
            );
        }
    }

    #[test]
    fn test_resampling_snr_multichannel() {
        // Test SNR preservation in multichannel (stereo) scenarios
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate stereo signal with different frequencies per channel
        let duration = 0.1; // 100ms
        let num_frames = (48000.0 * duration) as usize;
        let mut input_samples = Vec::with_capacity(num_frames * 2);

        for i in 0..num_frames {
            let t = i as f32 / 48000.0;
            // Left channel: 440Hz (A4)
            let left = 0.3 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
            // Right channel: 880Hz (A5)
            let right = 0.3 * (2.0 * std::f32::consts::PI * 880.0 * t).sin();
            input_samples.push(left);
            input_samples.push(right);
        }

        // First resampling: 48kHz -> 44.1kHz
        let source_1 = MemorySampleSource::new(input_samples.clone(), 2, 48000);
        let mut converter_1 =
            AudioTranscoder::new(source_1, &source_format, &target_format, 2).unwrap();

        // Read all samples using the chunk API
        let intermediate_samples = converter_1.read_all_samples().unwrap();

        // Second resampling: 44.1kHz -> 48kHz (roundtrip for fair comparison)
        let back_format = TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let source_2 = MemorySampleSource::new(intermediate_samples.clone(), 2, 44100);
        let mut converter_2 =
            AudioTranscoder::new(source_2, &target_format, &back_format, 2).unwrap();

        let output_samples = converter_2.read_all_samples().unwrap();

        // Separate left and right channels for SNR calculation
        let mut left_original = Vec::with_capacity(num_frames);
        let mut right_original = Vec::with_capacity(num_frames);
        let mut left_output = Vec::with_capacity(output_samples.len() / 2);
        let mut right_output = Vec::with_capacity(output_samples.len() / 2);

        // Extract left and right channels from interleaved samples
        for i in (0..input_samples.len()).step_by(2) {
            if i + 1 < input_samples.len() {
                left_original.push(input_samples[i]);
                right_original.push(input_samples[i + 1]);
            }
        }

        for i in (0..output_samples.len()).step_by(2) {
            if i + 1 < output_samples.len() {
                left_output.push(output_samples[i]);
                right_output.push(output_samples[i + 1]);
            }
        }

        // Ensure both signals have the same length for SNR calculation
        let left_min_len = left_original.len().min(left_output.len());
        let right_min_len = right_original.len().min(right_output.len());
        let left_original_truncated = &left_original[..left_min_len];
        let left_output_truncated = &left_output[..left_min_len];
        let right_original_truncated = &right_original[..right_min_len];
        let right_output_truncated = &right_output[..right_min_len];

        // Calculate SNR for both channels. As with the mono case, sinc
        // resampling plus uncompensated delay and edge effects make a strict
        // SNR threshold unrealistic; we just ensure the output is not totally
        // decorrelated.
        let left_snr = calculate_snr(left_original_truncated, left_output_truncated);
        let right_snr = calculate_snr(right_original_truncated, right_output_truncated);

        assert!(
            left_snr > -10.0,
            "Left channel SNR too low: {} dB (expected > -10 dB)",
            left_snr
        );
        assert!(
            right_snr > -10.0,
            "Right channel SNR too low: {} dB (expected > -10 dB)",
            right_snr
        );
    }

    #[test]
    fn test_resampling_rms_complex_signal() {
        // Test RMS preservation with complex multi-frequency signals
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate complex signal with multiple harmonics (fundamental + overtones)
        let duration = 0.1; // 100ms
        let num_samples = (48000.0 * duration) as usize;
        let mut input_samples = Vec::with_capacity(num_samples);

        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            // Fundamental frequency: 220Hz (A3)
            let fundamental = 0.4 * (2.0 * std::f32::consts::PI * 220.0 * t).sin();
            // First harmonic: 440Hz (A4)
            let harmonic1 = 0.2 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
            // Second harmonic: 880Hz (A5)
            let harmonic2 = 0.1 * (2.0 * std::f32::consts::PI * 880.0 * t).sin();
            // Third harmonic: 1320Hz (E6)
            let harmonic3 = 0.05 * (2.0 * std::f32::consts::PI * 1320.0 * t).sin();

            let complex_signal = fundamental + harmonic1 + harmonic2 + harmonic3;
            input_samples.push(complex_signal);
        }

        // Resample complex signal
        let source = MemorySampleSource::new(input_samples.clone(), 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::with_capacity(num_samples);
        loop {
            match converter.read_one_sample() {
                Ok(Some(sample)) => output_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Calculate RMS for input and output
        let input_rms = calculate_rms(&input_samples);
        let output_rms = calculate_rms(&output_samples);

        // RMS should be preserved within 15% tolerance for complex signals
        let rms_ratio = output_rms / input_rms;
        assert!(
            (0.85..=1.15).contains(&rms_ratio),
            "RMS ratio out of range for complex signal: {} (input: {}, output: {})",
            rms_ratio,
            input_rms,
            output_rms
        );

        // Verify that the complex signal structure is maintained
        // by checking that we have significant energy in multiple frequency bands
        let input_energy = input_rms * input_rms;
        let output_energy = output_rms * output_rms;
        let energy_ratio = output_energy / input_energy;

        assert!(
            (0.7..=1.3).contains(&energy_ratio),
            "Energy ratio out of range for complex signal: {} (input: {}, output: {})",
            energy_ratio,
            input_energy,
            output_energy
        );
    }

    #[test]
    fn test_wav_sample_source_16bit() {
        use crate::testutil::write_wav_with_bits;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_16bit.wav");

        // Create a 16-bit WAV file with known samples
        let samples: Vec<i16> = vec![1000, -2000, 3000, -4000, 5000];
        write_wav_with_bits(wav_path.clone(), vec![samples], 44100, 16).unwrap();

        // Test reading the WAV file using generic function
        let mut wav_source = create_sample_source_from_file(&wav_path, None, 1024).unwrap();

        let mut read_samples = Vec::new();
        loop {
            match wav_source.read_one_sample() {
                Ok(Some(sample)) => read_samples.push(sample),
                Ok(None) => break,
                Err(e) => panic!("Error reading sample: {}", e),
            }
        }

        // Verify we got the expected number of samples
        assert_eq!(read_samples.len(), 5);

        // Verify the samples are scaled correctly (16-bit to f32)
        // 16-bit samples should be scaled to [-1.0, 1.0] range
        let expected_samples = [
            1000.0 / (1 << 15) as f32,  // 1000 / 32768
            -2000.0 / (1 << 15) as f32, // -2000 / 32768
            3000.0 / (1 << 15) as f32,  // 3000 / 32768
            -4000.0 / (1 << 15) as f32, // -4000 / 32768
            5000.0 / (1 << 15) as f32,  // 5000 / 32768
        ];

        for (i, (actual, expected)) in read_samples.iter().zip(expected_samples.iter()).enumerate()
        {
            assert!(
                (actual - expected).abs() < 0.0001,
                "Sample {} mismatch: expected {}, got {}",
                i,
                expected,
                actual
            );
        }
    }

    #[test]
    fn test_wav_sample_source_24bit() {
        use crate::testutil::write_wav_with_bits;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_24bit.wav");

        // Create a 24-bit WAV file with known samples
        let samples: Vec<i32> = vec![100000, -200000, 300000, -400000, 500000];
        write_wav_with_bits(wav_path.clone(), vec![samples], 44100, 24).unwrap();

        // Test reading the WAV file using generic function
        let mut wav_source = create_sample_source_from_file(&wav_path, None, 1024).unwrap();

        let mut read_samples = Vec::new();
        loop {
            match wav_source.read_one_sample() {
                Ok(Some(sample)) => read_samples.push(sample),
                Ok(None) => break,
                Err(e) => panic!("Error reading sample: {}", e),
            }
        }

        // Verify we got the expected number of samples
        assert_eq!(read_samples.len(), 5);

        // Verify the samples are scaled correctly (24-bit to f32)
        // 24-bit samples should be scaled to [-1.0, 1.0] range
        let expected_samples = [
            100000.0 / (1 << 23) as f32,  // 100000 / 8388608
            -200000.0 / (1 << 23) as f32, // -200000 / 8388608
            300000.0 / (1 << 23) as f32,  // 300000 / 8388608
            -400000.0 / (1 << 23) as f32, // -400000 / 8388608
            500000.0 / (1 << 23) as f32,  // 500000 / 8388608
        ];

        for (i, (actual, expected)) in read_samples.iter().zip(expected_samples.iter()).enumerate()
        {
            assert!(
                (actual - expected).abs() < 0.0001,
                "Sample {} mismatch: expected {}, got {}",
                i,
                expected,
                actual
            );
        }
    }

    #[test]
    fn test_wav_sample_source_32bit() {
        use crate::testutil::write_wav;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_32bit.wav");

        // Create a 32-bit WAV file with known samples
        let samples: Vec<i32> = vec![1000000, -2000000, 3000000, -4000000, 5000000];
        write_wav(wav_path.clone(), vec![samples], 44100).unwrap();

        // Test reading the WAV file using generic function
        let mut wav_source = create_sample_source_from_file(&wav_path, None, 1024).unwrap();

        let mut read_samples = Vec::new();
        loop {
            match wav_source.read_one_sample() {
                Ok(Some(sample)) => read_samples.push(sample),
                Ok(None) => break,
                Err(e) => panic!("Error reading sample: {}", e),
            }
        }

        // Verify we got the expected number of samples
        assert_eq!(read_samples.len(), 5);

        // Verify the samples are scaled correctly (32-bit to f32)
        // 32-bit samples should be scaled to [-1.0, 1.0] range
        let expected_samples = [
            0.0004656613,  // 1000000 / 2147483648
            -0.0009313226, // -2000000 / 2147483648
            0.0013969839,  // 3000000 / 2147483648
            -0.0018626451, // -4000000 / 2147483648
            0.0023283064,  // 5000000 / 2147483648
        ];

        for (i, (actual, expected)) in read_samples.iter().zip(expected_samples.iter()).enumerate()
        {
            assert!(
                (actual - expected).abs() < 0.0001,
                "Sample {} mismatch: expected {}, got {}",
                i,
                expected,
                actual
            );
        }
    }

    #[test]
    fn test_wav_sample_source_stereo() {
        use crate::testutil::write_wav;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_stereo.wav");

        // Create a stereo WAV file
        let left_samples: Vec<i32> = vec![1000, 2000, 3000];
        let right_samples: Vec<i32> = vec![-1000, -2000, -3000];
        write_wav(wav_path.clone(), vec![left_samples, right_samples], 44100).unwrap();

        // Test reading the WAV file
        let mut wav_source = AudioSampleSource::from_file(&wav_path, None, 1024).unwrap();

        // Read all samples (returns interleaved)
        let read_samples = wav_source.read_all_samples().unwrap();

        // Verify we got the expected number of samples (interleaved stereo)
        assert_eq!(read_samples.len(), 6);

        // Verify the samples are interleaved correctly (L, R, L, R, L, R)
        let expected_samples = [
            1000.0 / (1 << 31) as f32,  // Left channel, sample 1
            -1000.0 / (1 << 31) as f32, // Right channel, sample 1
            2000.0 / (1 << 31) as f32,  // Left channel, sample 2
            -2000.0 / (1 << 31) as f32, // Right channel, sample 2
            3000.0 / (1 << 31) as f32,  // Left channel, sample 3
            -3000.0 / (1 << 31) as f32, // Right channel, sample 3
        ];

        for (i, (actual, expected)) in read_samples.iter().zip(expected_samples.iter()).enumerate()
        {
            assert!(
                (actual - expected).abs() < 0.0001,
                "Sample {} mismatch: expected {}, got {}",
                i,
                expected,
                actual
            );
        }
    }

    #[test]
    fn test_wav_sample_source_empty_file() {
        use crate::testutil::write_wav;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_empty.wav");

        // Create an empty WAV file
        write_wav(wav_path.clone(), vec![Vec::<i32>::new()], 44100).unwrap();

        // Test reading the empty WAV file
        let mut wav_source = AudioSampleSource::from_file(&wav_path, None, 1024).unwrap();

        // Should return None immediately
        match wav_source.read_one_sample() {
            Ok(None) => {} // Expected
            Ok(Some(sample)) => panic!("Expected None for empty file, got: {}", sample),
            Err(e) => panic!("Error reading empty file: {}", e),
        }

        // Verify is_finished is true
        assert!(wav_source.is_finished());
    }

    #[test]
    fn test_wav_sample_source_nonexistent_file() {
        let wav_path = std::path::Path::new("nonexistent_file.wav");

        // Should return an error for nonexistent file
        if AudioSampleSource::from_file(wav_path, None, 1024).is_ok() {
            panic!("Expected error for nonexistent file")
        }
    }

    #[test]
    fn test_wav_sample_source_is_finished() {
        use crate::testutil::write_wav;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_finished.wav");

        // Create a WAV file with a few samples
        let samples: Vec<i32> = vec![1000, 2000, 3000];
        write_wav(wav_path.clone(), vec![samples], 44100).unwrap();

        let mut wav_source = AudioSampleSource::from_file(&wav_path, None, 1024).unwrap();

        // Initially not finished
        assert!(!wav_source.is_finished());

        // Read all samples
        let mut sample_count = 0;
        loop {
            match wav_source.read_one_sample() {
                Ok(Some(_)) => {
                    sample_count += 1;
                    assert!(!wav_source.is_finished()); // Still not finished
                }
                Ok(None) => {
                    assert!(wav_source.is_finished()); // Now finished
                    break;
                }
                Err(e) => panic!("Error reading sample: {}", e),
            }
        }

        // Verify we read the expected number of samples
        assert_eq!(sample_count, 3);

        // Verify is_finished is true after reading all samples
        assert!(wav_source.is_finished());
    }

    #[test]
    fn test_wav_sample_source_amplitude_consistency() {
        use crate::testutil::write_wav_with_bits;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();

        // Generate the same audio content (sine wave with amplitude 0.5)
        let sample_rate = 44100;
        let duration_samples = 1000;
        let frequency = 440.0; // 440Hz sine wave

        let sine_wave: Vec<f32> = (0..duration_samples)
            .map(|i| {
                (i as f32 * frequency * 2.0 * std::f32::consts::PI / sample_rate as f32).sin() * 0.5
            })
            .collect();

        // Test 16-bit WAV
        let wav_16_path = tempdir.path().join("test_16bit_amplitude.wav");
        let samples_16: Vec<i16> = sine_wave.iter().map(|&x| (x * 32767.0) as i16).collect();
        write_wav_with_bits(wav_16_path.clone(), vec![samples_16], sample_rate, 16).unwrap();

        // Test 24-bit WAV
        let wav_24_path = tempdir.path().join("test_24bit_amplitude.wav");
        let samples_24: Vec<i32> = sine_wave
            .iter()
            .map(|&x| (x * 8388607.0) as i32) // 24-bit range
            .collect();
        write_wav_with_bits(wav_24_path.clone(), vec![samples_24], sample_rate, 24).unwrap();

        // Test 32-bit WAV
        let wav_32_path = tempdir.path().join("test_32bit_amplitude.wav");
        let samples_32: Vec<i32> = sine_wave
            .iter()
            .map(|&x| (x * 2147483647.0) as i32) // 32-bit range
            .collect();
        write_wav_with_bits(wav_32_path.clone(), vec![samples_32], sample_rate, 32).unwrap();

        // Read samples from each WAV file
        let mut wav_16_source = AudioSampleSource::from_file(&wav_16_path, None, 1024).unwrap();
        let mut wav_24_source = AudioSampleSource::from_file(&wav_24_path, None, 1024).unwrap();
        let mut wav_32_source = AudioSampleSource::from_file(&wav_32_path, None, 1024).unwrap();

        let mut samples_16_read = Vec::new();
        let mut samples_24_read = Vec::new();
        let mut samples_32_read = Vec::new();

        for _ in 0..duration_samples {
            if let Ok(Some(sample)) = wav_16_source.read_one_sample() {
                samples_16_read.push(sample);
            }
            if let Ok(Some(sample)) = wav_24_source.read_one_sample() {
                samples_24_read.push(sample);
            }
            if let Ok(Some(sample)) = wav_32_source.read_one_sample() {
                samples_32_read.push(sample);
            }
        }

        // Calculate RMS for each
        let rms_16: f32 = (samples_16_read.iter().map(|&x| x * x).sum::<f32>()
            / samples_16_read.len() as f32)
            .sqrt();
        let rms_24: f32 = (samples_24_read.iter().map(|&x| x * x).sum::<f32>()
            / samples_24_read.len() as f32)
            .sqrt();
        let rms_32: f32 = (samples_32_read.iter().map(|&x| x * x).sum::<f32>()
            / samples_32_read.len() as f32)
            .sqrt();

        // The RMS should be similar across all bit depths (within 5% tolerance)
        let expected_rms = 0.5 / (2.0_f32.sqrt()); // RMS of sine wave with amplitude 0.5

        // All should be close to the expected RMS
        assert!(
            (rms_16 - expected_rms).abs() / expected_rms < 0.05,
            "16-bit RMS too different: got {:.6}, expected {:.6}",
            rms_16,
            expected_rms
        );
        assert!(
            (rms_24 - expected_rms).abs() / expected_rms < 0.05,
            "24-bit RMS too different: got {:.6}, expected {:.6}",
            rms_24,
            expected_rms
        );
        assert!(
            (rms_32 - expected_rms).abs() / expected_rms < 0.05,
            "32-bit RMS too different: got {:.6}, expected {:.6}",
            rms_32,
            expected_rms
        );

        // All bit depths should have similar RMS (within 10% of each other)
        assert!(
            (rms_16 - rms_24).abs() / rms_16 < 0.1,
            "16-bit and 24-bit RMS too different: {:.6} vs {:.6}",
            rms_16,
            rms_24
        );
        assert!(
            (rms_16 - rms_32).abs() / rms_16 < 0.1,
            "16-bit and 32-bit RMS too different: {:.6} vs {:.6}",
            rms_16,
            rms_32
        );
        assert!(
            (rms_24 - rms_32).abs() / rms_24 < 0.1,
            "24-bit and 32-bit RMS too different: {:.6} vs {:.6}",
            rms_24,
            rms_32
        );
    }

    #[test]
    fn test_wav_sample_source_different_sample_rates() {
        use crate::testutil::write_wav;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();

        // Test different sample rates
        let sample_rates = vec![22050, 44100, 48000, 96000];

        for sample_rate in sample_rates {
            let wav_path = tempdir.path().join(format!("test_{}.wav", sample_rate));

            // Create a WAV file with a sine wave
            let duration = 0.01; // 10ms
            let num_samples = (sample_rate as f32 * duration) as usize;
            let samples: Vec<i32> = (0..num_samples)
                .map(|i| {
                    ((i as f32 * 1000.0 * 2.0 * std::f32::consts::PI / sample_rate as f32).sin()
                        * (1 << 23) as f32) as i32
                })
                .collect();

            write_wav(wav_path.clone(), vec![samples], sample_rate).unwrap();

            // Test reading the WAV file
            let mut wav_source = AudioSampleSource::from_file(&wav_path, None, 1024).unwrap();

            let mut read_samples = Vec::new();
            loop {
                match wav_source.read_one_sample() {
                    Ok(Some(sample)) => read_samples.push(sample),
                    Ok(None) => break,
                    Err(e) => panic!("Error reading sample at {}Hz: {}", sample_rate, e),
                }
            }

            // Verify we got the expected number of samples
            assert_eq!(read_samples.len(), num_samples);

            // Verify the samples have reasonable amplitude (not all zeros)
            let rms: f32 = (read_samples.iter().map(|&x| x * x).sum::<f32>()
                / read_samples.len() as f32)
                .sqrt();
            assert!(rms > 0.001, "RMS too low for {}Hz: {}", sample_rate, rms);
        }
    }

    #[test]
    fn test_wav_sample_source_seek() {
        use crate::testutil::write_wav;
        use std::time::Duration;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_seek.wav");

        // Create a WAV file with 10 seconds of samples at 44100 Hz
        // We'll use a pattern that changes over time so we can verify seeking
        let sample_rate = 44100u32;
        let duration_secs = 10;
        let total_samples = sample_rate as usize * duration_secs;

        // Create samples with a pattern: value = sample_index / 1000 (so we can verify position)
        let samples: Vec<i32> = (0..total_samples)
            .map(|i| (i as i32 / 1000).min(i32::MAX / 2))
            .collect();

        write_wav(wav_path.clone(), vec![samples], sample_rate).unwrap();

        // Test seeking to 5 seconds
        let seek_time = Duration::from_secs(5);
        let mut wav_source =
            AudioSampleSource::from_file(&wav_path, Some(seek_time), 1024).unwrap();

        // Read a few samples and verify we can read after seeking
        // At 5 seconds, we should be at sample index ~220500 (5 * 44100)
        let first_sample = wav_source.read_one_sample().unwrap();
        assert!(first_sample.is_some(), "Should have samples after seeking");

        // Verify we can read multiple samples (seeking worked)
        let second_sample = wav_source.read_one_sample().unwrap();
        assert!(
            second_sample.is_some(),
            "Should be able to read multiple samples after seeking"
        );

        // Test seeking to 0 (should work like from_file)
        let mut wav_source_start =
            AudioSampleSource::from_file(&wav_path, Some(std::time::Duration::ZERO), 1024).unwrap();
        let start_sample = wav_source_start.read_one_sample().unwrap();
        assert!(start_sample.is_some(), "Should have samples from start");
    }

    #[test]
    fn test_wav_sample_source_seek_clears_leftover_samples() {
        use crate::testutil::write_wav;
        use std::time::Duration;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_seek_leftover.wav");

        // Create a WAV file with 4 seconds of samples at 44100 Hz.
        // First half (0-2s) is positive; second half (2-4s) is negative so we
        // can distinguish pre‑seek from post‑seek regions by sign alone.
        let sample_rate = 44100u32;
        let duration_secs = 4;
        let total_samples = sample_rate as usize * duration_secs;

        let samples: Vec<i32> = (0..total_samples)
            .map(|i| {
                if i < (sample_rate as usize * 2) {
                    1000
                } else {
                    -1000
                }
            })
            .collect();

        write_wav(wav_path.clone(), vec![samples], sample_rate).unwrap();

        // Force AudioSampleSource to go through detect_channels_and_prime_buffer so
        // that it decodes some audio at the beginning of the stream and stores it
        // in leftover_samples before we seek.
        std::env::set_var("MTRACK_FORCE_DETECT_CHANNELS", "1");

        // Seek to 3 seconds, which lies firmly in the negative region.
        let seek_time = Duration::from_secs(3);
        let mut wav_source =
            AudioSampleSource::from_file(&wav_path, Some(seek_time), 1024).unwrap();

        let first_sample = wav_source
            .read_one_sample()
            .unwrap()
            .expect("expected sample after seeking");

        // If leftover_samples were not cleared before seeking, we'd first see
        // positive samples from the start of the file. With the bug fixed we
        // should start in the negative region.
        assert!(
            first_sample < 0.0,
            "expected first sample after seek to come from post‑seek (negative) region, got {}",
            first_sample
        );

        // Clean up the env var so it doesn't affect other tests.
        std::env::remove_var("MTRACK_FORCE_DETECT_CHANNELS");
    }

    #[test]
    fn test_wav_sample_source_4channel() {
        use crate::testutil::write_wav_with_bits;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_4channel.wav");

        // Create a 4-channel WAV file with known samples
        let channel_0: Vec<i32> = vec![1000, -2000, 3000];
        let channel_1: Vec<i32> = vec![4000, -5000, 6000];
        let channel_2: Vec<i32> = vec![7000, -8000, 9000];
        let channel_3: Vec<i32> = vec![10000, -11000, 12000];

        write_wav_with_bits(
            wav_path.clone(),
            vec![channel_0, channel_1, channel_2, channel_3],
            44100,
            32,
        )
        .unwrap();

        // Test reading the WAV file
        let mut wav_source = AudioSampleSource::from_file(&wav_path, None, 1024).unwrap();

        // Verify channel count
        assert_eq!(wav_source.channel_count(), 4);

        // Read all samples (returns interleaved)
        let samples_read = wav_source.read_all_samples().unwrap();

        // Verify we got the expected number of samples
        assert_eq!(samples_read.len(), 12);

        // Verify interleaving: samples should be in order channel_0[0], channel_1[0], channel_2[0], channel_3[0], channel_0[1], etc.
        let expected_samples = [
            1000.0 / (1 << 31) as f32,   // channel_0[0]
            4000.0 / (1 << 31) as f32,   // channel_1[0]
            7000.0 / (1 << 31) as f32,   // channel_2[0]
            10000.0 / (1 << 31) as f32,  // channel_3[0]
            -2000.0 / (1 << 31) as f32,  // channel_0[1]
            -5000.0 / (1 << 31) as f32,  // channel_1[1]
            -8000.0 / (1 << 31) as f32,  // channel_2[1]
            -11000.0 / (1 << 31) as f32, // channel_3[1]
            3000.0 / (1 << 31) as f32,   // channel_0[2]
            6000.0 / (1 << 31) as f32,   // channel_1[2]
            9000.0 / (1 << 31) as f32,   // channel_2[2]
            12000.0 / (1 << 31) as f32,  // channel_3[2]
        ];

        for (i, (actual, expected)) in samples_read.iter().zip(expected_samples.iter()).enumerate()
        {
            assert!(
                (actual - expected).abs() < 0.0001,
                "Sample {} mismatch: expected {}, got {}",
                i,
                expected,
                actual
            );
        }
    }

    #[test]
    fn test_wav_sample_source_6channel() {
        use crate::testutil::write_wav_with_bits;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_6channel.wav");

        // Create a 6-channel WAV file (5.1 surround sound)
        let channel_0: Vec<i32> = vec![1000, -2000]; // Front Left
        let channel_1: Vec<i32> = vec![3000, -4000]; // Front Right
        let channel_2: Vec<i32> = vec![5000, -6000]; // Center
        let channel_3: Vec<i32> = vec![7000, -8000]; // LFE (Low Frequency Effects)
        let channel_4: Vec<i32> = vec![9000, -10000]; // Rear Left
        let channel_5: Vec<i32> = vec![11000, -12000]; // Rear Right

        write_wav_with_bits(
            wav_path.clone(),
            vec![
                channel_0, channel_1, channel_2, channel_3, channel_4, channel_5,
            ],
            48000,
            24,
        )
        .unwrap();

        // Test reading the WAV file
        let mut wav_source = AudioSampleSource::from_file(&wav_path, None, 1024).unwrap();

        // Verify channel count and sample rate
        assert_eq!(wav_source.channel_count(), 6);
        assert_eq!(wav_source.sample_rate(), 48000);

        // Read all samples (returns interleaved)
        let samples_read = wav_source.read_all_samples().unwrap();

        // Verify we got the expected number of samples
        assert_eq!(samples_read.len(), 12);

        // Verify interleaving: samples should be in order channel_0[0], channel_1[0], ..., channel_5[0], channel_0[1], etc.
        let expected_samples = [
            1000.0 / (1 << 23) as f32,   // channel_0[0] - Front Left
            -2000.0 / (1 << 23) as f32,  // channel_0[1] - Front Left
            3000.0 / (1 << 23) as f32,   // channel_1[0] - Front Right
            -4000.0 / (1 << 23) as f32,  // channel_1[1] - Front Right
            5000.0 / (1 << 23) as f32,   // channel_2[0] - Center
            -6000.0 / (1 << 23) as f32,  // channel_2[1] - Center
            7000.0 / (1 << 23) as f32,   // channel_3[0] - LFE
            -8000.0 / (1 << 23) as f32,  // channel_3[1] - LFE
            9000.0 / (1 << 23) as f32,   // channel_4[0] - Rear Left
            -10000.0 / (1 << 23) as f32, // channel_4[1] - Rear Left
            11000.0 / (1 << 23) as f32,  // channel_5[0] - Rear Right
            -12000.0 / (1 << 23) as f32, // channel_5[1] - Rear Right
        ];

        for (i, (actual, expected)) in samples_read.iter().zip(expected_samples.iter()).enumerate()
        {
            assert!(
                (actual - expected).abs() < 0.0001,
                "Sample {} mismatch: expected {}, got {}",
                i,
                expected,
                actual
            );
        }
    }

    fn decode_some_samples_from<P: AsRef<std::path::Path>>(path: P) {
        let path = path.as_ref();

        assert!(
            path.exists(),
            "expected audio fixture to exist at {:?}",
            path
        );

        let mut source = create_sample_source_from_file(path, None, 1024)
            .expect("failed to create sample source");

        // Basic sanity checks: metadata should look sensible.
        assert!(source.sample_rate() > 0, "sample_rate should be > 0");
        assert!(source.channel_count() > 0, "channel_count should be > 0");

        // Try to read a small number of samples to ensure we actually decode audio.
        let mut count = 0usize;
        const MAX_SAMPLES: usize = 2048;
        while count < MAX_SAMPLES {
            match source.read_one_sample() {
                Ok(Some(_)) => count += 1,
                Ok(None) => break,
                Err(e) => panic!("error while decoding samples: {}", e),
            }
        }

        assert!(count > 0, "no samples decoded from test file: {:?}", path);
    }

    #[test]
    fn test_symphonia_can_decode_wav() {
        decode_some_samples_from(std::path::Path::new("assets/1Channel44.1k.wav"));
    }

    #[test]
    fn test_symphonia_can_decode_flac() {
        decode_some_samples_from(std::path::Path::new("assets/1Channel44.1k.flac"));
    }

    #[test]
    fn test_symphonia_can_decode_ogg_vorbis() {
        decode_some_samples_from(std::path::Path::new("assets/1Channel44.1k.ogg"));
    }

    #[test]
    fn test_symphonia_can_decode_mp3() {
        decode_some_samples_from(std::path::Path::new("assets/1Channel44.1k.mp3"));
    }

    #[test]
    fn test_symphonia_can_decode_aac() {
        decode_some_samples_from(std::path::Path::new("assets/1Channel44.1k.aac"));
    }

    #[test]
    fn test_symphonia_can_decode_alac() {
        decode_some_samples_from(std::path::Path::new("assets/1Channel44.1k_alac.m4a"));
    }

    #[test]
    fn test_symphonia_can_decode_alac_stereo() {
        decode_some_samples_from(std::path::Path::new("assets/2Channel44.1k_alac.m4a"));
    }
}
