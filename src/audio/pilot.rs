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

//! The virtual voice-pilot track.
//!
//! [`PilotSource`] renders the audio of a song's pilot hints — short cues
//! like "bridge in 3..2..1" — at their resolved positions, as a mono
//! [`SampleSource`] that participates in the mixer like any file-backed
//! track. Hints are individual short clips placed on a timeline, so moving
//! one is a config edit and seeking works sample-accurately: a hint already
//! in progress at the seek target plays its remaining tail.

use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;

use super::sample_source::error::SampleSourceError;
use super::sample_source::traits::SampleSource;

/// A hint clip scheduled onto the pilot track: the audio file plus its
/// resolved start position in seconds from the song start (negative when the
/// clip begins before the song, e.g. a countdown into the first downbeat).
#[derive(Debug, Clone)]
pub struct PilotClip {
    pub start_secs: f64,
    pub path: PathBuf,
}

/// A loaded clip positioned in samples relative to the source start.
struct LoadedClip {
    start: i64,
    samples: Vec<f32>,
}

/// A virtual pilot track rendering scheduled hint clips.
pub struct PilotSource {
    /// Clips sorted by start position (relative to source start).
    clips: Vec<LoadedClip>,
    /// Current playback position in samples.
    position: i64,
    /// The source is exhausted at this position (song end).
    end_position: i64,
    /// Index of the next not-yet-active clip.
    next_clip: usize,
    /// Indices of currently sounding clips.
    active: Vec<usize>,
    sample_rate: u32,
}

impl PilotSource {
    /// Creates a pilot source for a song.
    ///
    /// `start_time` is the playback start offset within the song: clips are
    /// shifted accordingly, clips wholly before the offset are skipped, and
    /// a clip overlapping the offset plays its remaining tail.
    pub fn new(
        clips: &[PilotClip],
        sample_rate: u32,
        start_time: Duration,
        song_duration: Duration,
    ) -> Result<Self, Box<dyn Error>> {
        let rate = sample_rate as f64;
        let start_secs = start_time.as_secs_f64();
        let end_position =
            ((song_duration.saturating_sub(start_time)).as_secs_f64() * rate).round() as i64;

        let mut loaded = Vec::with_capacity(clips.len());
        for clip in clips {
            let start = ((clip.start_secs - start_secs) * rate).round() as i64;
            if start >= end_position {
                continue;
            }
            let samples = super::monoclip::load_mono_clip(&clip.path, 1.0, sample_rate)?;
            if start + samples.len() as i64 <= 0 {
                continue;
            }
            loaded.push(LoadedClip { start, samples });
        }
        loaded.sort_by_key(|clip| clip.start);

        Ok(PilotSource {
            clips: loaded,
            position: 0,
            end_position,
            next_clip: 0,
            active: Vec::with_capacity(2),
            sample_rate,
        })
    }
}

impl SampleSource for PilotSource {
    fn next_sample(&mut self) -> Result<Option<f32>, SampleSourceError> {
        if self.position >= self.end_position {
            return Ok(None);
        }

        // Activate clips that start at or before this position.
        while self.next_clip < self.clips.len() && self.clips[self.next_clip].start <= self.position
        {
            self.active.push(self.next_clip);
            self.next_clip += 1;
        }

        // Sum currently-sounding clips and drop finished ones.
        let position = self.position;
        let clips = &self.clips;
        let mut sample = 0.0f32;
        self.active.retain(|&idx| {
            let clip = &clips[idx];
            let offset = position - clip.start;
            if offset < 0 {
                return true;
            }
            match clip.samples.get(offset as usize) {
                Some(value) => {
                    sample += value;
                    true
                }
                None => false,
            }
        });

        self.position += 1;
        Ok(Some(sample.clamp(-1.0, 1.0)))
    }

