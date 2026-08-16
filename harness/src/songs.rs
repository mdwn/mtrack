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
//! Generates diagnostic songs.
//!
//! The example songs shipped in `examples/` are demo material; these are built
//! to be *identifiable*. Each track carries a distinct steady tone, so
//! capturing the interface's output and asking "which frequency is on which
//! physical channel?" is a direct test of track mapping — something no
//! example song can tell us.

use std::fmt::Write as _;
use std::path::Path;

/// Sample rate for generated audio. Matches mtrack's default output rate so
/// the routing checks are not measuring the resampler.
pub const SAMPLE_RATE: u32 = 44_100;

/// Peak amplitude of generated tones.
///
/// Deliberately conservative: this suite drives real speakers. Raising it is an
/// explicit opt-in via `MTRACK_E2E_AMPLITUDE`.
const DEFAULT_AMPLITUDE: f32 = 0.25;

/// Tone frequencies assigned to tracks, in order.
///
/// Spaced by roughly a factor of 1.57 so no frequency is a low-order harmonic
/// of another. Harmonically related tones would make a Goertzel measurement on
/// one channel light up for a neighbour's leakage.
pub const TRACK_TONES: [f32; 8] = [400.0, 630.0, 990.0, 1560.0, 2460.0, 3870.0, 6090.0, 9580.0];

/// Returns the configured tone amplitude.
pub fn amplitude() -> f32 {
    std::env::var("MTRACK_E2E_AMPLITUDE")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|a| *a > 0.0 && *a <= 1.0)
        .unwrap_or(DEFAULT_AMPLITUDE)
}

/// One audio track within a generated song.
#[derive(Debug, Clone)]
pub struct TrackSpec {
    pub name: String,
    /// File name within the song directory.
    pub file: String,
    /// 1-based channel within `file`.
    pub file_channel: u16,
    /// The tone this track carries, if it is a tone track. `None` means the
    /// track holds something else (a click, silence) and is not identifiable
    /// by frequency.
    pub tone_hz: Option<f32>,
    /// When set, this track is a click track: accented downbeats at this
    /// tempo, which is what `Song::initialize` analyses into a beat grid.
    pub click: Option<ClickSpec>,
}

/// A generated click track's tempo and meter.
#[derive(Debug, Clone, Copy)]
pub struct ClickSpec {
    pub bpm: f32,
    pub beats_per_measure: usize,
    pub measures: usize,
}

/// A song the suite generates and plays.
#[derive(Debug, Clone)]
pub struct SongSpec {
    /// Display name, as it appears in playlists and the API.
    pub name: String,
    /// Directory name under `songs/`.
    pub dir: String,
    pub tracks: Vec<TrackSpec>,
    pub duration_secs: f32,
    /// Optional MIDI file to generate alongside the audio.
    pub midi: Option<MidiSpec>,
    /// Optional lighting shows.
    pub lighting: Vec<LightingSpec>,
}

/// A generated MIDI file with events at known positions.
#[derive(Debug, Clone)]
pub struct MidiSpec {
    pub file: String,
    pub tempo_bpm: f32,
    /// MIDI channel the notes are sent on, 0-based.
    pub channel: u8,
    /// `(beat, note)` pairs. Beat 0.0 is the start of the file.
    pub notes: Vec<(f32, u8)>,
}

impl MidiSpec {
    /// A file with one note per beat, ascending from middle C.
    ///
    /// Distinct notes make each beat individually identifiable when the
    /// stream is captured back, so a dropped or reordered event is visible
    /// rather than just a wrong total.
    pub fn scale(file: &str, tempo_bpm: f32, beats: usize) -> MidiSpec {
        MidiSpec {
            file: file.to_string(),
            tempo_bpm,
            channel: 0,
            notes: (0..beats)
                .map(|b| (b as f32, 60 + (b as u8 % 12)))
                .collect(),
        }
    }

    /// Beats per minute expressed as microseconds per quarter note.
    pub fn micros_per_beat(&self) -> u32 {
        (60_000_000.0 / self.tempo_bpm) as u32
    }

