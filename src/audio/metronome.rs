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

//! A generated metronome click track.
//!
//! [`MetronomeSource`] is a virtual, mono [`SampleSource`] that renders
//! accented and normal clicks at the song's beat grid positions, synthesized
//! directly at the device sample rate (or from optional per-click sample
//! files). It participates in the mixer like any file-backed track: wrap it
//! in a `ChannelMappedSource` labeled with the configured track name and it
//! gets routing and per-track gain for free.

use std::error::Error;
use std::path::Path;
use std::time::Duration;

use super::click_analysis::BeatGrid;
use super::sample_source::error::SampleSourceError;
use super::sample_source::traits::SampleSource;
use crate::config::metronome::{
    ClickSound, MetronomeConfig, DEFAULT_ACCENT_FREQ, DEFAULT_ACCENT_VOLUME, DEFAULT_NORMAL_FREQ,
    DEFAULT_NORMAL_VOLUME,
};

/// Length of a synthesized click in seconds.
const SYNTH_CLICK_SECS: f64 = 0.05;
/// Attack ramp of a synthesized click in seconds.
const SYNTH_ATTACK_SECS: f64 = 0.001;
/// Exponential decay time constant of a synthesized click in seconds.
const SYNTH_DECAY_TAU_SECS: f64 = 0.012;

/// The kind of click at a beat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickKind {
    /// Downbeat / accent-group start.
    Accent,
    /// Any other beat.
    Normal,
}

/// A scheduled click, positioned in samples relative to the source start.
/// Positions can be negative when playback starts mid-click (seek).
#[derive(Debug, Clone, Copy)]
pub struct ClickEvent {
    pub sample_pos: i64,
    pub kind: ClickKind,
}

/// Derives click events from a beat grid.
///
/// Accents fall on measure starts and, when `accent_groups` is non-empty, on
/// each group start within the measure (e.g. `[3, 2, 2]` accents beats 1, 4
/// and 6). `start_time` shifts events so positions are relative to playback
/// start rather than song start.
pub fn click_events_from_beat_grid(
    grid: &BeatGrid,
    accent_groups: &[u32],
    sample_rate: u32,
    start_time: Duration,
) -> Vec<ClickEvent> {
    // Beat offsets within a measure that get an accent.
    let group_starts: Vec<usize> = {
        let mut starts = vec![0usize];
        let mut acc = 0usize;
        for group in accent_groups {
            acc += *group as usize;
            starts.push(acc);
        }
        starts
    };

    let start_secs = start_time.as_secs_f64();
    let rate = sample_rate as f64;

    grid.beats
        .iter()
        .enumerate()
        .map(|(i, beat_secs)| {
            // The measure this beat belongs to: the last measure start at or
            // before the beat index.
            let measure_start = grid
                .measure_starts
                .partition_point(|&start| start <= i)
                .checked_sub(1)
                .map(|m| grid.measure_starts[m])
                .unwrap_or(0);
            let beat_in_measure = i - measure_start;
            let kind = if group_starts.contains(&beat_in_measure) {
                ClickKind::Accent
            } else {
                ClickKind::Normal
            };
            ClickEvent {
                sample_pos: ((beat_secs - start_secs) * rate).round() as i64,
                kind,
            }
        })
        .collect()
}

/// Synthesizes a click: a sine burst with a short attack and exponential decay.
fn synthesize_click(freq: f64, volume: f64, sample_rate: u32) -> Vec<f32> {
    let rate = sample_rate as f64;
    let total = (rate * SYNTH_CLICK_SECS).max(1.0) as usize;
    let attack = (rate * SYNTH_ATTACK_SECS).max(1.0) as usize;

    (0..total)
        .map(|n| {
            let t = n as f64 / rate;
            let envelope = if n < attack {
                n as f64 / attack as f64
            } else {
                (-((n - attack) as f64 / rate) / SYNTH_DECAY_TAU_SECS).exp()
            };
            ((std::f64::consts::TAU * freq * t).sin() * envelope * volume) as f32
        })
        .collect()
}

