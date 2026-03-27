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

//! Extract a tempo map from a MIDI file.
//!
//! MIDI files contain explicit `SetTempo` and `TimeSignature` meta events
//! that provide an authoritative tempo map. Combined with the beat grid's
//! start offset (when the first beat actually sounds in the audio), this
//! produces an accurate tempo section for lighting show authoring.

use std::path::Path;

use midly::{Format, MetaMessage, Smf, TrackEventKind};

use super::tempo_guess::{GuessedTempo, GuessedTempoChange};

/// A tempo event at a tick position.
struct TempoEvent {
    tick: u64,
    micros_per_beat: u32,
}

/// A time signature event at a tick position.
struct TimeSigEvent {
    tick: u64,
    numerator: u8,
    denominator_power: u8, // actual denominator = 2^power
}

/// Extract a tempo map from a MIDI file, using the beat grid's start offset.
///
/// Returns `None` if the MIDI file can't be parsed or has no tempo information.
pub fn extract_tempo_from_midi(
    midi_path: &Path,
    start_offset_seconds: f64,
) -> Option<GuessedTempo> {
    let data = std::fs::read(midi_path).ok()?;
    let smf = Smf::parse(&data).ok()?;

    let tpb = match smf.header.timing {
        midly::Timing::Metrical(tpb) => tpb.as_int() as u64,
        midly::Timing::Timecode(_, _) => return None, // SMPTE not supported
    };

    // Extract tempo and time signature events from the appropriate track(s)
    let (tempo_events, time_sig_events) = extract_events(&smf);

    if tempo_events.is_empty() {
        return None;
    }

    // Build the tempo map: convert tick positions to measure/beat
    let base_micros_per_beat = tempo_events[0].micros_per_beat;
    let base_bpm = (60_000_000.0 / base_micros_per_beat as f64).round() as u32;

    let base_time_sig = if let Some(ts) = time_sig_events.first() {
        if ts.tick == 0 {
            [ts.numerator as u32, 1u32 << ts.denominator_power as u32]
        } else {
            [4, 4]
        }
    } else {
        [4, 4]
    };

    let mut changes = Vec::new();
    let mut current_bpm = base_bpm;
    let mut current_ts = base_time_sig;

    // Walk through all events in tick order, tracking measure/beat position
    let mut all_events: Vec<(u64, EventKind)> = Vec::new();
    for te in &tempo_events {
        if te.tick > 0 {
            all_events.push((te.tick, EventKind::Tempo(te.micros_per_beat)));
        }
    }
    for ts in &time_sig_events {
        if ts.tick > 0 {
            all_events.push((
                ts.tick,
                EventKind::TimeSig(ts.numerator, ts.denominator_power),
            ));
        }
    }
    all_events.sort_by_key(|(tick, _)| *tick);

    // Convert ticks to measure/beat position
    let mut tick_cursor: u64 = 0;
    let mut measure: u32 = 1;
    let mut beat_in_measure: f64 = 0.0;
    let mut beats_per_measure = base_time_sig[0];
    let ticks_per_beat = tpb;

    for (tick, event) in &all_events {
        // Advance measure/beat from cursor to this tick
        let delta_ticks = tick - tick_cursor;
        let delta_beats = delta_ticks as f64 / ticks_per_beat as f64;
        beat_in_measure += delta_beats;

        // Roll over complete measures
        while beat_in_measure >= beats_per_measure as f64 {
            beat_in_measure -= beats_per_measure as f64;
            measure += 1;
        }

        tick_cursor = *tick;

        let beat_number = beat_in_measure.floor() as u32 + 1;

        match event {
            EventKind::Tempo(micros_per_beat) => {
                let bpm = (60_000_000.0 / *micros_per_beat as f64).round() as u32;
                if bpm != current_bpm {
                    changes.push(GuessedTempoChange {
                        measure,
                        beat: beat_number,
                        bpm,
                        time_signature: [beats_per_measure, current_ts[1]],
                        transition_beats: None,
                    });
                    current_bpm = bpm;
                }
            }
            EventKind::TimeSig(numerator, denom_power) => {
                let new_ts = [*numerator as u32, 1u32 << *denom_power as u32];
                if new_ts != current_ts {
                    // Emit a change with the current BPM and new time sig
                    changes.push(GuessedTempoChange {
                        measure,
                        beat: beat_number,
                        bpm: current_bpm,
                        time_signature: new_ts,
                        transition_beats: None,
                    });
                    beats_per_measure = new_ts[0];
                    current_ts = new_ts;
                }
            }
        }
    }

    // Deduplicate: if a tempo and time sig change land at the same measure/beat,
    // merge them into one change
    dedup_changes(&mut changes);

    // Collapse consecutive monotonic BPM changes into transitions (rit/accel).
    // With MIDI data this is exact — no estimation needed.
    collapse_ramps(&mut changes);

    Some(GuessedTempo {
        start_seconds: start_offset_seconds,
        bpm: base_bpm,
        time_signature: base_time_sig,
        changes,
    })
}