    /// Expected MIDI beat clock ticks per second at this tempo.
    ///
    /// The MIDI spec fixes the clock at 24 pulses per quarter note, so this is
    /// what a captured clock stream must average.
    pub fn expected_clock_hz(&self) -> f32 {
        self.tempo_bpm * 24.0 / 60.0
    }
}

/// A generated lighting show.
#[derive(Debug, Clone)]
pub struct LightingSpec {
    pub name: String,
    /// Path within the song directory, e.g. `lighting/show.light`.
    pub file: String,
    /// The show's DSL source.
    pub source: String,
}

impl LightingSpec {
    /// A show that drives `group` through a few timed cues.
    ///
    /// Every effect carries an explicit duration: the lighting engine requires
    /// finite durations, so an effect without one is a parse error rather than
    /// a perpetual state.
    /// A show timed in bars and beats, with **no** `tempo` block of its own.
    ///
    /// It therefore only resolves if the song supplies a tempo map — which,
    /// for a song whose tempo comes from its click track, means the grid was
    /// derived and converted correctly. A show like this is the only way to
    /// observe that conversion from outside the player.
    pub fn at_bar(name: &str, file: &str, group: &str, bar: u32) -> LightingSpec {
        let source = format!(
            r#"# Generated by the mtrack hardware e2e suite.
show "{name}" {{
    @{bar}/1
    {group}: static color: "red", dimmer: 100%, duration: 2s
}}
"#
        );
        LightingSpec {
            name: name.to_string(),
            file: file.to_string(),
            source,
        }
    }

    pub fn simple(name: &str, file: &str, group: &str) -> LightingSpec {
        let source = format!(
            r#"# Generated by the mtrack hardware e2e suite.
show "{name}" {{
    @00:00.000
    {group}: static color: "red", dimmer: 100%, duration: 2s

    @00:02.000
    {group}: static color: "green", dimmer: 75%, duration: 2s

    @00:04.000
    {group}: dimmer start_level: 1.0, end_level: 0.0, duration: 2s, curve: sine
}}
"#
        );
        LightingSpec {
            name: name.to_string(),
            file: file.to_string(),
            source,
        }
    }
}

impl SongSpec {
    /// Builds a song whose tracks each carry a distinct tone.
    ///
    /// Every track is written into its own single-channel WAV so that a track
    /// maps cleanly onto one output channel and one tone.
    /// More tracks than there are defined tones is clamped rather than
    /// rejected: the caller's channel count comes from discovered hardware, and
    /// a rig with more patched channels than we have tones is a reason to
    /// verify fewer of them, not to abort the run.
    pub fn tones(name: &str, dir: &str, track_count: usize, duration_secs: f32) -> SongSpec {
        let track_count = track_count.min(TRACK_TONES.len());

        let tracks = (0..track_count)
            .map(|i| TrackSpec {
                name: format!("tone{}", i + 1),
                file: format!("tone{}.wav", i + 1),
                file_channel: 1,
                tone_hz: Some(TRACK_TONES[i]),
                click: None,
            })
            .collect();

        SongSpec {
            name: name.to_string(),
            dir: dir.to_string(),
            tracks,
            duration_secs,
            midi: None,
            lighting: Vec::new(),
        }
    }

    /// The tone carried by a named track, if any.
    pub fn tone_for(&self, track: &str) -> Option<f32> {
        self.tracks
            .iter()
            .find(|t| t.name == track)
            .and_then(|t| t.tone_hz)
    }

    /// Adds a click track, which `Song::initialize` analyses into a beat grid.
    ///
    /// The track must be named `click`: that is how `analyze_click_track`
    /// finds it.
    pub fn with_click(mut self, bpm: f32, beats_per_measure: usize, measures: usize) -> SongSpec {
        self.tracks.push(TrackSpec {
            name: "click".to_string(),
            file: "click.wav".to_string(),
            file_channel: 1,
            tone_hz: None,
            click: Some(ClickSpec {
                bpm,
                beats_per_measure,
                measures,
            }),
        });
        self
    }

    /// Adds a MIDI file to this song.
    pub fn with_midi(mut self, midi: MidiSpec) -> SongSpec {
        self.midi = Some(midi);
        self
    }

    /// Adds a lighting show to this song.
    pub fn with_lighting(mut self, show: LightingSpec) -> SongSpec {
        self.lighting.push(show);
        self
    }