/// Renders the waveform for one click sound role. Fields resolve per-field:
/// the song's sound, then the player-wide default sound, then the built-in
/// synthesized click.
fn render_click_sound(
    sound: Option<&ClickSound>,
    default_sound: Option<&ClickSound>,
    builtin_freq: f64,
    builtin_volume: f64,
    base_path: &Path,
    sample_rate: u32,
) -> Result<Vec<f32>, Box<dyn Error>> {
    let pick = |get: fn(&ClickSound) -> Option<f64>| {
        sound.and_then(get).or_else(|| default_sound.and_then(get))
    };
    // A song-level file wins outright; otherwise a song freq overrides a
    // default file (the song asked for a synth click).
    let file = match (
        sound.and_then(|s| s.file.as_deref()),
        sound.and_then(|s| s.freq),
    ) {
        (Some(file), _) => Some(file),
        (None, Some(_)) => None,
        (None, None) => default_sound.and_then(|s| s.file.as_deref()),
    };
    let freq = pick(|s| s.freq).unwrap_or(builtin_freq);
    let volume = pick(|s| s.volume).unwrap_or(builtin_volume);
    match file {
        Some(file) => {
            let path = if Path::new(file).is_absolute() {
                Path::new(file).to_path_buf()
            } else {
                base_path.join(file)
            };
            super::monoclip::load_mono_clip(&path, volume, sample_rate)
        }
        None => Ok(synthesize_click(freq, volume, sample_rate)),
    }
}

/// A virtual metronome track rendered from a beat grid.
pub struct MetronomeSource {
    /// Click events sorted by sample position (relative to source start).
    events: Vec<ClickEvent>,
    accent: Vec<f32>,
    normal: Vec<f32>,
    /// Current playback position in samples.
    position: i64,
    /// The source is exhausted at this position (song end).
    end_position: i64,
    /// Index of the next not-yet-active event.
    next_event: usize,
    /// Currently sounding clicks as (start_position, kind).
    active: Vec<(i64, ClickKind)>,
    sample_rate: u32,
}

impl MetronomeSource {
    /// Creates a metronome source for a song.
    ///
    /// `start_time` is the playback start offset within the song: clicks are
    /// shifted accordingly and a click already in progress at the offset
    /// plays its remaining tail, keeping seek sample-accurate.
    pub fn new(
        grid: &BeatGrid,
        config: &MetronomeConfig,
        defaults: Option<&crate::config::metronome::MetronomeSounds>,
        base_path: &Path,
        sample_rate: u32,
        start_time: Duration,
        song_duration: Duration,
    ) -> Result<Self, Box<dyn Error>> {
        let sounds = config.sounds.as_ref();
        let accent = render_click_sound(
            sounds.and_then(|s| s.accent.as_ref()),
            defaults.and_then(|s| s.accent.as_ref()),
            DEFAULT_ACCENT_FREQ,
            DEFAULT_ACCENT_VOLUME,
            base_path,
            sample_rate,
        )?;
        let normal = render_click_sound(
            sounds.and_then(|s| s.normal.as_ref()),
            defaults.and_then(|s| s.normal.as_ref()),
            DEFAULT_NORMAL_FREQ,
            DEFAULT_NORMAL_VOLUME,
            base_path,
            sample_rate,
        )?;

        let end_position = ((song_duration.saturating_sub(start_time)).as_secs_f64()
            * sample_rate as f64)
            .round() as i64;
        let max_click_len = accent.len().max(normal.len()) as i64;

        let mut events = click_events_from_beat_grid(grid, &config.accent, sample_rate, start_time);
        // Keep only clicks that are at least partially audible in
        // [0, end_position).
        events.retain(|e| e.sample_pos + max_click_len > 0 && e.sample_pos < end_position);
        events.sort_by_key(|e| e.sample_pos);

        Ok(MetronomeSource {
            events,
            accent,
            normal,
            position: 0,
            end_position,
            next_event: 0,
            active: Vec::with_capacity(4),
            sample_rate,
        })
    }
}