enum EventKind {
    Tempo(u32),
    TimeSig(u8, u8),
}

/// Extract tempo and time signature events from all relevant tracks.
fn extract_events(smf: &Smf) -> (Vec<TempoEvent>, Vec<TimeSigEvent>) {
    let mut tempo_events = Vec::new();
    let mut time_sig_events = Vec::new();

    // For Format 0 and 1, meta events are in the first track (or the only track)
    // For Format 2, we'd need to handle each track separately (rare)
    let tracks_to_scan: Vec<&[midly::TrackEvent]> = match smf.header.format {
        Format::SingleTrack => smf.tracks.iter().map(|t| t.as_slice()).collect(),
        Format::Parallel => {
            // Scan all tracks for meta events (some DAWs put them on track 0,
            // others distribute them)
            smf.tracks.iter().map(|t| t.as_slice()).collect()
        }
        Format::Sequential => smf.tracks.iter().map(|t| t.as_slice()).collect(),
    };

    for track in tracks_to_scan {
        let mut tick: u64 = 0;
        for event in track {
            tick += event.delta.as_int() as u64;
            match event.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(tempo)) => {
                    // Avoid duplicate tempo events at the same tick
                    if !tempo_events.iter().any(|e: &TempoEvent| e.tick == tick) {
                        tempo_events.push(TempoEvent {
                            tick,
                            micros_per_beat: tempo.as_int(),
                        });
                    }
                }
                TrackEventKind::Meta(MetaMessage::TimeSignature(
                    numerator,
                    denominator,
                    _clocks_per_click,
                    _thirty_seconds_per_quarter,
                )) => {
                    if !time_sig_events
                        .iter()
                        .any(|e: &TimeSigEvent| e.tick == tick)
                    {
                        time_sig_events.push(TimeSigEvent {
                            tick,
                            numerator,
                            denominator_power: denominator,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    tempo_events.sort_by_key(|e| e.tick);
    time_sig_events.sort_by_key(|e| e.tick);

    (tempo_events, time_sig_events)
}

/// Collapse consecutive monotonic BPM changes into single transitions.
/// For example, 92→82→72→62 at consecutive beats becomes a single change
/// with transition_beats spanning the full run.
fn collapse_ramps(changes: &mut Vec<GuessedTempoChange>) {
    if changes.len() < 2 {
        return;
    }

    const MAX_GAP_MEASURES: u32 = 4;

    let mut i = 0;
    while i + 1 < changes.len() {
        // Determine direction
        let bpm_i = changes[i].bpm as i32;
        let bpm_next = changes[i + 1].bpm as i32;
        let going_down = bpm_next < bpm_i;
        let going_up = bpm_next > bpm_i;

        if !going_down && !going_up {
            i += 1;
            continue;
        }

        // Must have same time signature and be close together
        if changes[i].time_signature != changes[i + 1].time_signature {
            i += 1;
            continue;
        }

        let first_gap = changes[i + 1].measure.saturating_sub(changes[i].measure);
        if first_gap > MAX_GAP_MEASURES {
            i += 1;
            continue;
        }

        // Extend the run as long as BPM keeps moving in the same direction,
        // same time signature, and changes are close together.
        let mut run_end = i + 1;
        while run_end + 1 < changes.len() {
            let prev_bpm = changes[run_end].bpm as i32;
            let next_bpm = changes[run_end + 1].bpm as i32;
            let same_dir = if going_down {
                next_bpm < prev_bpm
            } else {
                next_bpm > prev_bpm
            };
            let same_ts = changes[run_end + 1].time_signature == changes[i].time_signature;
            let measure_gap = changes[run_end + 1]
                .measure
                .saturating_sub(changes[run_end].measure);
            let close = measure_gap <= MAX_GAP_MEASURES;
            if same_dir && same_ts && close {
                run_end += 1;
            } else {
                break;
            }
        }

        let run_len = run_end - i + 1;
        if run_len < 2 {
            i += 1;
            continue;
        }

        // Compute total beat span from first change to last change.
        // Use the time signature to convert measure/beat deltas to beats.
        let first = &changes[i];
        let last = &changes[run_end];
        let bpm_ts = first.time_signature[0]; // beats per measure

        let first_beat_abs = (first.measure - 1) * bpm_ts + (first.beat - 1);
        let last_beat_abs = (last.measure - 1) * bpm_ts + (last.beat - 1);
        let transition_beats = last_beat_abs.saturating_sub(first_beat_abs);

        // Collapse: keep the first change, set BPM to the last value,
        // set transition_beats
        let final_bpm = changes[run_end].bpm;
        changes[i].bpm = final_bpm;
        changes[i].transition_beats = if transition_beats > 0 {
            Some(transition_beats)
        } else {
            None
        };

        // Remove the collapsed changes
        changes.drain((i + 1)..=run_end);
        i += 1;
    }
}

/// Merge changes at the same measure/beat into a single change.
fn dedup_changes(changes: &mut Vec<GuessedTempoChange>) {
    let mut i = 0;
    while i + 1 < changes.len() {
        if changes[i].measure == changes[i + 1].measure && changes[i].beat == changes[i + 1].beat {
            // Keep the later one's BPM and time sig (it's the most recent)
            changes[i].bpm = changes[i + 1].bpm;
            changes[i].time_signature = changes[i + 1].time_signature;
            changes.remove(i + 1);
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saxon_shore_midi() {
        let midi_path = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
            .join("src/backing-tracks/Isenmor/Saxon Shore/Saxon Shore.mid");
        if !midi_path.exists() {
            eprintln!("Skipping: MIDI not found");
            return;
        }

        let result = extract_tempo_from_midi(&midi_path, 0.0).unwrap();

        eprintln!("Base: {} BPM, {:?}", result.bpm, result.time_signature);
        for c in &result.changes {
            eprintln!(
                "  m{}/{}  {} BPM  ts={}/{}",
                c.measure, c.beat, c.bpm, c.time_signature[0], c.time_signature[1]
            );
        }

        assert_eq!(result.bpm, 150, "Base should be 150");
    }

    #[test]
    fn sigurds_song_midi() {
        let midi_path = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
            .join("src/backing-tracks/Isenmor/Sigurd's Song/Sigurd's Song.mid");
        if !midi_path.exists() {
            // Try alternate name
            let alt = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
                .join("src/backing-tracks/Isenmor/Sigurd's Song/Midi.mid");
            if !alt.exists() {
                eprintln!("Skipping: MIDI not found");
                return;
            }
            let result = extract_tempo_from_midi(&alt, 0.0).unwrap();
            eprintln!("Base: {} BPM, {:?}", result.bpm, result.time_signature);
            for c in &result.changes {
                eprintln!(
                    "  m{}/{}  {} BPM  ts={}/{}",
                    c.measure, c.beat, c.bpm, c.time_signature[0], c.time_signature[1]
                );
            }
            assert_eq!(result.bpm, 120, "Base should be 120");
            return;
        }

        let result = extract_tempo_from_midi(&midi_path, 0.0).unwrap();
        eprintln!("Base: {} BPM, {:?}", result.bpm, result.time_signature);
        for c in &result.changes {
            eprintln!(
                "  m{}/{}  {} BPM  ts={}/{}",
                c.measure, c.beat, c.bpm, c.time_signature[0], c.time_signature[1]
            );
        }
        assert_eq!(result.bpm, 120, "Base should be 120");
    }

    #[test]
    fn operation_orcinianus_copia_midi() {
        let midi_path = std::path::Path::new(&std::env::var("HOME").unwrap_or_default()).join(
            "src/backing-tracks/Recently Vacated Graves/Operation Orcinianus Copia/automation.mid",
        );
        if !midi_path.exists() {
            eprintln!("Skipping: MIDI not found");
            return;
        }

        let result = extract_tempo_from_midi(&midi_path, 0.0).unwrap();

        eprintln!("Base: {} BPM, {:?}", result.bpm, result.time_signature);
        for c in &result.changes {
            eprintln!(
                "  m{}/{}  {} BPM  ts={}/{}  transition={:?}",
                c.measure,
                c.beat,
                c.bpm,
                c.time_signature[0],
                c.time_signature[1],
                c.transition_beats
            );
        }

        // Should detect the rit at measure 25 as a transition, not discrete steps
        let rit = result
            .changes
            .iter()
            .find(|c| c.measure >= 24 && c.measure <= 26 && c.transition_beats.is_some());
        assert!(
            rit.is_some(),
            "Expected a rit transition near measure 25, got: {:?}",
            result.changes
        );
    }
}