    fn channel_count(&self) -> u16 {
        1
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn bits_per_sample(&self) -> u16 {
        32
    }

    fn sample_format(&self) -> crate::audio::SampleFormat {
        crate::audio::SampleFormat::Float
    }

    fn duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(
            self.end_position as f64 / self.sample_rate as f64,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44100;

    fn write_clip(dir: &std::path::Path, name: &str, frames: usize) -> PathBuf {
        let path = dir.join(name);
        crate::testutil::write_wav(path.clone(), vec![vec![1_000_000_000_i32; frames]], RATE)
            .unwrap();
        path
    }

    #[test]
    fn renders_clip_at_position() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        // A 0.1s clip placed at 0.5s in a 2s song.
        let path = write_clip(tempdir.path(), "hint.wav", 4410);
        let clips = vec![PilotClip {
            start_secs: 0.5,
            path,
        }];
        let mut source = PilotSource::new(&clips, RATE, Duration::ZERO, Duration::from_secs(2))?;

        let mut rendered = Vec::new();
        while let Some(sample) = source.next_sample()? {
            rendered.push(sample);
        }
        assert_eq!(rendered.len(), 2 * RATE as usize);

        // Silence before the clip, energy inside it, silence after.
        assert!(rendered[..22000].iter().all(|s| s.abs() < 1e-6));
        let inside = &rendered[22050..22050 + 4410];
        assert!(inside.iter().any(|s| s.abs() > 0.1));
        assert!(rendered[26500..].iter().all(|s| s.abs() < 1e-6));
        Ok(())
    }

    #[test]
    fn seek_mid_clip_plays_tail() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let path = write_clip(tempdir.path(), "hint.wav", 4410);
        let clips = vec![PilotClip {
            start_secs: 0.5,
            path,
        }];
        // Start playback 0.55s in — mid-clip.
        let mut source = PilotSource::new(
            &clips,
            RATE,
            Duration::from_secs_f64(0.55),
            Duration::from_secs(2),
        )?;

        let mut first = Vec::new();
        for _ in 0..1000 {
            match source.next_sample()? {
                Some(sample) => first.push(sample),
                None => break,
            }
        }
        assert!(first.iter().any(|s| s.abs() > 0.1), "tail should sound");
        Ok(())
    }

    #[test]
    fn seek_past_clip_skips_it() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let path = write_clip(tempdir.path(), "hint.wav", 4410);
        let clips = vec![PilotClip {
            start_secs: 0.5,
            path,
        }];
        let mut source =
            PilotSource::new(&clips, RATE, Duration::from_secs(1), Duration::from_secs(2))?;

        let mut rendered = Vec::new();
        while let Some(sample) = source.next_sample()? {
            rendered.push(sample);
        }
        assert_eq!(rendered.len(), RATE as usize);
        assert!(rendered.iter().all(|s| s.abs() < 1e-6));
        Ok(())
    }

    #[test]
    fn negative_start_plays_remainder() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        // A 0.2s clip that started 0.1s before the song (countdown into the
        // first downbeat): only its second half is audible.
        let path = write_clip(tempdir.path(), "hint.wav", 8820);
        let clips = vec![PilotClip {
            start_secs: -0.1,
            path,
        }];
        let mut source = PilotSource::new(&clips, RATE, Duration::ZERO, Duration::from_secs(1))?;

        let mut rendered = Vec::new();
        while let Some(sample) = source.next_sample()? {
            rendered.push(sample);
        }
        // Energy right at the start (the clip is already sounding).
        assert!(rendered[..100].iter().any(|s| s.abs() > 0.1));
        // Silence after the clip ends (~0.1s in).
        assert!(rendered[5000..].iter().all(|s| s.abs() < 1e-6));
        Ok(())
    }

    #[test]
    fn overlapping_clips_sum() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        let a = write_clip(tempdir.path(), "a.wav", 4410);
        let b = write_clip(tempdir.path(), "b.wav", 4410);
        let clips = vec![
            PilotClip {
                start_secs: 0.5,
                path: a,
            },
            PilotClip {
                start_secs: 0.55,
                path: b,
            },
        ];
        let mut source = PilotSource::new(&clips, RATE, Duration::ZERO, Duration::from_secs(2))?;

        let mut rendered = Vec::new();
        while let Some(sample) = source.next_sample()? {
            rendered.push(sample);
        }
        // In the overlap region both clips contribute (sum is larger than a
        // single clip's amplitude, clamped at 1.0).
        let single = rendered[22100];
        let overlap = rendered[(0.57 * RATE as f64) as usize];
        assert!(overlap > single, "overlap {overlap} vs single {single}");
        Ok(())
    }
}
