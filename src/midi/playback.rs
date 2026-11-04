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
use std::time::Duration;

use midly::{Format, MetaMessage, MidiMessage, TrackEvent, TrackEventKind};

/// A single MIDI event with its absolute timestamp.
#[derive(Clone)]
pub(crate) struct TimedMidiEvent {
    pub time: Duration,
    pub channel: u8,
    pub message: MidiMessage,
}

/// Pre-computed, time-sorted MIDI event stream.
/// Replaces nodi's Sheet + Ticker + Player with a single-pass pre-computation
/// that goes directly from midly's TrackEvents to absolute-timestamped events.
pub(crate) struct PrecomputedMidi {
    events: Vec<TimedMidiEvent>,
}

/// A tempo change at a specific tick position.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TempoEntry {
    pub(crate) tick: u64,
    pub(crate) micros_per_beat: u32,
}

/// A time signature from a specific tick position.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SignatureEntry {
    pub(crate) tick: u64,
    pub(crate) numerator: u8,
    /// The denominator is `2^denominator_power`, as the file has it.
    pub(crate) denominator_power: u8,
}

/// Default tempo: 120 BPM = 500_000 microseconds per beat.
const DEFAULT_MICROS_PER_BEAT: u32 = 500_000;

/// A MIDI file's timing: its tempo and time-signature events at their
/// ticks, by the one rule for where a file's tempo lives. A Format 0 file's
/// are on its track; a Format 1 file's on the conductor track (track 0) and
/// nowhere else; a Format 2 file's on each track, offset by the tracks
/// before it. The tempo is 120 BPM until the first tempo event, and the
/// last event at a tick wins. Playback, the beat clock and the tempo guess
/// all read a file through this.
pub(crate) struct MidiTiming {
    pub(crate) tempo: Vec<TempoEntry>,
    pub(crate) signatures: Vec<SignatureEntry>,
    pub(crate) ticks_per_beat: u16,
    /// The file's length in ticks: the longest track's.
    pub(crate) total_ticks: u64,
}

