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

//! Loading of short audio clips as mono buffers at a target sample rate.
//! Used by the generated virtual tracks (metronome clicks, pilot hints).

use std::error::Error;
use std::path::Path;

use super::sample_source::create_sample_source_from_file;
use super::sample_source::traits::SampleSource;

/// Loads an audio file fully into memory, downmixed to mono and resampled to
/// `sample_rate` with linear interpolation (sufficient for short clips, same
/// approach as the triggered-sample loader).
pub fn load_mono_clip(
    path: &Path,
    volume: f64,
    sample_rate: u32,
) -> Result<Vec<f32>, Box<dyn Error>> {
    let mut source = create_sample_source_from_file(path, None, 4096)
        .map_err(|e| format!("Failed to load clip {}: {}", path.display(), e))?;
    let source_rate = source.sample_rate();
    let channels = source.channel_count().max(1) as usize;

    let mut interleaved = Vec::new();
    while let Some(sample) = source.next_sample()? {
        interleaved.push(sample);
    }

    // Downmix to mono.
    let mono: Vec<f32> = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();

    // Resample with linear interpolation.
    let resampled = if source_rate != sample_rate {
        let ratio = sample_rate as f64 / source_rate as f64;
        let target_frames = (mono.len() as f64 * ratio).ceil() as usize;
        (0..target_frames)
            .map(|frame| {
                let source_pos = frame as f64 / ratio;
                let idx = source_pos.floor() as usize;
                let frac = source_pos.fract() as f32;
                let a = mono.get(idx).copied().unwrap_or(0.0);
                let b = mono.get(idx + 1).copied().unwrap_or(a);
                a + (b - a) * frac
            })
            .collect()
    } else {
        mono
    };

    Ok(resampled.into_iter().map(|s| s * volume as f32).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_downmixes() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let path = tempdir.path().join("clip.wav");
        crate::testutil::write_wav(
            path.clone(),
            vec![vec![1_000_000_000_i32; 100], vec![0_i32; 100]],
            44100,
        )?;
        let clip = load_mono_clip(&path, 1.0, 44100)?;
        assert_eq!(clip.len(), 100);
        // Mono downmix averages the two channels.
        assert!(clip[0] > 0.0 && clip[0] < 0.5);
        Ok(())
    }

    #[test]
    fn resamples_to_target_rate() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let path = tempdir.path().join("clip.wav");
        crate::testutil::write_wav(path.clone(), vec![vec![1_000_000_000_i32; 4410]], 44100)?;
        let clip = load_mono_clip(&path, 1.0, 48000)?;
        // 0.1s of audio at 48k.
        assert!((clip.len() as i64 - 4800).abs() <= 2, "got {}", clip.len());
        Ok(())
    }

    #[test]
    fn missing_file_errors() {
        assert!(load_mono_clip(Path::new("/nonexistent/clip.wav"), 1.0, 44100).is_err());
    }
}