    /// Writes the song directory: audio files, MIDI, lighting, `song.yaml`.
    pub fn write(&self, songs_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let dir = songs_root.join(&self.dir);
        std::fs::create_dir_all(&dir)?;

        for track in &self.tracks {
            if let Some(click) = track.click {
                write_click_wav(
                    &dir.join(&track.file),
                    click.bpm,
                    click.beats_per_measure,
                    click.measures,
                )?;
            } else if let Some(hz) = track.tone_hz {
                write_tone_wav(&dir.join(&track.file), hz, self.duration_secs)?;
            }
        }

        if let Some(midi) = &self.midi {
            write_midi(&dir.join(&midi.file), midi)?;
        }

        for show in &self.lighting {
            let path = dir.join(&show.file);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, &show.source)?;
        }

        std::fs::write(dir.join("song.yaml"), self.render_yaml())?;
        Ok(())
    }

    fn render_yaml(&self) -> String {
        let mut out = String::from("# Generated by the mtrack hardware e2e suite.\n");
        out.push_str("kind: song\n");
        let _ = writeln!(out, "name: \"{}\"", self.name);

        if let Some(midi) = &self.midi {
            let _ = writeln!(out, "midi_file: \"{}\"", midi.file);
        }

        if !self.lighting.is_empty() {
            out.push_str("lighting:\n");
            for show in &self.lighting {
                let _ = writeln!(out, "  - name: \"{}\"", show.name);
                let _ = writeln!(out, "    file: \"{}\"", show.file);
            }
        }

        out.push_str("tracks:\n");
        for track in &self.tracks {
            let _ = writeln!(out, "  - name: \"{}\"", track.name);
            let _ = writeln!(out, "    file: \"{}\"", track.file);
            let _ = writeln!(out, "    file_channel: {}", track.file_channel);
        }
        out
    }
}

/// Ticks per quarter note in generated MIDI files.
///
/// 480 is the common DAW value and divides evenly by 24, so beat clock pulses
/// land on exact ticks rather than accumulating rounding error.
const TICKS_PER_BEAT: u16 = 480;

/// Writes a single-track MIDI file with a tempo and the spec's notes.
pub fn write_midi(path: &Path, spec: &MidiSpec) -> Result<(), Box<dyn std::error::Error>> {
    use midly::num::{u15, u24, u28, u4, u7};
    use midly::{
        Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind,
    };

    let mut track = Track::new();
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(spec.micros_per_beat()))),
    });

    // MIDI deltas are relative, so events are emitted in time order and each
    // delta is the gap since the previous event -- including note-offs.
    let mut events: Vec<(u32, bool, u8)> = Vec::new();
    for (beat, note) in &spec.notes {
        let on = (*beat * TICKS_PER_BEAT as f32) as u32;
        // Each note is held for most of its beat, leaving a gap so consecutive
        // notes are distinguishable in a captured stream.
        let off = on + (TICKS_PER_BEAT as u32 * 3) / 4;
        events.push((on, true, *note));
        events.push((off, false, *note));
    }
    events.sort_by_key(|(tick, is_on, _)| (*tick, *is_on));

    let mut previous = 0u32;
    for (tick, is_on, note) in events {
        let message = if is_on {
            MidiMessage::NoteOn {
                key: u7::new(note),
                vel: u7::new(100),
            }
        } else {
            MidiMessage::NoteOff {
                key: u7::new(note),
                vel: u7::new(0),
            }
        };
        track.push(TrackEvent {
            delta: u28::new(tick - previous),
            kind: TrackEventKind::Midi {
                channel: u4::new(spec.channel),
                message,
            },
        });
        previous = tick;
    }

    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    let smf = Smf {
        header: Header::new(
            Format::SingleTrack,
            Timing::Metrical(u15::new(TICKS_PER_BEAT)),
        ),
        tracks: vec![track],
    };
    smf.save(path)?;
    Ok(())
}

