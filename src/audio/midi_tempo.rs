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
//! that provide an authoritative tempo map. When a beat grid is available
//! (from click track analysis), the MIDI is cross-correlated against it
//! to auto-detect the correct alignment offset. This handles both MIDI
//! files with and without lead-in silence.

use std::path::Path;

use midly::{Format, MetaMessage, Smf, TrackEventKind};

use super::click_analysis::BeatGrid;
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

/// Extract a tempo map from a MIDI file.
///
/// When a beat grid is provided, cross-correlates the MIDI's predicted beat
/// positions against the audio beat grid to find the optimal alignment offset.
/// This handles MIDI files both with and without lead-in silence.
///
/// When no beat grid is available, assumes offset 0.0 (tick 0 = beat 1).
///
/// Returns `None` if the MIDI file can't be parsed or has no tempo information.
pub fn extract_tempo_from_midi(
    midi_path: &Path,
    beat_grid: Option<&BeatGrid>,
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

    // Determine the start offset by cross-correlating with the beat grid,
    // or fall back to 0.0 if no beat grid is available.
    let start_offset_seconds = match beat_grid {
        Some(grid) if grid.beats.len() >= 4 => {
            find_best_offset(&tempo_events, &time_sig_events, tpb, grid)
        }
        Some(grid) => grid.beats.first().copied().unwrap_or(0.0),
        None => 0.0,
    };

    // Convert start_offset_seconds to a tick position by walking through
    // tempo events. The MIDI file may include a lead-in before beat 1
    // (tick 0 = audio file start, not necessarily beat 1).
    let offset_tick = seconds_to_tick(start_offset_seconds, &tempo_events, tpb);

    // Find the tempo and time signature active at the offset tick.
    // These become the base values — they represent what the song starts at.
    let base_micros_per_beat = tempo_events
        .iter()
        .rev()
        .find(|e| e.tick <= offset_tick)
        .map(|e| e.micros_per_beat)
        .unwrap_or(tempo_events[0].micros_per_beat);
    let base_bpm = (60_000_000.0 / base_micros_per_beat as f64).round() as u32;

    let base_time_sig = time_sig_events
        .iter()
        .rev()
        .find(|e| e.tick <= offset_tick)
        .map(|e| [e.numerator as u32, 1u32 << e.denominator_power as u32])
        .unwrap_or([4, 4]);

    let mut changes = Vec::new();
    let mut current_bpm = base_bpm;
    let mut current_ts = base_time_sig;

    // Collect all events that occur AFTER the offset tick.
    let mut all_events: Vec<(u64, EventKind)> = Vec::new();
    for te in &tempo_events {
        if te.tick > offset_tick {
            all_events.push((te.tick, EventKind::Tempo(te.micros_per_beat)));
        }
    }
    for ts in &time_sig_events {
        if ts.tick > offset_tick {
            all_events.push((
                ts.tick,
                EventKind::TimeSig(ts.numerator, ts.denominator_power),
            ));
        }
    }
    all_events.sort_by_key(|(tick, _)| *tick);

    // Convert ticks to measure/beat position, counting from the offset tick.
    let mut tick_cursor: u64 = offset_tick;
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

    // Compute alignment quality: RMS error (ms) between MIDI-predicted beats
    // and the click-track beat grid. Only available when a beat grid was used.
    let alignment_rms_ms = beat_grid.and_then(|grid| {
        if grid.beats.len() < 4 {
            return None;
        }
        let max_time = grid.beats.last().copied().unwrap_or(0.0);
        let midi_beats = midi_beat_times(
            &tempo_events,
            &time_sig_events,
            tpb,
            start_offset_seconds,
            max_time,
        );
        let score = alignment_score(&midi_beats, &grid.beats);
        if score == f64::NEG_INFINITY {
            return None;
        }
        // score = -MSE in seconds²; convert to RMSE in milliseconds.
        Some((-score).sqrt() * 1000.0)
    });

    Some(GuessedTempo {
        start_seconds: start_offset_seconds,
        bpm: base_bpm,
        time_signature: base_time_sig,
        changes,
        alignment_rms_ms,
    })
}

/// Converts a time in seconds to a MIDI tick position by walking through
/// tempo events. Handles tempo changes during the lead-in period.
fn seconds_to_tick(target_seconds: f64, tempo_events: &[TempoEvent], tpb: u64) -> u64 {
    if target_seconds <= 0.0 {
        return 0;
    }

    let mut elapsed_seconds = 0.0;
    let mut current_tick: u64 = 0;
    let mut current_micros_per_beat = tempo_events[0].micros_per_beat;

    for te in tempo_events.iter().skip(1) {
        let delta_ticks = te.tick - current_tick;
        let seconds_per_tick = current_micros_per_beat as f64 / 1_000_000.0 / tpb as f64;
        let delta_seconds = delta_ticks as f64 * seconds_per_tick;

        if elapsed_seconds + delta_seconds >= target_seconds {
            // The target falls within this tempo segment.
            let remaining = target_seconds - elapsed_seconds;
            return current_tick + (remaining / seconds_per_tick).round() as u64;
        }

        elapsed_seconds += delta_seconds;
        current_tick = te.tick;
        current_micros_per_beat = te.micros_per_beat;
    }

    // Target is past the last tempo event — extrapolate.
    let remaining = target_seconds - elapsed_seconds;
    let seconds_per_tick = current_micros_per_beat as f64 / 1_000_000.0 / tpb as f64;
    current_tick + (remaining / seconds_per_tick).round() as u64
}

/// Converts a MIDI tick position to absolute seconds by walking through
/// tempo events. Inverse of `seconds_to_tick()`.
fn tick_to_seconds(target_tick: u64, tempo_events: &[TempoEvent], tpb: u64) -> f64 {
    if target_tick == 0 {
        return 0.0;
    }

    let mut elapsed_seconds = 0.0;
    let mut current_tick: u64 = 0;
    let mut current_micros_per_beat = tempo_events[0].micros_per_beat;

    for te in tempo_events.iter().skip(1) {
        if te.tick >= target_tick {
            break;
        }
        let delta_ticks = te.tick - current_tick;
        let seconds_per_tick = current_micros_per_beat as f64 / 1_000_000.0 / tpb as f64;
        elapsed_seconds += delta_ticks as f64 * seconds_per_tick;
        current_tick = te.tick;
        current_micros_per_beat = te.micros_per_beat;
    }

    let remaining_ticks = target_tick - current_tick;
    let seconds_per_tick = current_micros_per_beat as f64 / 1_000_000.0 / tpb as f64;
    elapsed_seconds + remaining_ticks as f64 * seconds_per_tick
}