impl MidiTiming {
    /// The timing of a parsed file's tracks.
    pub(crate) fn of(tracks: &[Vec<TrackEvent<'_>>], ticks_per_beat: u16, format: Format) -> Self {
        let mut timing = MidiTiming {
            tempo: Vec::new(),
            signatures: Vec::new(),
            ticks_per_beat,
            total_ticks: 0,
        };
        match format {
            Format::SingleTrack | Format::Parallel => {
                if let Some(track) = tracks.first() {
                    (timing.tempo, timing.signatures) = extract_meta(track);
                }
                timing.total_ticks = tracks.iter().map(|t| track_ticks(t)).max().unwrap_or(0);
            }
            Format::Sequential => {
                // Each track follows the one before, so its events are
                // placed after the ticks before it.
                for track in tracks {
                    let (tempo, signatures) = extract_meta(track);
                    let offset = timing.total_ticks;
                    timing.tempo.extend(tempo.into_iter().map(|e| TempoEntry {
                        tick: e.tick + offset,
                        ..e
                    }));
                    timing
                        .signatures
                        .extend(signatures.into_iter().map(|e| SignatureEntry {
                            tick: e.tick + offset,
                            ..e
                        }));
                    timing.total_ticks += track_ticks(track);
                }
            }
        }
        timing
    }

    /// The clock's pulse times over the file, and on past its end at its
    /// last tempo until `until` (see
    /// [`super::beat_clock::PrecomputedBeatClock::from_timing_until`]).
    pub(crate) fn beat_clock_until(&self, until: Duration) -> Vec<Duration> {
        super::beat_clock::PrecomputedBeatClock::from_timing_until(self, until)
            .ticks_from(Duration::ZERO)
            .to_vec()
    }
}

/// A track's length in ticks.
fn track_ticks(track: &[TrackEvent<'_>]) -> u64 {
    track.iter().map(|e| e.delta.as_int() as u64).sum()
}

/// A track's tempo and time-signature events at their ticks.
fn extract_meta(track: &[TrackEvent<'_>]) -> (Vec<TempoEntry>, Vec<SignatureEntry>) {
    let mut tempo = Vec::new();
    let mut signatures = Vec::new();
    let mut tick: u64 = 0;
    for event in track {
        tick += event.delta.as_int() as u64;
        match event.kind {
            TrackEventKind::Meta(MetaMessage::Tempo(micros_per_beat)) => tempo.push(TempoEntry {
                tick,
                micros_per_beat: micros_per_beat.as_int(),
            }),
            TrackEventKind::Meta(MetaMessage::TimeSignature(
                numerator,
                denominator_power,
                _,
                _,
            )) => signatures.push(SignatureEntry {
                tick,
                numerator,
                denominator_power,
            }),
            _ => {}
        }
    }
    (tempo, signatures)
}

/// The one walk from a file's ticks to its time and back: 120 BPM until
/// the first tempo event, each event's tempo from its tick on, the last at
/// a tick winning. A walk only goes forward; a position behind the last is
/// answered as the last.
pub(crate) struct TempoWalk<'a> {
    tempo: &'a [TempoEntry],
    ticks_per_beat: f64,
    next: usize,
    /// The position reached, in ticks and in microseconds, and the tempo
    /// there.
    tick: f64,
    micros: f64,
    micros_per_tick: f64,
}

impl<'a> TempoWalk<'a> {
    pub(crate) fn new(tempo: &'a [TempoEntry], ticks_per_beat: u16) -> Self {
        let ticks_per_beat = ticks_per_beat as f64;
        TempoWalk {
            tempo,
            ticks_per_beat,
            next: 0,
            tick: 0.0,
            micros: 0.0,
            micros_per_tick: DEFAULT_MICROS_PER_BEAT as f64 / ticks_per_beat,
        }
    }

    /// Takes the next tempo event, which starts at or before the position.
    fn take(&mut self, entry: &TempoEntry) {
        self.micros_per_tick = entry.micros_per_beat as f64 / self.ticks_per_beat;
        self.next += 1;
    }

    /// The time at `tick`, in microseconds, the walk advanced to it.
    pub(crate) fn micros_at(&mut self, tick: f64) -> f64 {
        while let Some(entry) = self.tempo.get(self.next) {
            if entry.tick as f64 > tick {
                break;
            }
            if entry.tick as f64 > self.tick {
                self.micros += (entry.tick as f64 - self.tick) * self.micros_per_tick;
                self.tick = entry.tick as f64;
            }
            self.take(entry);
        }
        if tick > self.tick {
            self.micros += (tick - self.tick) * self.micros_per_tick;
            self.tick = tick;
        }
        self.micros
    }

    /// The tick at `micros`, the walk advanced to the tempo in effect there.
    pub(crate) fn tick_at(&mut self, micros: f64) -> f64 {
        while let Some(entry) = self.tempo.get(self.next) {
            if entry.tick as f64 > self.tick {
                let at = self.micros + (entry.tick as f64 - self.tick) * self.micros_per_tick;
                if at > micros {
                    break;
                }
                self.micros = at;
                self.tick = entry.tick as f64;
            }
            self.take(entry);
        }
        // A tempo of no length (or none) never gets anywhere: the position
        // reached is the answer.
        let rate = self.micros_per_tick;
        if micros <= self.micros || !rate.is_finite() || rate <= 0.0 {
            return self.tick;
        }
        self.tick + (micros - self.micros) / self.micros_per_tick
    }

    /// The tempo at the position reached, as microseconds per tick.
    pub(crate) fn micros_per_tick(&self) -> f64 {
        self.micros_per_tick
    }
}

/// Result of processing a single track: MIDI events and total track duration.
struct TrackResult {
    events: Vec<TimedMidiEvent>,
    /// Total duration of the track (including trailing silence after last MIDI event).
    total_duration: Duration,
}

impl PrecomputedMidi {
    /// Builds a pre-computed MIDI timeline from parsed tracks, timed by
    /// `timing` ([`MidiTiming::of`] the same tracks).
    ///
    /// Single-pass algorithm: for each track, accumulates tick positions and
    /// converts to absolute time using tempo changes. For Format 1 (parallel),
    /// the conductor track (track 0) provides the tempo map for all tracks.
    pub fn from_tracks(
        tracks: &[Vec<TrackEvent<'_>>],
        timing: &MidiTiming,
        format: Format,
    ) -> Self {
        let tpb = timing.ticks_per_beat;
        match format {
            Format::SingleTrack => {
                let events = if let Some(track) = tracks.first() {
                    Self::process_track(track, tpb, &timing.tempo).events
                } else {
                    Vec::new()
                };
                PrecomputedMidi { events }
            }
            Format::Parallel => {
                // Skip track 0 (conductor) — it provides the tempo map but should
                // not emit MIDI events. Non-conformant files with MIDI events on
                // track 0 would otherwise get double-tempo-mapped. A file whose
                // conductor sets no tempo leaves each track to its own events.
                let mut all_events = Vec::new();
                for track in tracks.iter().skip(1) {
                    let own;
                    let tempo: &[TempoEntry] = if timing.tempo.is_empty() {
                        own = extract_meta(track).0;
                        &own
                    } else {
                        &timing.tempo
                    };
                    let mut track_events = Self::process_track(track, tpb, tempo).events;
                    all_events.append(&mut track_events);
                }
                all_events.sort_by(|a, b| a.time.cmp(&b.time));
                PrecomputedMidi { events: all_events }
            }
            Format::Sequential => {
                // Each track is processed independently with inline tempo handling only.
                // Format 2 is rare; if a sequential file has conductor-style tempo tracks,
                // they won't cross-apply to other tracks (each track is self-contained).
                let mut all_events = Vec::new();
                let mut cumulative_offset = Duration::ZERO;
                for track in tracks {
                    let result = Self::process_track(track, tpb, &extract_meta(track).0);
                    for event in result.events {
                        all_events.push(TimedMidiEvent {
                            time: event.time + cumulative_offset,
                            channel: event.channel,
                            message: event.message,
                        });
                    }
                    // Use total track duration (including trailing silence) as offset
                    cumulative_offset += result.total_duration;
                }
                PrecomputedMidi { events: all_events }
            }
        }
    }

    /// Processes a single track into timed events, timed by `tempo`: the
    /// conductor's, or the track's own. Returns both the MIDI events and the
    /// total track duration (including trailing silence).
    ///
    /// Per the MIDI spec, a delta is timed at the tempo current when it
    /// starts: a tempo change at the same tick as a note applies to the
    /// *next* delta, not retroactively, which is how the walk has it.
    fn process_track(track: &[TrackEvent<'_>], tpb: u16, tempo: &[TempoEntry]) -> TrackResult {
        let mut events = Vec::new();
        let mut walk = TempoWalk::new(tempo, tpb);
        let mut tick: u64 = 0;

        for event in track {
            tick += event.delta.as_int() as u64;
            let micros = walk.micros_at(tick as f64);

            // Emit MIDI events (skip meta events)
            if let TrackEventKind::Midi { channel, message } = event.kind {
                events.push(TimedMidiEvent {
                    time: Duration::from_micros(micros.round() as u64),
                    channel: channel.as_int(),
                    message,
                });
            }
        }

        TrackResult {
            events,
            total_duration: Duration::from_micros(walk.micros_at(tick as f64).round() as u64),
        }
    }

    /// Creates a PrecomputedMidi by taking ownership of a Vec of events.
    #[cfg(test)]
    pub fn from_events(events: Vec<TimedMidiEvent>) -> Self {
        PrecomputedMidi { events }
    }

    #[cfg(test)]
    /// Returns the slice of events starting from the first event at or after `start_time`.
    pub fn events_from(&self, start_time: Duration) -> &[TimedMidiEvent] {
        let idx = self.events.partition_point(|e| e.time < start_time);
        &self.events[idx..]
    }

    /// Returns all events.
    pub fn events(&self) -> &[TimedMidiEvent] {
        &self.events
    }

    /// Returns true if there are no events.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Returns the number of events.
    pub fn len(&self) -> usize {
        self.events.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file's events timed by its own timing, as the sheet loader has it.
    fn precomputed(tracks: &[Vec<TrackEvent<'_>>], tpb: u16, format: Format) -> PrecomputedMidi {
        PrecomputedMidi::from_tracks(tracks, &MidiTiming::of(tracks, tpb, format), format)
    }
    use midly::{num::u7, MetaMessage, MidiMessage, TrackEvent, TrackEventKind};

    /// Helper: build a TrackEvent with a delta and MIDI note-on.
    fn note_on(delta: u32, channel: u8, key: u8, vel: u8) -> TrackEvent<'static> {
        TrackEvent {
            delta: delta.into(),
            kind: TrackEventKind::Midi {
                channel: channel.into(),
                message: MidiMessage::NoteOn {
                    key: u7::new(key),
                    vel: u7::new(vel),
                },
            },
        }
    }

    /// Helper: build a tempo meta event.
    fn tempo_event(delta: u32, micros_per_beat: u32) -> TrackEvent<'static> {
        TrackEvent {
            delta: delta.into(),
            kind: TrackEventKind::Meta(MetaMessage::Tempo(micros_per_beat.into())),
        }
    }

    /// Helper: build an end-of-track meta event.
    fn end_of_track(delta: u32) -> TrackEvent<'static> {
        TrackEvent {
            delta: delta.into(),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        }
    }

    #[test]
    fn test_empty_tracks() {
        let midi = precomputed(&[], 480, Format::Parallel);
        assert!(midi.is_empty());
        assert_eq!(midi.len(), 0);
    }

    #[test]
    fn test_single_track_basic() {
        // 480 ticks per beat, default 120 BPM = 500_000 µs per beat
        // micros_per_tick = 500_000 / 480 ≈ 1041.667
        let tpb = 480;
        let track = vec![
            note_on(0, 0, 60, 100),   // t=0
            note_on(480, 0, 62, 100), // t=1 beat = 500_000 µs = 0.5s
            note_on(480, 0, 64, 100), // t=2 beats = 1_000_000 µs = 1.0s
            end_of_track(0),
        ];
        let midi = precomputed(&[track], tpb, Format::SingleTrack);

        assert_eq!(midi.len(), 3);
        assert_eq!(midi.events()[0].time, Duration::from_micros(0));
        assert_eq!(midi.events()[1].time, Duration::from_micros(500_000));
        assert_eq!(midi.events()[2].time, Duration::from_micros(1_000_000));
    }

    #[test]
    fn test_parallel_tracks() {
        let tpb = 480;
        // Track 0: conductor (no MIDI events, just tempo)
        let track0 = vec![end_of_track(0)];
        // Track 1: notes at beat 0 and beat 2
        let track1 = vec![
            note_on(0, 0, 60, 100),
            note_on(960, 0, 64, 100), // 2 beats
            end_of_track(0),
        ];
        // Track 2: note at beat 1
        let track2 = vec![
            note_on(480, 1, 62, 100), // 1 beat
            end_of_track(0),
        ];

        let midi = precomputed(&[track0, track1, track2], tpb, Format::Parallel);

        assert_eq!(midi.len(), 3);
        // Events should be sorted by time: beat 0, beat 1, beat 2
        assert_eq!(midi.events()[0].time, Duration::from_micros(0));
        assert_eq!(midi.events()[1].time, Duration::from_micros(500_000));
        assert_eq!(midi.events()[2].time, Duration::from_micros(1_000_000));

        // Verify channels are preserved
        assert_eq!(midi.events()[0].channel, 0); // track1 note
        assert_eq!(midi.events()[1].channel, 1); // track2 note
        assert_eq!(midi.events()[2].channel, 0); // track1 note
    }

    #[test]
    fn test_tempo_change() {
        // Start at 120 BPM (500_000 µs/beat), change to 60 BPM (1_000_000 µs/beat) at beat 1
        let tpb = 480;
        let track = vec![
            note_on(0, 0, 60, 100),      // t=0
            tempo_event(480, 1_000_000), // At beat 1, change to 60 BPM
            note_on(0, 0, 62, 100),      // t=beat 1 = 500_000 µs
            note_on(480, 0, 64, 100), // t=beat 2, but at 60 BPM: 500_000 + 1_000_000 = 1_500_000 µs
            end_of_track(0),
        ];

        let midi = precomputed(&[track], tpb, Format::SingleTrack);

        assert_eq!(midi.len(), 3);
        assert_eq!(midi.events()[0].time, Duration::from_micros(0));
        assert_eq!(midi.events()[1].time, Duration::from_micros(500_000));
        assert_eq!(midi.events()[2].time, Duration::from_micros(1_500_000));
    }

    #[test]
    fn test_tempo_change_parallel_conductor() {
        // In Format 1, tempo from track 0 applies to all tracks
        let tpb = 480;
        // Track 0: conductor with tempo change at beat 1
        let track0 = vec![
            tempo_event(480, 1_000_000), // At beat 1, change to 60 BPM
            end_of_track(0),
        ];
        // Track 1: notes at beats 0, 1, and 2
        let track1 = vec![
            note_on(0, 0, 60, 100),   // t=0
            note_on(480, 0, 62, 100), // t=beat 1 = 500_000 µs (still at 120 BPM)
            note_on(480, 0, 64, 100), // t=beat 2 at 60 BPM: 500_000 + 1_000_000 = 1_500_000 µs
            end_of_track(0),
        ];

        let midi = precomputed(&[track0, track1], tpb, Format::Parallel);

        assert_eq!(midi.len(), 3);
        assert_eq!(midi.events()[0].time, Duration::from_micros(0));
        assert_eq!(midi.events()[1].time, Duration::from_micros(500_000));
        assert_eq!(midi.events()[2].time, Duration::from_micros(1_500_000));
    }

    #[test]
    fn test_seek() {
        let tpb = 480;
        let track = vec![
            note_on(0, 0, 60, 100),   // t=0
            note_on(480, 0, 62, 100), // t=0.5s
            note_on(480, 0, 64, 100), // t=1.0s
            note_on(480, 0, 65, 100), // t=1.5s
            end_of_track(0),
        ];
        let midi = precomputed(&[track], tpb, Format::SingleTrack);

        // Seek to 0 — all events
        let from_zero = midi.events_from(Duration::ZERO);
        assert_eq!(from_zero.len(), 4);

        // Seek to 0.5s — skip first event
        let from_half = midi.events_from(Duration::from_millis(500));
        assert_eq!(from_half.len(), 3);
        assert_eq!(from_half[0].time, Duration::from_micros(500_000));

        // Seek to 0.75s — skip first two events (0.75s > 0.5s)
        let from_750 = midi.events_from(Duration::from_millis(750));
        assert_eq!(from_750.len(), 2);
        assert_eq!(from_750[0].time, Duration::from_micros(1_000_000));

        // Seek past all events
        let from_end = midi.events_from(Duration::from_secs(10));
        assert_eq!(from_end.len(), 0);
    }

    #[test]
    fn test_channel_preserved() {
        let tpb = 480;
        let track = vec![
            note_on(0, 3, 60, 100),
            note_on(0, 7, 62, 100),
            note_on(0, 15, 64, 100),
            end_of_track(0),
        ];
        let midi = precomputed(&[track], tpb, Format::SingleTrack);

        assert_eq!(midi.len(), 3);
        assert_eq!(midi.events()[0].channel, 3);
        assert_eq!(midi.events()[1].channel, 7);
        assert_eq!(midi.events()[2].channel, 15);
    }

    #[test]
    fn test_sequential_tracks_with_trailing_silence() {
        // Format 2: two tracks played sequentially.
        // Track 1: note at t=0, EndOfTrack at 2 beats (1 beat of trailing silence).
        // Track 2: note at t=0, should start at 2-beat offset.
        let tpb = 480;
        let track1 = vec![
            note_on(0, 0, 60, 100),   // t=0
            note_on(480, 0, 62, 100), // t=1 beat = 500_000 µs
            end_of_track(480),        // EndOfTrack at beat 2 (trailing silence)
        ];
        let track2 = vec![
            note_on(0, 1, 72, 100), // t=0 (relative to track 2 start)
            end_of_track(0),
        ];

        let midi = precomputed(&[track1, track2], tpb, Format::Sequential);

        assert_eq!(midi.len(), 3);
        // Track 1 events
        assert_eq!(midi.events()[0].time, Duration::from_micros(0));
        assert_eq!(midi.events()[1].time, Duration::from_micros(500_000));
        // Track 2 starts at beat 2 (1_000_000 µs) — includes the trailing silence
        assert_eq!(midi.events()[2].time, Duration::from_micros(1_000_000));
        assert_eq!(midi.events()[2].channel, 1);
    }

    #[test]
    fn test_from_events_and_into_events() {
        let events = vec![
            TimedMidiEvent {
                time: Duration::from_millis(0),
                channel: 0,
                message: MidiMessage::NoteOn {
                    key: u7::new(60),
                    vel: u7::new(100),
                },
            },
            TimedMidiEvent {
                time: Duration::from_millis(500),
                channel: 1,
                message: MidiMessage::NoteOff {
                    key: u7::new(60),
                    vel: u7::new(0),
                },
            },
        ];

        let midi = PrecomputedMidi::from_events(events);
        assert_eq!(midi.len(), 2);
        assert!(!midi.is_empty());
        assert_eq!(midi.events()[0].channel, 0);
        assert_eq!(midi.events()[1].channel, 1);

        let recovered = midi.events();
        assert_eq!(recovered.len(), 2);
        assert_eq!(recovered[0].time, Duration::from_millis(0));
        assert_eq!(recovered[1].time, Duration::from_millis(500));
    }

    #[test]
    fn test_events_from_boundary() {
        let tpb = 480;
        let track = vec![
            note_on(0, 0, 60, 100),   // t=0
            note_on(480, 0, 62, 100), // t=0.5s
            note_on(480, 0, 64, 100), // t=1.0s
            end_of_track(0),
        ];
        let midi = precomputed(&[track], tpb, Format::SingleTrack);

        // Exact boundary: events_from at exactly an event time
        let from_exact = midi.events_from(Duration::from_micros(500_000));
        assert_eq!(from_exact.len(), 2);
        assert_eq!(from_exact[0].time, Duration::from_micros(500_000));
    }

    #[test]
    fn test_sequential_with_inline_tempo() {
        // Format 2 with inline tempo change within a track
        let tpb = 480;
        let track = vec![
            note_on(0, 0, 60, 100),      // t=0 at 120 BPM
            tempo_event(480, 1_000_000), // At beat 1, change to 60 BPM
            note_on(480, 0, 62, 100),    // t=beat 2 at 60 BPM: 500_000 + 1_000_000
            end_of_track(0),
        ];

        let midi = precomputed(&[track], tpb, Format::Sequential);
        assert_eq!(midi.len(), 2);
        assert_eq!(midi.events()[0].time, Duration::from_micros(0));
        assert_eq!(midi.events()[1].time, Duration::from_micros(1_500_000));
    }

    #[test]
    fn test_parallel_single_track_no_conductor() {
        // Parallel format with only one track (no conductor track to skip)
        let tpb = 480;
        let track0 = vec![note_on(0, 0, 60, 100), end_of_track(0)];

        let midi = precomputed(&[track0], tpb, Format::Parallel);
        // Track 0 is the conductor and is skipped in Parallel format
        assert_eq!(midi.len(), 0);
    }

    #[test]
    fn test_parallel_empty_tracks() {
        let midi = precomputed(&[], 480, Format::Parallel);
        assert!(midi.is_empty());
    }

    #[test]
    fn test_single_track_empty() {
        let midi = precomputed(&[], 480, Format::SingleTrack);
        assert!(midi.is_empty());
    }

    #[test]
    fn test_sequential_empty() {
        let midi = precomputed(&[], 480, Format::Sequential);
        assert!(midi.is_empty());
    }

    #[test]
    fn test_build_tempo_info_no_tempo_events() {
        let tpb = 480;
        let track = vec![note_on(0, 0, 60, 100), end_of_track(480)];
        let timing = MidiTiming::of(&[track], tpb, Format::SingleTrack);
        assert!(timing.tempo.is_empty());
        assert_eq!(timing.ticks_per_beat, tpb);
        assert_eq!(timing.total_ticks, 480);
    }

    #[test]
    fn test_build_tempo_info_single_track_with_tempo() {
        let tpb = 480;
        let track = vec![
            tempo_event(0, 500_000),
            note_on(480, 0, 60, 100),
            end_of_track(480),
        ];
        let timing = MidiTiming::of(&[track], tpb, Format::SingleTrack);
        let (tempo_map, total_ticks) = (&timing.tempo, timing.total_ticks);
        assert_eq!(tempo_map.len(), 1);
        assert_eq!(tempo_map[0].tick, 0);
        assert_eq!(total_ticks, 960);
    }

    #[test]
    fn test_build_tempo_info_parallel_uses_conductor() {
        let tpb = 480;
        let track0 = vec![
            tempo_event(0, 1_000_000),
            tempo_event(480, 500_000),
            end_of_track(0),
        ];
        let track1 = vec![note_on(0, 0, 60, 100), end_of_track(960)];
        let timing = MidiTiming::of(&[track0, track1], tpb, Format::Parallel);
        let (tempo_map, total_ticks) = (&timing.tempo, timing.total_ticks);
        assert_eq!(tempo_map.len(), 2);
        assert_eq!(tempo_map[0].tick, 0);
        assert_eq!(tempo_map[1].tick, 480);
        // Max across tracks: track1 has 960 ticks
        assert_eq!(total_ticks, 960);
    }

    #[test]
    fn test_build_tempo_info_sequential_offsets_ticks() {
        let tpb = 480;
        let track1 = vec![tempo_event(0, 500_000), end_of_track(480)];
        let track2 = vec![tempo_event(0, 1_000_000), end_of_track(480)];
        let timing = MidiTiming::of(&[track1, track2], tpb, Format::Sequential);
        let (tempo_map, total_ticks) = (&timing.tempo, timing.total_ticks);
        assert_eq!(tempo_map.len(), 2);
        assert_eq!(tempo_map[0].tick, 0);
        // Second track's tempo change is offset by first track's total ticks
        assert_eq!(tempo_map[1].tick, 480);
        assert_eq!(total_ticks, 960);
    }

    #[test]
    fn test_build_tempo_info_empty_tracks() {
        let timing = MidiTiming::of(&[], 480, Format::Parallel);
        let (tempo_map, tpb, total_ticks) =
            (&timing.tempo, timing.ticks_per_beat, timing.total_ticks);
        assert!(tempo_map.is_empty());
        assert_eq!(tpb, 480);
        assert_eq!(total_ticks, 0);
    }
}
