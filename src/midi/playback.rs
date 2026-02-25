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
struct TempoEntry {
    tick: u64,
    micros_per_tick: f64,
}

/// Result of processing a single track: MIDI events and total track duration.
struct TrackResult {
    events: Vec<TimedMidiEvent>,
    /// Total duration of the track (including trailing silence after last MIDI event).
    total_duration: Duration,
}

impl PrecomputedMidi {
    /// Builds a pre-computed MIDI timeline from parsed tracks.
    ///
    /// Single-pass algorithm: for each track, accumulates tick positions and
    /// converts to absolute time using tempo changes. For Format 1 (parallel),
    /// the conductor track (track 0) provides the tempo map for all tracks.
    pub fn from_tracks(
        tracks: &[Vec<TrackEvent<'_>>],
        ticks_per_beat: u16,
        format: Format,
    ) -> Self {
        let tpb = ticks_per_beat as f64;
        // Default tempo: 120 BPM = 500_000 microseconds per beat
        let default_micros_per_tick = 500_000.0 / tpb;

        match format {
            Format::SingleTrack => {
                let events = if let Some(track) = tracks.first() {
                    Self::process_track(track, default_micros_per_tick, tpb, &[]).events
                } else {
                    Vec::new()
                };
                PrecomputedMidi { events }
            }
            Format::Parallel => {
                // Extract tempo map from conductor track (track 0)
                let tempo_map = tracks
                    .first()
                    .map(|track| Self::extract_tempo_map(track, tpb))
                    .unwrap_or_default();

                // Skip track 0 (conductor) — it provides the tempo map but should
                // not emit MIDI events. Non-conformant files with MIDI events on
                // track 0 would otherwise get double-tempo-mapped.
                let mut all_events = Vec::new();
                for track in tracks.iter().skip(1) {
                    let mut track_events =
                        Self::process_track(track, default_micros_per_tick, tpb, &tempo_map).events;
                    all_events.append(&mut track_events);
                }
                all_events.sort_by(|a, b| a.time.cmp(&b.time));
                PrecomputedMidi { events: all_events }
            }
            Format::Sequential => {
                let mut all_events = Vec::new();
                let mut cumulative_offset = Duration::ZERO;
                for track in tracks {
                    let result = Self::process_track(track, default_micros_per_tick, tpb, &[]);
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

    /// Extracts a tempo map from a track (tick position → micros_per_tick).
    fn extract_tempo_map(track: &[TrackEvent<'_>], tpb: f64) -> Vec<TempoEntry> {
        let mut tempo_map = Vec::new();
        let mut tick_position: u64 = 0;
        for event in track {
            tick_position += event.delta.as_int() as u64;
            if let TrackEventKind::Meta(MetaMessage::Tempo(tempo)) = event.kind {
                tempo_map.push(TempoEntry {
                    tick: tick_position,
                    micros_per_tick: tempo.as_int() as f64 / tpb,
                });
            }
        }
        tempo_map
    }

    /// Processes a single track into timed events, using an optional external tempo map.
    /// Returns both the MIDI events and the total track duration (including trailing silence).
    fn process_track(
        track: &[TrackEvent<'_>],
        default_micros_per_tick: f64,
        tpb: f64,
        external_tempo_map: &[TempoEntry],
    ) -> TrackResult {
        let mut events = Vec::new();
        let mut tick_position: u64 = 0;
        let mut elapsed_micros: f64 = 0.0;
        let mut last_tick: u64 = 0;
        let mut micros_per_tick = default_micros_per_tick;

        // Index into external tempo map for efficient traversal
        let mut tempo_idx: usize = 0;

        for event in track {
            let delta = event.delta.as_int() as u64;
            tick_position += delta;

            // Apply any tempo changes from the external map that fall within this delta
            if !external_tempo_map.is_empty() {
                let mut remaining_ticks = tick_position - last_tick;
                let mut cursor = last_tick;
                while tempo_idx < external_tempo_map.len()
                    && external_tempo_map[tempo_idx].tick <= tick_position
                {
                    let entry = &external_tempo_map[tempo_idx];
                    if entry.tick > cursor {
                        let ticks_at_old_tempo = entry.tick - cursor;
                        elapsed_micros += ticks_at_old_tempo as f64 * micros_per_tick;
                        remaining_ticks -= ticks_at_old_tempo;
                        cursor = entry.tick;
                    }
                    micros_per_tick = entry.micros_per_tick;
                    tempo_idx += 1;
                }
                // Remaining ticks at current tempo
                elapsed_micros += remaining_ticks as f64 * micros_per_tick;
            } else {
                // No external tempo map — handle inline tempo changes
                // For single-track (Format 0) or sequential tracks
                let ticks_since_last = tick_position - last_tick;
                elapsed_micros += ticks_since_last as f64 * micros_per_tick;

                // Check for inline tempo change
                if let TrackEventKind::Meta(MetaMessage::Tempo(tempo)) = event.kind {
                    micros_per_tick = tempo.as_int() as f64 / tpb;
                }
            }

            last_tick = tick_position;

            // Emit MIDI events (skip meta events)
            if let TrackEventKind::Midi { channel, message } = event.kind {
                events.push(TimedMidiEvent {
                    time: Duration::from_micros(elapsed_micros as u64),
                    channel: channel.as_int(),
                    message,
                });
            }
        }

        TrackResult {
            events,
            total_duration: Duration::from_micros(elapsed_micros as u64),
        }
    }

    /// Creates a PrecomputedMidi from a slice of events (cloning them).
    pub fn from_events(events: &[TimedMidiEvent]) -> Self {
        PrecomputedMidi {
            events: events.to_vec(),
        }
    }

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
    #[allow(dead_code)]
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
        let midi = PrecomputedMidi::from_tracks(&[], 480, Format::Parallel);
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
        let midi = PrecomputedMidi::from_tracks(&[track], tpb, Format::SingleTrack);

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

        let midi = PrecomputedMidi::from_tracks(&[track0, track1, track2], tpb, Format::Parallel);

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

        let midi = PrecomputedMidi::from_tracks(&[track], tpb, Format::SingleTrack);

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

        let midi = PrecomputedMidi::from_tracks(&[track0, track1], tpb, Format::Parallel);

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
        let midi = PrecomputedMidi::from_tracks(&[track], tpb, Format::SingleTrack);

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
        let midi = PrecomputedMidi::from_tracks(&[track], tpb, Format::SingleTrack);

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

        let midi = PrecomputedMidi::from_tracks(&[track1, track2], tpb, Format::Sequential);

        assert_eq!(midi.len(), 3);
        // Track 1 events
        assert_eq!(midi.events()[0].time, Duration::from_micros(0));
        assert_eq!(midi.events()[1].time, Duration::from_micros(500_000));
        // Track 2 starts at beat 2 (1_000_000 µs) — includes the trailing silence
        assert_eq!(midi.events()[2].time, Duration::from_micros(1_000_000));
        assert_eq!(midi.events()[2].channel, 1);
    }
}