impl SampleSource for MetronomeSource {
    fn next_sample(&mut self) -> Result<Option<f32>, SampleSourceError> {
        if self.position >= self.end_position {
            return Ok(None);
        }

        // Activate any clicks that start at or before this position.
        while self.next_event < self.events.len()
            && self.events[self.next_event].sample_pos <= self.position
        {
            let event = self.events[self.next_event];
            self.active.push((event.sample_pos, event.kind));
            self.next_event += 1;
        }

        // Sum currently-sounding clicks and drop finished ones. Split field
        // borrows keep this allocation-free in the audio path.
        let position = self.position;
        let mut sample = 0.0f32;
        let accent = &self.accent;
        let normal = &self.normal;
        self.active.retain(|(start, kind)| {
            let waveform: &[f32] = match kind {
                ClickKind::Accent => accent,
                ClickKind::Normal => normal,
            };
            let idx = position - start;
            if idx < 0 {
                return true;
            }
            match waveform.get(idx as usize) {
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

    const RATE: u32 = 48_000;

    fn simple_grid(beats_per_measure: usize, beat_secs: f64, measures: usize) -> BeatGrid {
        let mut beats = Vec::new();
        let mut measure_starts = Vec::new();
        for m in 0..measures {
            measure_starts.push(beats.len());
            for b in 0..beats_per_measure {
                beats.push((m * beats_per_measure + b) as f64 * beat_secs);
            }
        }
        BeatGrid {
            beats,
            measure_starts,
        }
    }

    #[test]
    fn events_default_accents_on_downbeats() {
        // 7 beats per measure (7/8), eighth = 0.25s.
        let grid = simple_grid(7, 0.25, 2);
        let events = click_events_from_beat_grid(&grid, &[], RATE, Duration::ZERO);
        assert_eq!(events.len(), 14);
        for (i, event) in events.iter().enumerate() {
            let expected_kind = if i % 7 == 0 {
                ClickKind::Accent
            } else {
                ClickKind::Normal
            };
            assert_eq!(event.kind, expected_kind, "beat {i}");
            assert_eq!(event.sample_pos, (i as f64 * 0.25 * RATE as f64) as i64);
        }
    }

    #[test]
    fn events_accent_groups_7_8() {
        // Grouping [3, 2, 2] accents beats 1, 4 and 6 of each 7-beat measure.
        let grid = simple_grid(7, 0.25, 1);
        let events = click_events_from_beat_grid(&grid, &[3, 2, 2], RATE, Duration::ZERO);
        let kinds: Vec<ClickKind> = events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ClickKind::Accent, // beat 1
                ClickKind::Normal,
                ClickKind::Normal,
                ClickKind::Accent, // beat 4
                ClickKind::Normal,
                ClickKind::Accent, // beat 6
                ClickKind::Normal,
            ]
        );
    }

    #[test]
    fn events_shift_with_start_time() {
        let grid = simple_grid(4, 0.5, 2);
        let events = click_events_from_beat_grid(&grid, &[], RATE, Duration::from_secs_f64(1.0));
        // Beat at 1.0s lands exactly at position 0 after the shift.
        assert!(events.iter().any(|e| e.sample_pos == 0));
        // Beats before the offset have negative positions.
        assert!(events.iter().any(|e| e.sample_pos < 0));
    }

    #[test]
    fn synthesized_click_shape() {
        let click = synthesize_click(1000.0, 1.0, RATE);
        assert_eq!(click.len(), (RATE as f64 * SYNTH_CLICK_SECS) as usize);
        let peak = click.iter().fold(0.0f32, |max, s| max.max(s.abs()));
        assert!(
            peak > 0.5,
            "click should have audible amplitude, got {peak}"
        );
        // Decays towards silence at the end.
        let tail = click[click.len() - 1].abs();
        assert!(tail < 0.05, "click should decay, tail was {tail}");
    }

    #[test]
    fn source_renders_clicks_at_beat_positions() {
        let grid = simple_grid(4, 0.5, 2);
        let config = MetronomeConfig::default();
        let mut source = MetronomeSource::new(
            &grid,
            &config,
            None,
            Path::new("/nonexistent"),
            RATE,
            Duration::ZERO,
            Duration::from_secs(4),
        )
        .unwrap();

        let mut rendered = Vec::new();
        while let Some(sample) = source.next_sample().unwrap() {
            rendered.push(sample);
        }
        assert_eq!(rendered.len(), 4 * RATE as usize);

        // Energy present right after each beat, silence just before the next.
        for beat in 0..8 {
            let start = (beat as f64 * 0.5 * RATE as f64) as usize;
            let window = &rendered[start..start + 200];
            let peak = window.iter().fold(0.0f32, |max, s| max.max(s.abs()));
            assert!(peak > 0.1, "expected click energy at beat {beat}");

            let quiet_at = start + (RATE as f64 * 0.4) as usize;
            let quiet = &rendered[quiet_at..quiet_at + 100];
            let quiet_peak = quiet.iter().fold(0.0f32, |max, s| max.max(s.abs()));
            assert!(
                quiet_peak < 0.01,
                "expected silence between clicks at beat {beat}, got {quiet_peak}"
            );
        }
    }

    #[test]
    fn source_seek_plays_click_tail() {
        let grid = simple_grid(4, 0.5, 2);
        let config = MetronomeConfig::default();
        // Start 5ms into the second beat: its tail should still sound.
        let start_time = Duration::from_secs_f64(0.505);
        let mut source = MetronomeSource::new(
            &grid,
            &config,
            None,
            Path::new("/nonexistent"),
            RATE,
            start_time,
            Duration::from_secs(4),
        )
        .unwrap();

        let mut first: Vec<f32> = Vec::new();
        for _ in 0..200 {
            match source.next_sample().unwrap() {
                Some(sample) => first.push(sample),
                None => break,
            }
        }
        let peak = first.iter().fold(0.0f32, |max, s| max.max(s.abs()));
        assert!(
            peak > 0.01,
            "mid-click seek should play the tail, got {peak}"
        );
    }

    #[test]
    fn player_defaults_apply_when_song_has_no_sounds() {
        use crate::config::metronome::{ClickSound, MetronomeSounds};

        let grid = simple_grid(4, 0.5, 1);
        let config = MetronomeConfig::default();
        let render = |defaults: Option<&MetronomeSounds>| {
            let mut source = MetronomeSource::new(
                &grid,
                &config,
                defaults,
                Path::new("/nonexistent"),
                RATE,
                Duration::ZERO,
                Duration::from_secs(1),
            )
            .unwrap();
            let mut peak = 0.0f32;
            while let Some(sample) = source.next_sample().unwrap() {
                peak = peak.max(sample.abs());
            }
            peak
        };

        let builtin_peak = render(None);
        let defaults = MetronomeSounds {
            accent: Some(ClickSound {
                file: None,
                freq: Some(1125.0),
                volume: Some(0.25),
            }),
            normal: Some(ClickSound {
                file: None,
                freq: Some(1125.0),
                volume: Some(0.25),
            }),
        };
        let default_peak = render(Some(&defaults));
        assert!(
            default_peak < builtin_peak * 0.5,
            "player defaults should scale the click ({default_peak} vs {builtin_peak})"
        );
    }

    #[test]
    fn source_exhausts_at_song_end() {
        let grid = simple_grid(4, 0.5, 1);
        let config = MetronomeConfig::default();
        let mut source = MetronomeSource::new(
            &grid,
            &config,
            None,
            Path::new("/nonexistent"),
            RATE,
            Duration::ZERO,
            Duration::from_secs(1),
        )
        .unwrap();

        let mut count = 0usize;
        while source.next_sample().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, RATE as usize);
        // Stays exhausted.
        assert!(source.next_sample().unwrap().is_none());
    }
}