/// Writes a mono WAV of percussive clicks at a fixed tempo, accenting each
/// downbeat.
///
/// The accent is what `measure_starts` is derived from — the detector reads
/// measure boundaries off louder onsets — so a click track with uniform
/// amplitude yields beats but no bars, and every measure-based cue then
/// resolves against nothing.
///
/// Each click is a short burst with a fast exponential decay: a sharp
/// transient is what onset detection keys on, and a slow attack would move the
/// detected time away from the beat it represents.
pub fn write_click_wav(
    path: &Path,
    bpm: f32,
    beats_per_measure: usize,
    measures: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;

    let beat_samples = (60.0 / bpm * SAMPLE_RATE as f32) as usize;
    let click_len = (0.03 * SAMPLE_RATE as f32) as usize;
    let total_beats = beats_per_measure * measures;

    for beat in 0..total_beats {
        let accented = beat % beats_per_measure == 0;
        let hz = if accented { 1600.0_f32 } else { 900.0_f32 };
        let peak = if accented { 0.95_f32 } else { 0.4_f32 };

        for i in 0..beat_samples {
            let value = if i < click_len {
                let t = i as f32 / SAMPLE_RATE as f32;
                // Exponential decay: a hard attack and a short tail, so the
                // onset lands on the beat rather than smeared after it.
                let envelope = (-30.0 * t).exp();
                peak * envelope * (std::f32::consts::TAU * hz * t).sin()
            } else {
                0.0
            };
            writer.write_sample((value * i16::MAX as f32) as i16)?;
        }
    }

    writer.finalize()?;
    Ok(())
}

/// Writes a mono WAV containing a steady sine tone.
///
/// The tone is fixed-amplitude apart from short raised-cosine fades at each
/// end; without them the discontinuity at the boundaries is an audible click
/// and puts broadband energy into the capture, which would blunt the frequency
/// measurement the routing check depends on.
pub fn write_tone_wav(
    path: &Path,
    hz: f32,
    duration_secs: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(path, spec)?;
    let total = (duration_secs * SAMPLE_RATE as f32) as usize;
    let fade = (0.01 * SAMPLE_RATE as f32) as usize;
    let amplitude = amplitude();

    for i in 0..total {
        let t = i as f32 / SAMPLE_RATE as f32;
        let envelope = if i < fade {
            0.5 * (1.0 - (std::f32::consts::PI * i as f32 / fade as f32).cos())
        } else if i + fade >= total {
            let remaining = total - i;
            0.5 * (1.0 - (std::f32::consts::PI * remaining as f32 / fade as f32).cos())
        } else {
            1.0
        };

        let value = amplitude * envelope * (std::f32::consts::TAU * hz * t).sin();
        writer.write_sample((value * i16::MAX as f32) as i16)?;
    }

    writer.finalize()?;
    Ok(())
}

#[cfg(test)]
mod click_track_tests {
    use super::*;

    /// The generated click track has to be something the real analyser can
    /// read. A grid that comes back empty takes the whole tempo chain with it,
    /// and the failure surfaces far away as an empty playlist rather than as
    /// anything about audio.
    #[test]
    fn a_generated_click_track_yields_a_beat_grid() {
        let dir = tempfile::tempdir().expect("tempdir");
        let song = SongSpec::tones("Clicked", "clicked", 1, 16.0)
            .with_click(120.0, 4, 8)
            .with_lighting(LightingSpec::at_bar(
                "Bar Cue",
                "lighting/bars.light",
                crate::project::LIGHTING_GROUP,
                3,
            ));
        song.write(dir.path()).expect("write song");

        let song_dir = dir.path().join("clicked");
        let cfg = mtrack::config::Song::deserialize(&song_dir.join("song.yaml"))
            .expect("song.yaml parses");
        // A bar-timed show with no `tempo` block only parses if the click
        // track produced a usable grid, so loading at all is the assertion.
        let loaded = mtrack::songs::Song::new(&song_dir, &cfg).expect("song loads");

        let grid = loaded.beat_grid().expect("a click track yields a grid");
        assert_eq!(grid.beats.len(), 32, "8 measures of 4 beats");
        assert_eq!(grid.measure_starts.len(), 8, "one start per measure");
        // The first onset lands within a detector hop of zero.
        assert!(
            grid.beats[0] < 256.0 / 44100.0 * 2.0,
            "first beat at {}s",
            grid.beats[0]
        );
    }
}