/// Returns the tick step (pulse width) for one click-track beat given a time
/// signature. The click track marks the natural pulse of each meter:
///
/// - Simple meters (denominator 4): quarter-note pulse → `tpb` ticks
/// - Simple meters (denominator 2): half-note pulse → `2 * tpb` ticks
/// - Compound meters (denominator 8, numerator divisible by 3 — 6/8, 9/8, 12/8):
///   dotted-quarter pulse → `3 * tpb / 2` ticks
/// - Other denominator-8 meters (5/8, 7/8, …): eighth-note pulse → `tpb / 2` ticks
/// - Denominator 16: sixteenth-note pulse → `tpb / 4` ticks
///
/// Falls back to `tpb` (quarter note) for any unrecognised denominator.
fn beat_step_ticks(numerator: u8, denominator_power: u8, tpb: u64) -> u64 {
    let denominator = 1u32 << denominator_power;
    match denominator {
        2 => 2 * tpb,
        4 => tpb,
        8 if numerator.is_multiple_of(3) => 3 * tpb / 2, // compound: dotted-quarter
        8 => tpb / 2,                                    // simple eighth-note pulse
        16 => tpb / 4,
        _ => tpb,
    }
}

/// Generates expected beat positions (in seconds) from the MIDI tempo map.
///
/// Steps through ticks using the natural pulse of the current time signature
/// (quarter notes for simple meter, dotted quarters for compound meter, etc.),
/// so the predicted beat positions match what a typical click track marks.
/// Stops when the generated time exceeds `max_seconds`.
fn midi_beat_times(
    tempo_events: &[TempoEvent],
    time_sig_events: &[TimeSigEvent],
    tpb: u64,
    start_seconds: f64,
    max_seconds: f64,
) -> Vec<f64> {
    let start_tick = seconds_to_tick(start_seconds, tempo_events, tpb);
    let mut beats = Vec::new();
    let mut tick = start_tick;

    loop {
        let time = tick_to_seconds(tick, tempo_events, tpb);
        if time > max_seconds {
            break;
        }
        beats.push(time);

        // Look up the time signature active at this tick.
        let (num, denom_pow) = time_sig_events
            .iter()
            .rev()
            .find(|e| e.tick <= tick)
            .map(|e| (e.numerator, e.denominator_power))
            .unwrap_or((4, 2)); // default: 4/4

        tick += beat_step_ticks(num, denom_pow, tpb);
    }

    beats
}

/// Scores how well MIDI-predicted beats align with actual beat grid beats.
///
/// For each grid beat that falls within the MIDI beat range, binary-searches
/// for the nearest MIDI beat and accumulates squared error. Returns negative
/// mean squared error (higher = better fit).
fn alignment_score(midi_beats: &[f64], grid_beats: &[f64]) -> f64 {
    if midi_beats.is_empty() || grid_beats.is_empty() {
        return f64::NEG_INFINITY;
    }

    let midi_min = midi_beats[0];
    let midi_max = midi_beats[midi_beats.len() - 1];
    let mut sum_sq = 0.0;
    let mut count = 0u32;

    for &gb in grid_beats {
        if gb < midi_min || gb > midi_max {
            continue;
        }

        // Binary search for the nearest MIDI beat.
        let idx = midi_beats.partition_point(|&mb| mb < gb);
        let mut best_dist = f64::MAX;
        if idx < midi_beats.len() {
            best_dist = best_dist.min((midi_beats[idx] - gb).abs());
        }
        if idx > 0 {
            best_dist = best_dist.min((midi_beats[idx - 1] - gb).abs());
        }

        sum_sq += best_dist * best_dist;
        count += 1;
    }

    if count == 0 {
        return f64::NEG_INFINITY;
    }

    -(sum_sq / count as f64)
}

/// Maximum lead-in duration to search (seconds).
const MAX_LEADIN_SECONDS: f64 = 10.0;

/// Finds the start_seconds value (when beat 1 occurs in audio time) that
/// best aligns MIDI-predicted beats with the beat grid.
///
/// Uses a two-phase search: coarse candidates from beat grid positions in
/// the first 10 seconds, then fine refinement around the best candidate.
fn find_best_offset(
    tempo_events: &[TempoEvent],
    time_sig_events: &[TimeSigEvent],
    tpb: u64,
    grid: &BeatGrid,
) -> f64 {
    let max_time = grid.beats.last().copied().unwrap_or(0.0);

    // Coarse phase: each beat grid beat in the first ~10s is a candidate
    // for "this is where beat 1 falls in the audio".
    let mut candidates: Vec<f64> = Vec::new();
    for &beat_time in &grid.beats {
        if beat_time > MAX_LEADIN_SECONDS {
            break;
        }
        candidates.push(beat_time);
    }
    // Also try 0.0 in case beat 1 is at the very start.
    if candidates.first().is_none_or(|&t| t > 0.001) {
        candidates.insert(0, 0.0);
    }

    // Prefer the candidate closest to the grid's first beat when scores are
    // within epsilon of each other. This prevents a monotonically constant-tempo
    // lead-in from drifting toward later offsets due to marginal MSE differences
    // (excluding early beats from the comparison window always looks slightly
    // better when those beats have any jitter).
    let grid_first = grid.beats[0];
    // Epsilon: ~7ms average per-beat error. Differences smaller than this are
    // treated as ties and resolved by proximity to the grid's first beat.
    let score_epsilon = 5e-5_f64;

    let mut best_score = f64::NEG_INFINITY;
    let mut best_start = grid_first;

    for &candidate in &candidates {
        let midi_beats = midi_beat_times(tempo_events, time_sig_events, tpb, candidate, max_time);
        let score = alignment_score(&midi_beats, &grid.beats);
        let closer = (candidate - grid_first).abs() < (best_start - grid_first).abs();
        let meaningfully_better = score > best_score + score_epsilon;
        let tied_and_closer = score >= best_score - score_epsilon && closer;
        if meaningfully_better || tied_and_closer {
            best_score = score;
            best_start = candidate;
        }
    }

    // Fine phase: refine around the best candidate in 1ms steps over
    // +/- one beat duration at the initial tempo.
    let beat_duration = tempo_events[0].micros_per_beat as f64 / 1_000_000.0;
    let fine_start = (best_start - beat_duration).max(0.0);
    let fine_end = best_start + beat_duration;
    let step = 0.001; // 1ms

    let mut t = fine_start;
    while t <= fine_end {
        let midi_beats = midi_beat_times(tempo_events, time_sig_events, tpb, t, max_time);
        let score = alignment_score(&midi_beats, &grid.beats);
        let closer = (t - grid_first).abs() < (best_start - grid_first).abs();
        let meaningfully_better = score > best_score + score_epsilon;
        let tied_and_closer = score >= best_score - score_epsilon && closer;
        if meaningfully_better || tied_and_closer {
            best_score = score;
            best_start = t;
        }
        t += step;
    }

    best_start
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
    use std::io::Write;

    /// Helper: builds a minimal Format 0 MIDI file with given tempo/time-sig events.
    /// Returns the raw bytes. Uses 480 ticks per beat.
    fn build_midi(events: &[(u32, MidiMetaEvent)]) -> Vec<u8> {
        let tpb: u16 = 480;
        let mut track_data = Vec::new();

        let mut last_tick: u32 = 0;
        for (tick, event) in events {
            let delta = tick - last_tick;
            write_vlq(&mut track_data, delta);
            match event {
                MidiMetaEvent::Tempo(micros) => {
                    track_data.extend_from_slice(&[0xFF, 0x51, 0x03]);
                    track_data.push((micros >> 16) as u8);
                    track_data.push((micros >> 8) as u8);
                    track_data.push(*micros as u8);
                }
                MidiMetaEvent::TimeSig(num, denom_pow) => {
                    track_data.extend_from_slice(&[0xFF, 0x58, 0x04]);
                    track_data.push(*num);
                    track_data.push(*denom_pow);
                    track_data.push(24); // clocks per click
                    track_data.push(8); // 32nds per quarter
                }
            }
            last_tick = *tick;
        }
        // End of track
        write_vlq(&mut track_data, 0);
        track_data.extend_from_slice(&[0xFF, 0x2F, 0x00]);

        let track_len = track_data.len() as u32;

        let mut midi = Vec::new();
        // Header chunk
        midi.extend_from_slice(b"MThd");
        midi.extend_from_slice(&6u32.to_be_bytes()); // chunk length
        midi.extend_from_slice(&0u16.to_be_bytes()); // format 0
        midi.extend_from_slice(&1u16.to_be_bytes()); // 1 track
        midi.extend_from_slice(&tpb.to_be_bytes());
        // Track chunk
        midi.extend_from_slice(b"MTrk");
        midi.extend_from_slice(&track_len.to_be_bytes());
        midi.extend_from_slice(&track_data);

        midi
    }

    enum MidiMetaEvent {
        Tempo(u32),      // microseconds per beat
        TimeSig(u8, u8), // numerator, denominator power
    }

    fn bpm_to_micros(bpm: u32) -> u32 {
        60_000_000 / bpm
    }

    fn write_vlq(buf: &mut Vec<u8>, mut value: u32) {
        if value == 0 {
            buf.push(0);
            return;
        }
        let mut bytes = Vec::new();
        while value > 0 {
            bytes.push((value & 0x7F) as u8);
            value >>= 7;
        }
        bytes.reverse();
        for (i, b) in bytes.iter().enumerate() {
            if i < bytes.len() - 1 {
                buf.push(b | 0x80);
            } else {
                buf.push(*b);
            }
        }
    }

    fn write_test_midi(events: &[(u32, MidiMetaEvent)]) -> tempfile::NamedTempFile {
        let data = build_midi(events);
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&data).unwrap();
        f.flush().unwrap();
        f
    }

    /// Build a synthetic beat grid with beats at the given BPM starting at `start`.
    fn make_beat_grid(
        bpm: f64,
        start: f64,
        num_beats: usize,
        beats_per_measure: usize,
    ) -> BeatGrid {
        let interval = 60.0 / bpm;
        let beats: Vec<f64> = (0..num_beats)
            .map(|i| start + i as f64 * interval)
            .collect();
        let measure_starts: Vec<usize> = (0..num_beats).step_by(beats_per_measure).collect();
        BeatGrid {
            beats,
            measure_starts,
        }
    }

    // ── alignment_rms_ms tests ────────────────────────────────────────────────

    #[test]
    fn alignment_rms_ms_none_without_beat_grid() {
        let f = write_test_midi(&[(0, MidiMetaEvent::Tempo(bpm_to_micros(120)))]);
        let result = extract_tempo_from_midi(f.path(), None).unwrap();
        assert!(
            result.alignment_rms_ms.is_none(),
            "alignment_rms_ms should be None when no beat grid provided"
        );
    }

    #[test]
    fn alignment_rms_ms_low_for_well_matched_grid() {
        // MIDI at 120 BPM, beat grid at exactly 120 BPM — should be near-zero.
        let f = write_test_midi(&[(0, MidiMetaEvent::Tempo(bpm_to_micros(120)))]);
        let grid = make_beat_grid(120.0, 0.0, 64, 4);
        let result = extract_tempo_from_midi(f.path(), Some(&grid)).unwrap();
        let rms = result
            .alignment_rms_ms
            .expect("alignment_rms_ms should be Some");
        assert!(
            rms < 2.0,
            "RMS should be near-zero for a perfectly matched grid, got {rms:.3}ms"
        );
    }

    #[test]
    fn alignment_rms_ms_high_for_mismatched_grid() {
        // MIDI at 120 BPM but beat grid at 90 BPM — wildly mismatched.
        let f = write_test_midi(&[(0, MidiMetaEvent::Tempo(bpm_to_micros(120)))]);
        let grid = make_beat_grid(90.0, 0.0, 48, 4);
        let result = extract_tempo_from_midi(f.path(), Some(&grid)).unwrap();
        let rms = result
            .alignment_rms_ms
            .expect("alignment_rms_ms should be Some");
        assert!(
            rms > 15.0,
            "RMS should exceed warning threshold for mismatched BPM, got {rms:.3}ms"
        );
    }

    #[test]
    fn no_beat_grid_uses_zero_offset() {
        // MIDI starts at 120 BPM, no beat grid provided.
        let f = write_test_midi(&[(0, MidiMetaEvent::Tempo(bpm_to_micros(120)))]);
        let result = extract_tempo_from_midi(f.path(), None).unwrap();
        assert_eq!(result.bpm, 120);
        assert!(result.changes.is_empty());
    }

    #[test]
    fn beat_grid_auto_detects_leadin_offset() {
        // MIDI has 100 BPM during lead-in, then 140 BPM at tick 1600 (~2s).
        // Beat grid has beats at 140 BPM starting at 2.0s.
        let offset_ticks = 1600;
        let f = write_test_midi(&[
            (0, MidiMetaEvent::Tempo(bpm_to_micros(100))),
            (offset_ticks, MidiMetaEvent::Tempo(bpm_to_micros(140))),
        ]);

        let grid = make_beat_grid(140.0, 2.0, 32, 4);
        let result = extract_tempo_from_midi(f.path(), Some(&grid)).unwrap();

        // Should auto-detect that beat 1 is at ~2.0s and use 140 BPM as base.
        assert_eq!(result.bpm, 140);
        assert!(result.changes.is_empty(), "No changes after beat 1");
        assert!(
            (result.start_seconds - 2.0).abs() < 0.01,
            "Offset should be ~2.0s, got {}",
            result.start_seconds
        );
    }

    #[test]
    fn beat_grid_detects_no_leadin() {
        // MIDI at 120 BPM, beat grid starts at 0.0s. No lead-in.
        let f = write_test_midi(&[(0, MidiMetaEvent::Tempo(bpm_to_micros(120)))]);
        let grid = make_beat_grid(120.0, 0.0, 32, 4);
        let result = extract_tempo_from_midi(f.path(), Some(&grid)).unwrap();

        assert_eq!(result.bpm, 120);
        assert!(result.start_seconds.abs() < 0.01, "Offset should be ~0.0s");
    }

    #[test]
    fn beat_grid_with_tempo_change_after_start() {
        // MIDI: 120 BPM for lead-in + 4 measures, then 150 BPM.
        // Beat grid: 120 BPM beats starting at 1.0s (2 beat lead-in).
        let offset_ticks = 960; // 2 beats at 120 BPM = 1.0s
        let change_ticks = offset_ticks + 16 * 480; // 4 measures after start
        let f = write_test_midi(&[
            (0, MidiMetaEvent::Tempo(bpm_to_micros(120))),
            (change_ticks, MidiMetaEvent::Tempo(bpm_to_micros(150))),
        ]);

        // Build beat grid: 16 beats at 120 BPM, then beats at 150 BPM.
        let mut beats = Vec::new();
        let mut measure_starts = Vec::new();
        let start = 1.0;
        for i in 0..16 {
            if i % 4 == 0 {
                measure_starts.push(beats.len());
            }
            beats.push(start + i as f64 * 0.5); // 120 BPM = 0.5s/beat
        }
        for i in 0..16 {
            if i % 4 == 0 {
                measure_starts.push(beats.len());
            }
            beats.push(start + 16.0 * 0.5 + i as f64 * 0.4); // 150 BPM = 0.4s/beat
        }
        let grid = BeatGrid {
            beats,
            measure_starts,
        };

        let result = extract_tempo_from_midi(f.path(), Some(&grid)).unwrap();

        assert_eq!(result.bpm, 120);
        assert!((result.start_seconds - 1.0).abs() < 0.01);
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].measure, 5);
        assert_eq!(result.changes[0].bpm, 150);
    }

    #[test]
    fn beat_grid_with_time_sig_at_start() {
        // Lead-in in 4/4, music in 3/4 at 120 BPM starting at 2.0s.
        let f = write_test_midi(&[
            (0, MidiMetaEvent::Tempo(bpm_to_micros(120))),
            (0, MidiMetaEvent::TimeSig(4, 2)),    // 4/4
            (1920, MidiMetaEvent::TimeSig(3, 2)), // 3/4 at tick 1920 = 2.0s
        ]);

        let grid = make_beat_grid(120.0, 2.0, 24, 3);
        let result = extract_tempo_from_midi(f.path(), Some(&grid)).unwrap();

        assert_eq!(result.bpm, 120);
        assert_eq!(result.time_signature, [3, 4]);
        assert!((result.start_seconds - 2.0).abs() < 0.01);
    }

    // --- tick/seconds conversion tests ---

    #[test]
    fn seconds_to_tick_zero_offset() {
        let events = vec![TempoEvent {
            tick: 0,
            micros_per_beat: bpm_to_micros(120),
        }];
        assert_eq!(seconds_to_tick(0.0, &events, 480), 0);
    }

    #[test]
    fn seconds_to_tick_simple() {
        // 120 BPM, 480 tpb: 1 beat = 0.5s, 1 tick = 0.5/480 s
        // 2.0s = 4 beats = 1920 ticks
        let events = vec![TempoEvent {
            tick: 0,
            micros_per_beat: bpm_to_micros(120),
        }];
        assert_eq!(seconds_to_tick(2.0, &events, 480), 1920);
    }

    #[test]
    fn seconds_to_tick_with_tempo_change() {
        // 120 BPM for first 1s (960 ticks), then 60 BPM.
        // At 60 BPM: 1 beat = 1.0s, so 0.5s more = 0.5 beats = 240 ticks.
        // Total for 1.5s = 960 + 240 = 1200 ticks.
        let events = vec![
            TempoEvent {
                tick: 0,
                micros_per_beat: bpm_to_micros(120),
            },
            TempoEvent {
                tick: 960,
                micros_per_beat: bpm_to_micros(60),
            },
        ];
        assert_eq!(seconds_to_tick(1.5, &events, 480), 1200);
    }

    #[test]
    fn saxon_shore_midi() {
        let midi_path = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
            .join("src/backing-tracks/Isenmor/Saxon Shore/Saxon Shore.mid");
        if !midi_path.exists() {
            eprintln!("Skipping: MIDI not found");
            return;
        }

        let result = extract_tempo_from_midi(&midi_path, None).unwrap();

        eprintln!("Base: {} BPM, {:?}", result.bpm, result.time_signature);
        for c in &result.changes {
            eprintln!(
                "  m{}/{}  {} BPM  ts={}/{}",
                c.measure, c.beat, c.bpm, c.time_signature[0], c.time_signature[1]
            );
        }

        assert_eq!(result.bpm, 150, "Base should be 150");
    }

    /// Evaluates MIDI beat alignment against a real click track.
    ///
    /// Loads the beat grid from the click track, runs `extract_tempo_from_midi`
    /// with the cross-correlation, then generates MIDI-predicted beat times
    /// and measures how well they line up against the click-detected beats.
    ///
    /// Prints per-beat error, summary statistics (mean error, RMSE, max error),
    /// and histograms by error magnitude bucket.
    fn eval_midi_vs_click(
        song_dir: &str,
        midi_filename: &str,
        click_filename: &str,
        click_channel: u16,
    ) {
        let base = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
            .join("src/backing-tracks")
            .join(song_dir);

        let midi_path = base.join(midi_filename);
        let click_path = base.join(click_filename);

        if !midi_path.exists() || !click_path.exists() {
            eprintln!("Skipping {song_dir}: files not found");
            return;
        }

        // Step 1: Build the beat grid from the click track.
        let grid =
            crate::audio::click_analysis::analyze_click_track_default(&click_path, click_channel)
                .expect("click analysis failed");
        eprintln!("\n=== {song_dir} ===");
        eprintln!(
            "Click grid: {} beats, {} measures",
            grid.beat_count(),
            grid.measure_count()
        );
        if let (Some(&first), Some(&last)) = (grid.beats.first(), grid.beats.last()) {
            eprintln!(
                "  Span: {first:.3}s – {last:.3}s  (duration {:.1}s)",
                last - first
            );
        }

        // Step 2: Extract MIDI tempo WITH the beat grid (cross-correlation).
        let result =
            extract_tempo_from_midi(&midi_path, Some(&grid)).expect("MIDI extraction failed");
        eprintln!(
            "MIDI result: {} BPM, ts={}/{}, start_offset={:.4}s",
            result.bpm, result.time_signature[0], result.time_signature[1], result.start_seconds
        );
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

        // Step 3: Regenerate MIDI beat times using the found offset and compute errors.
        let data = std::fs::read(&midi_path).unwrap();
        let smf = midly::Smf::parse(&data).unwrap();
        let tpb = match smf.header.timing {
            midly::Timing::Metrical(tpb) => tpb.as_int() as u64,
            _ => panic!("SMPTE timing not supported"),
        };
        let (tempo_events, time_sig_events) = extract_events(&smf);
        let max_time = grid.beats.last().copied().unwrap_or(0.0) + 1.0;
        let midi_beats = midi_beat_times(
            &tempo_events,
            &time_sig_events,
            tpb,
            result.start_seconds,
            max_time,
        );

        eprintln!(
            "MIDI predicted {} beats vs {} click beats",
            midi_beats.len(),
            grid.beats.len()
        );

        // For each MIDI beat (in order), find the nearest click beat and record
        // the signed drift (positive = MIDI is ahead of click).
        let click_min = grid.beats.first().copied().unwrap_or(0.0);
        let click_max = grid.beats.last().copied().unwrap_or(0.0);
        let mut errors_ms: Vec<f64> = Vec::new();

        // Also track signed drift per beat to reveal systematic offset.
        let mut drift_series: Vec<(f64, f64)> = Vec::new(); // (midi_time, signed_drift_ms)

        for &mb in &midi_beats {
            if mb < click_min || mb > click_max {
                continue;
            }
            let idx = grid.beats.partition_point(|&cb| cb < mb);
            let mut best_dist = f64::MAX;
            let mut best_signed = 0.0f64;
            if idx < grid.beats.len() {
                let d = (mb - grid.beats[idx]).abs();
                if d < best_dist {
                    best_dist = d;
                    best_signed = mb - grid.beats[idx];
                }
            }
            if idx > 0 {
                let d = (mb - grid.beats[idx - 1]).abs();
                if d < best_dist {
                    best_dist = d;
                    best_signed = mb - grid.beats[idx - 1];
                }
            }
            errors_ms.push(best_dist * 1000.0);
            drift_series.push((mb, best_signed * 1000.0));
        }

        if errors_ms.is_empty() {
            eprintln!("No overlap between MIDI beats and click grid — cannot evaluate.");
            return;
        }

        let mean_err = errors_ms.iter().sum::<f64>() / errors_ms.len() as f64;
        let rmse = (errors_ms.iter().map(|e| e * e).sum::<f64>() / errors_ms.len() as f64).sqrt();
        let max_err = errors_ms.iter().cloned().fold(0.0f64, f64::max);
        let median = {
            let mut s = errors_ms.clone();
            s.sort_by(|a, b| a.partial_cmp(b).unwrap());
            s[s.len() / 2]
        };

        eprintln!("\nAlignment errors ({} beats evaluated):", errors_ms.len());
        eprintln!("  Mean:   {mean_err:.2}ms");
        eprintln!("  Median: {median:.2}ms");
        eprintln!("  RMSE:   {rmse:.2}ms");
        eprintln!("  Max:    {max_err:.2}ms");

        // Histogram by error bucket.
        let buckets = [1.0, 2.0, 5.0, 10.0, 20.0, f64::INFINITY];
        let labels = ["<1ms", "<2ms", "<5ms", "<10ms", "<20ms", "≥20ms"];
        let total = errors_ms.len() as f64;
        eprintln!("\n  Distribution:");
        let mut prev = 0.0f64;
        for (threshold, label) in buckets.iter().zip(labels.iter()) {
            let count = errors_ms
                .iter()
                .filter(|&&e| e >= prev && e < *threshold)
                .count();
            let pct = count as f64 / total * 100.0;
            eprintln!("    {label:>6}: {count:4} beats ({pct:5.1}%)");
            prev = *threshold;
        }

        // Signed drift in 30s windows — reveals where alignment breaks down
        // and whether errors are systematic (old MIDI) vs isolated (ts change).
        eprintln!("\n  Signed drift by 30s window (positive = MIDI ahead of click):");
        eprintln!(
            "  {:>8}  {:>8}  {:>8}  {:>8}  {:>8}",
            "window", "beats", "mean", "min", "max"
        );
        let window = 30.0f64;
        let song_end = drift_series.last().map(|(t, _)| *t).unwrap_or(0.0);
        let mut t = drift_series.first().map(|(t, _)| *t).unwrap_or(0.0);
        while t < song_end {
            let slice: Vec<f64> = drift_series
                .iter()
                .filter(|(mt, _)| *mt >= t && *mt < t + window)
                .map(|(_, d)| *d)
                .collect();
            if !slice.is_empty() {
                let wmean = slice.iter().sum::<f64>() / slice.len() as f64;
                let wmin = slice.iter().cloned().fold(f64::INFINITY, f64::min);
                let wmax = slice.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                eprintln!(
                    "  {:>7.0}s  {:>8}  {:>7.1}ms  {:>7.1}ms  {:>7.1}ms",
                    t,
                    slice.len(),
                    wmean,
                    wmin,
                    wmax
                );
            }
            t += window;
        }
    }

    #[test]
    fn saxon_shore_beat_grid_alignment() {
        eval_midi_vs_click("Isenmor/Saxon Shore", "Saxon Shore.mid", "Click.flac", 0);
    }

    /// Deep-dive on the Saxon Shore lead-in detection.
    ///
    /// Checks what click beats fall before the detected offset, compares
    /// alignment scores for offset=0 vs the found offset, and prints beat
    /// spacing in the lead-in region to reveal whether those clicks look
    /// like a count-in or actual song content.
    #[test]
    fn saxon_shore_leadin_analysis() {
        let base = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
            .join("src/backing-tracks/Isenmor/Saxon Shore");
        let midi_path = base.join("Saxon Shore.mid");
        let click_path = base.join("Click.flac");

        if !midi_path.exists() || !click_path.exists() {
            eprintln!("Skipping: files not found");
            return;
        }

        let grid = crate::audio::click_analysis::analyze_click_track_default(&click_path, 0)
            .expect("click analysis failed");

        let data = std::fs::read(&midi_path).unwrap();
        let smf = midly::Smf::parse(&data).unwrap();
        let tpb = match smf.header.timing {
            midly::Timing::Metrical(tpb) => tpb.as_int() as u64,
            _ => panic!("SMPTE not supported"),
        };
        let (tempo_events, time_sig_events) = extract_events(&smf);

        // What offset did the cross-correlation choose?
        let found_offset = find_best_offset(&tempo_events, &time_sig_events, tpb, &grid);
        let max_time = grid.beats.last().copied().unwrap_or(0.0);

        // Score at the found offset vs offset=0.
        let score_found = {
            let mb = midi_beat_times(&tempo_events, &time_sig_events, tpb, found_offset, max_time);
            alignment_score(&mb, &grid.beats)
        };
        let score_zero = {
            let mb = midi_beat_times(&tempo_events, &time_sig_events, tpb, 0.0, max_time);
            alignment_score(&mb, &grid.beats)
        };

        eprintln!("Found offset: {found_offset:.4}s  score={score_found:.6}");
        eprintln!("Zero  offset: 0.0000s  score={score_zero:.6}");
        eprintln!(
            "Score delta (found - zero): {:.6}",
            score_found - score_zero
        );

        // Click beats before the found offset — are these a count-in?
        let pre_beats: Vec<f64> = grid
            .beats
            .iter()
            .copied()
            .filter(|&t| t < found_offset)
            .collect();
        eprintln!(
            "\nClick beats before found offset ({found_offset:.3}s): {} beats",
            pre_beats.len()
        );
        if pre_beats.len() <= 30 {
            for (i, &t) in pre_beats.iter().enumerate() {
                let spacing = if i > 0 {
                    format!("  (+{:.3}s)", t - pre_beats[i - 1])
                } else {
                    String::new()
                };
                eprintln!("  beat {i:>2}: {t:.4}s{spacing}");
            }
        }

        // Beat spacing statistics in the lead-in vs the main body.
        let spacings_leadin: Vec<f64> = pre_beats.windows(2).map(|w| w[1] - w[0]).collect();
        let post_beats: Vec<f64> = grid
            .beats
            .iter()
            .copied()
            .filter(|&t| t >= found_offset)
            .take(20)
            .collect();
        let spacings_post: Vec<f64> = post_beats.windows(2).map(|w| w[1] - w[0]).collect();

        if !spacings_leadin.is_empty() {
            let mean_li = spacings_leadin.iter().sum::<f64>() / spacings_leadin.len() as f64;
            eprintln!(
                "\nLead-in beat spacing: mean={:.4}s  ({:.1} BPM)",
                mean_li,
                60.0 / mean_li
            );
        }
        if !spacings_post.is_empty() {
            let mean_post = spacings_post.iter().sum::<f64>() / spacings_post.len() as f64;
            eprintln!(
                "Post-offset beat spacing (first 20): mean={:.4}s  ({:.1} BPM)",
                mean_post,
                60.0 / mean_post
            );
        }

        // Also show alignment score for every beat in the first 10s as candidate.
        eprintln!("\nAlignment scores for candidates in first 10s:");
        eprintln!("  {:>8}  {:>12}  note", "offset", "score");
        let mut candidates: Vec<f64> = vec![0.0];
        candidates.extend(grid.beats.iter().copied().filter(|&t| t <= 10.0));
        candidates.dedup_by(|a, b| (*a - *b).abs() < 0.001);
        for c in &candidates {
            let mb = midi_beat_times(&tempo_events, &time_sig_events, tpb, *c, max_time);
            let score = alignment_score(&mb, &grid.beats);
            let marker = if (*c - found_offset).abs() < 0.002 {
                " ← chosen"
            } else {
                ""
            };
            eprintln!("  {:>8.4}s  {:>12.6}{marker}", c, score);
        }
    }

    #[test]
    fn sigurds_song_beat_grid_alignment() {
        eval_midi_vs_click("Isenmor/Sigurd's Song", "Midi.mid", "Click.flac", 0);
    }

    #[test]
    fn operation_orcinianus_copia_beat_grid_alignment() {
        eval_midi_vs_click(
            "Recently Vacated Graves/Operation Orcinianus Copia",
            "automation.mid",
            "Click.flac",
            0,
        );
    }

    #[test]
    fn alignment_rms_values() {
        let base = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
            .join("src/backing-tracks");

        let songs: &[(&str, &str, &str)] = &[
            ("Isenmor/Beornulf", "Beornulf.mid", "Click.flac"),
            ("Isenmor/Battle Scarred", "Battle Scarred.mid", "Click.flac"),
            ("Isenmor/Jotunheim", "Jotunheim.mid", "Click.flac"),
            ("Isenmor/Afar", "Afar.mid", "Click.flac"),
            ("Isenmor/The Pursuit of Vikings", "Midi.mid", "Click.flac"),
            ("Isenmor/Saxon Shore", "Saxon Shore.mid", "Click.flac"),
            ("Isenmor/Sigurd's Song", "Midi.mid", "Click.flac"),
            (
                "Recently Vacated Graves/Operation Orcinianus Copia",
                "automation.mid",
                "Click.flac",
            ),
        ];

        eprintln!("\n{:<50} {:>14}  note", "Song", "alignment_rms_ms");
        eprintln!("{}", "-".repeat(72));
        for (dir, midi, click) in songs {
            let midi_path = base.join(dir).join(midi);
            let click_path = base.join(dir).join(click);
            if !midi_path.exists() || !click_path.exists() {
                eprintln!("{:<50} (skipped — files not found)", dir);
                continue;
            }
            let grid = crate::audio::click_analysis::analyze_click_track_default(&click_path, 0);
            let result = extract_tempo_from_midi(&midi_path, grid.as_ref());
            match result {
                Some(t) => {
                    let rms = t
                        .alignment_rms_ms
                        .map(|v| format!("{v:>10.2}ms"))
                        .unwrap_or_else(|| "       n/a".into());
                    let flag = t
                        .alignment_rms_ms
                        .map(|v| if v > 15.0 { " ⚠ poor" } else { "" })
                        .unwrap_or("");
                    eprintln!("{:<50} {rms}{flag}", dir);
                }
                None => eprintln!("{:<50} (extraction failed)", dir),
            }
        }
    }

    #[test]
    fn batch_beat_grid_alignment() {
        let songs: &[(&str, &str, &str)] = &[
            // Isenmor
            ("Isenmor/Afar", "Afar.mid", "Click.flac"),
            ("Isenmor/Beornulf", "Beornulf.mid", "Click.flac"),
            ("Isenmor/Battle Scarred", "Battle Scarred.mid", "Click.flac"),
            (
                "Isenmor/Death is a Fine Companion",
                "Death is a Fine Companion.mid",
                "Click.flac",
            ),
            ("Isenmor/Jotunheim", "Jotunheim.mid", "Click.flac"),
            ("Isenmor/The Pursuit of Vikings", "Midi.mid", "Click.flac"),
            ("Isenmor/Throneless", "Throneless.mid", "Click.flac"),
            ("Isenmor/Wanderlust", "Wanderlust.mid", "Click.flac"),
            // RVG
            (
                "Recently Vacated Graves/Bored to Undeath",
                "automation.mid",
                "Click.flac",
            ),
            (
                "Recently Vacated Graves/Hurricane Zombie",
                "automation.mid",
                "Click.flac",
            ),
            (
                "Recently Vacated Graves/Send More Cops",
                "automation.mid",
                "Click.flac",
            ),
            (
                "Recently Vacated Graves/Zombie Ritual",
                "automation.mid",
                "Click.flac",
            ),
            (
                "Recently Vacated Graves/Devoured in Decay",
                "automation.mid",
                "Click.flac",
            ),
            (
                "Recently Vacated Graves/You Die",
                "automation.mid",
                "Click.flac",
            ),
        ];

        for (dir, midi, click) in songs {
            eval_midi_vs_click(dir, midi, click, 0);
        }
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
            let result = extract_tempo_from_midi(&alt, None).unwrap();
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

        let result = extract_tempo_from_midi(&midi_path, None).unwrap();
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

        let result = extract_tempo_from_midi(&midi_path, None).unwrap();

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

    // --- tick_to_seconds tests ---

    #[test]
    fn tick_to_seconds_zero() {
        let events = vec![TempoEvent {
            tick: 0,
            micros_per_beat: bpm_to_micros(120),
        }];
        assert_eq!(tick_to_seconds(0, &events, 480), 0.0);
    }

    #[test]
    fn tick_to_seconds_simple() {
        // 120 BPM, 480 tpb: 1920 ticks = 4 beats = 2.0s
        let events = vec![TempoEvent {
            tick: 0,
            micros_per_beat: bpm_to_micros(120),
        }];
        let result = tick_to_seconds(1920, &events, 480);
        assert!((result - 2.0).abs() < 1e-9);
    }

    #[test]
    fn tick_to_seconds_with_tempo_change() {
        // 120 BPM for 960 ticks (1.0s), then 60 BPM.
        // 1200 ticks = 960 at 120BPM (1.0s) + 240 at 60BPM (0.5s) = 1.5s
        let events = vec![
            TempoEvent {
                tick: 0,
                micros_per_beat: bpm_to_micros(120),
            },
            TempoEvent {
                tick: 960,
                micros_per_beat: bpm_to_micros(60),
            },
        ];
        let result = tick_to_seconds(1200, &events, 480);
        assert!((result - 1.5).abs() < 1e-9);
    }

    #[test]
    fn tick_to_seconds_roundtrip() {
        let events = vec![
            TempoEvent {
                tick: 0,
                micros_per_beat: bpm_to_micros(120),
            },
            TempoEvent {
                tick: 960,
                micros_per_beat: bpm_to_micros(90),
            },
        ];

        for target_secs in [0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 5.0] {
            let tick = seconds_to_tick(target_secs, &events, 480);
            let back = tick_to_seconds(tick, &events, 480);
            assert!(
                (back - target_secs).abs() < 0.002,
                "Roundtrip failed for {}s: got {}s (via tick {})",
                target_secs,
                back,
                tick,
            );
        }
    }

    // --- alignment score tests ---

    #[test]
    fn alignment_score_perfect() {
        let beats = vec![0.0, 0.5, 1.0, 1.5, 2.0];
        let score = alignment_score(&beats, &beats);
        assert!(
            (score - 0.0).abs() < 1e-12,
            "Perfect alignment should score 0.0"
        );
    }

    #[test]
    fn alignment_score_decreases_with_drift() {
        let midi_beats: Vec<f64> = (0..20).map(|i| i as f64 * 0.5).collect();

        let good_grid: Vec<f64> = (0..20).map(|i| i as f64 * 0.5).collect();
        let bad_grid: Vec<f64> = (0..20).map(|i| i as f64 * 0.5 + 0.05).collect();

        let good_score = alignment_score(&midi_beats, &good_grid);
        let bad_score = alignment_score(&midi_beats, &bad_grid);

        assert!(good_score > bad_score, "Drifted grid should score worse");
    }

    // --- find_best_offset tests ---

    #[test]
    fn find_offset_no_leadin() {
        // MIDI at 120 BPM, beat grid starts at 0.0.
        let events = vec![TempoEvent {
            tick: 0,
            micros_per_beat: bpm_to_micros(120),
        }];
        let grid = make_beat_grid(120.0, 0.0, 32, 4);

        let offset = find_best_offset(&events, &[], 480, &grid);
        assert!(offset.abs() < 0.01, "Expected ~0.0, got {}", offset);
    }

    #[test]
    fn find_offset_with_leadin() {
        // MIDI at 120 BPM, beat grid starts at 2.0s (4-beat lead-in).
        let events = vec![TempoEvent {
            tick: 0,
            micros_per_beat: bpm_to_micros(120),
        }];
        let grid = make_beat_grid(120.0, 2.0, 32, 4);

        let offset = find_best_offset(&events, &[], 480, &grid);
        assert!((offset - 2.0).abs() < 0.01, "Expected ~2.0, got {}", offset);
    }

    #[test]
    fn find_offset_with_tempo_change_in_leadin() {
        // 100 BPM lead-in, then 140 BPM at tick 1600 (~2.0s).
        let events = vec![
            TempoEvent {
                tick: 0,
                micros_per_beat: bpm_to_micros(100),
            },
            TempoEvent {
                tick: 1600,
                micros_per_beat: bpm_to_micros(140),
            },
        ];
        let grid = make_beat_grid(140.0, 2.0, 32, 4);

        let offset = find_best_offset(&events, &[], 480, &grid);
        assert!((offset - 2.0).abs() < 0.02, "Expected ~2.0, got {}", offset);
    }

    // ── beat_step_ticks tests ─────────────────────────────────────────────────

    #[test]
    fn beat_step_ticks_simple_meters() {
        let tpb = 480_u64;
        // 2/2 (cut time): half-note pulse = 2 × tpb
        assert_eq!(
            beat_step_ticks(2, 1, tpb),
            2 * tpb,
            "2/2 should step by half notes"
        );
        // 4/4: quarter-note pulse = 1 × tpb
        assert_eq!(
            beat_step_ticks(4, 2, tpb),
            tpb,
            "4/4 should step by quarter notes"
        );
        // 3/4: quarter-note pulse = 1 × tpb
        assert_eq!(
            beat_step_ticks(3, 2, tpb),
            tpb,
            "3/4 should step by quarter notes"
        );
        // 7/8: simple eighth-note pulse = tpb/2
        assert_eq!(
            beat_step_ticks(7, 3, tpb),
            tpb / 2,
            "7/8 should step by eighth notes"
        );
        // 4/16: sixteenth-note pulse = tpb/4
        assert_eq!(
            beat_step_ticks(4, 4, tpb),
            tpb / 4,
            "4/16 should step by sixteenth notes"
        );
    }

    #[test]
    fn beat_step_ticks_compound_meters() {
        let tpb = 480_u64;
        // 6/8: compound duple — dotted-quarter pulse = 3×tpb/2 = 720
        assert_eq!(
            beat_step_ticks(6, 3, tpb),
            3 * tpb / 2,
            "6/8 should step by dotted quarters"
        );
        // 9/8: compound triple — dotted-quarter pulse
        assert_eq!(
            beat_step_ticks(9, 3, tpb),
            3 * tpb / 2,
            "9/8 should step by dotted quarters"
        );
        // 12/8: compound quadruple — dotted-quarter pulse
        assert_eq!(
            beat_step_ticks(12, 3, tpb),
            3 * tpb / 2,
            "12/8 should step by dotted quarters"
        );
        // 3/8: simple (numerator not divisible by 3 in compound sense… wait, 3%3==0)
        // 3/8 is technically simple triple but shares numerator%3==0 with compound;
        // our heuristic treats it as compound (dotted-quarter), which is acceptable
        // since a 3/8 click track is usually one dotted-quarter per bar anyway.
        assert_eq!(
            beat_step_ticks(3, 3, tpb),
            3 * tpb / 2,
            "3/8 treated as compound dotted-quarter"
        );
    }

    // ── epsilon tie-breaking tests ─────────────────────────────────────────────

    #[test]
    fn find_offset_prefers_earlier_when_scores_are_tied() {
        // A perfectly constant-tempo grid with no lead-in.  Every candidate
        // offset is equally valid because removing early beats never hurts MSE
        // in a perfectly aligned grid (all errors are 0).  The epsilon rule
        // should resolve the tie in favour of the candidate closest to the
        // grid's first beat (i.e. 0.0 — no lead-in).
        let tpb = 480_u64;
        let events = vec![TempoEvent {
            tick: 0,
            micros_per_beat: bpm_to_micros(120),
        }];
        // Grid starts at 0.0: beat grid has no lead-in, first beat at 0.0.
        let grid = make_beat_grid(120.0, 0.0, 64, 4);

        let offset = find_best_offset(&events, &[], tpb, &grid);
        // Should snap to 0.0, not drift to some later beat.
        assert!(
            offset.abs() < 0.01,
            "Expected offset ~0.0 (no lead-in), got {offset:.4}s — epsilon tie-breaking may have failed",
        );
    }

    #[test]
    fn find_offset_uses_later_offset_when_meaningfully_better() {
        // Grid starts at exactly 1 beat (0.5s at 120 BPM) into the song.
        // The correct offset is 0.5s, and the score with that offset should be
        // meaningfully better than 0.0 (because 0.0 would misalign every beat).
        let tpb = 480_u64;
        let events = vec![TempoEvent {
            tick: 0,
            micros_per_beat: bpm_to_micros(120),
        }];
        let grid = make_beat_grid(120.0, 0.5, 32, 4);

        let offset = find_best_offset(&events, &[], tpb, &grid);
        assert!(
            (offset - 0.5).abs() < 0.01,
            "Expected offset ~0.5s (1-beat lead-in), got {offset:.4}s",
        );
    }
}
