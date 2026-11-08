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

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::Duration;

use super::{classify_midi_dmx_action, Engine, MidiDmxAction};
use crate::dmx::universe::{dim_ticks, dimmed_at, TARGET_HZ};
use crate::midi::playback::TimedMidiEvent;

/// Where a channel MIDI DMX drove stands at a point in the song: at
/// `current`, moving toward `target` by `rate` a tick (0 once there).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct MidiDmxChannelState {
    pub(super) current: f64,
    pub(super) target: f64,
    pub(super) rate: f64,
}

/// Where a MIDI DMX playback starting at `from` (delayed song time) picks
/// up: past every event a frame before the one at `from` dispatched, which
/// [`replay_midi_dmx`] puts back. One due by `from` but not a frame before
/// is left to the first frame, which dispatches it as the live run's frame
/// at `from` did: a CC still not out, a dim not yet stepped. From 0 (inside
/// the playback delay too, where the live loop holds them back), at the
/// first event.
pub(super) fn midi_dmx_cursor(events: &[TimedMidiEvent], from: Duration) -> usize {
    events.partition_point(|e| frames_before(from, e.time) > 0)
}

/// How many frames before the one at `until` the live loop, ticking at
/// [`TARGET_HZ`] on to `until`, dispatched an event at `time` in: 0 for
/// the frame at `until` itself (or later). A frame period is not a whole
/// number of nanoseconds, so a time within a nanosecond of a frame counts
/// as on it -- an event at 0 as the first frame's, whatever `until`.
fn frames_before(until: Duration, time: Duration) -> u64 {
    if time > until {
        return 0;
    }
    let nanos = (until - time).as_nanos() + 1;
    // TARGET_HZ is a whole 44.
    let frames = nanos * (TARGET_HZ as u128) / 1_000_000_000;
    u64::try_from(frames).unwrap_or(u64::MAX)
}

/// The DMX channels the notes and CCs among `events` write.
fn midi_dmx_channels(events: &[TimedMidiEvent], midi_channels: &HashSet<u8>) -> BTreeSet<u16> {
    events
        .iter()
        .filter(|e| midi_channels.is_empty() || midi_channels.contains(&e.channel))
        .filter_map(|e| match classify_midi_dmx_action(e.message, 1.0) {
            MidiDmxAction::KeyVelocity { channel, .. }
            | MidiDmxAction::Controller { channel, .. } => Some(channel),
            _ => None,
        })
        .collect()
}

/// A channel as the last write left it, dispatched `frames` frames before
/// the one a playback starts at.
struct Written {
    from: f64,
    target: f64,
    rate: f64,
    frames: u64,
}

impl Written {
    /// Where the channel has got to by the frame `frames` before the start,
    /// once its tick is in: as the store and the universe step it, `rate` a
    /// tick up to the target, ticking every frame after the write's.
    fn at(&self, frames: u64) -> f64 {
        dimmed_at(
            self.from,
            self.target,
            self.rate,
            self.frames.saturating_sub(frames) as f64,
        )
    }
}

/// The state the MIDI DMX events a frame before the one at `until`
/// dispatched leave each DMX channel in, replayed without their timing: a
/// playback starting part way through a song skips them, and since nothing
/// holds a channel no frame writes (#581), a look a MIDI show set once and
/// held stayed dark after a seek (#583). Notes and CCs land with their
/// values, a dim at the level it has reached and still on its way if it
/// has not finished. The level is the one the frame before `until` left:
/// the first frame ticks it on, then dispatches what is due at `until`
/// (see [`midi_dmx_cursor`]), so it sends the bytes the live run's frame at
/// `until` sent -- restored at `until`'s level, the tick took every dim one
/// step past it. `dim_rate` is the dim rate (ticks for a full transition)
/// the replay starts from; also returns the one the events leave, for the
/// playback to carry on with.
pub(super) fn replay_midi_dmx(
    events: &[TimedMidiEvent],
    midi_channels: &HashSet<u8>,
    until: Duration,
    dimming_speed_modifier: f64,
    mut dim_rate: f64,
) -> (BTreeMap<u16, MidiDmxChannelState>, f64) {
    let mut channels: BTreeMap<u16, Written> = BTreeMap::new();
    for event in events {
        let frames = frames_before(until, event.time);
        if frames == 0 {
            break;
        }
        if !midi_channels.is_empty() && !midi_channels.contains(&event.channel) {
            continue;
        }
        let (channel, value, dim) =
            match classify_midi_dmx_action(event.message, dimming_speed_modifier) {
                MidiDmxAction::KeyVelocity { channel, value } => (channel, value, true),
                MidiDmxAction::Controller { channel, value } => (channel, value, false),
                MidiDmxAction::Dimming { duration } => {
                    dim_rate = dim_ticks(duration);
                    continue;
                }
                MidiDmxAction::Unrecognized => continue,
            };
        // Ticked each frame up to and including the one this went out in.
        let from = channels.get(&channel).map_or(0.0, |w| w.at(frames));
        let target = f64::from(value);
        let rate = if dim {
            crate::dmx::universe::dim_rate(from, target, dim_rate)
        } else {
            0.0
        };
        channels.insert(
            channel,
            Written {
                from,
                target,
                rate,
                frames,
            },
        );
    }
    let states = channels
        .into_iter()
        .map(|(channel, written)| {
            let current = written.at(1);
            let rate = if (current - written.target).abs() > f64::EPSILON {
                written.rate
            } else {
                0.0
            };
            (
                channel,
                MidiDmxChannelState {
                    current,
                    target: written.target,
                    rate,
                },
            )
        })
        .collect();
    (states, dim_rate)
}

impl Engine {
    /// Advances all MIDI DMX playback cursors to the current song time,
    /// dispatching events via handle_midi_event_by_id.
    pub(super) fn advance_midi_dmx_playbacks(&self) {
        let song_time = match self.delayed_song_time() {
            Some(t) => t,
            None => return, // Still within the playback delay period
        };
        let mut playbacks = self.midi_dmx_playbacks.lock();
        for playback in playbacks.iter_mut() {
            let events = playback.precomputed.events();
            while playback.cursor < events.len() && events[playback.cursor].time <= song_time {
                let event = &events[playback.cursor];
                if playback.midi_channels.is_empty()
                    || playback.midi_channels.contains(&event.channel)
                {
                    self.handle_midi_event_by_id(playback.universe_id, event.message);
                }
                playback.cursor += 1;
            }
        }
    }

    /// Moves every MIDI DMX playback's cursor to `from` (delayed song time)
    /// and puts the channels back where the events before it leave them
    /// (see [`replay_midi_dmx`]), the dim rate with them: as it was when the
    /// playback started, for a section loop back to before the first
    /// dimming change. Run under the engine's lock, once a start has cleared
    /// the store: no frame renders the store empty in between (a black flash
    /// at every seek, #581), and the live loop dispatches nothing ahead of
    /// the clear for it to wipe.
    pub(super) fn seek_midi_dmx_playbacks(&self, from: Duration) {
        self.move_midi_dmx_playbacks(from, true);
    }

    /// Moves every MIDI DMX playback to `from` (delayed song time) for a
    /// crossfading song's pass begun late there: the channels the events
    /// before it set are put back, as for a seek, but one they set only
    /// later keeps what the outgoing pass left it, as the store carries it
    /// into an on-time pass (see [`Self::rewind_midi_dmx_playbacks`]).
    pub(super) fn join_midi_dmx_playbacks(&self, from: Duration) {
        self.move_midi_dmx_playbacks(from, false);
    }

    fn move_midi_dmx_playbacks(&self, from: Duration, zero_unset: bool) {
        let mut playbacks = self.midi_dmx_playbacks.lock();
        for playback in playbacks.iter_mut() {
            let events = playback.precomputed.events();
            let cursor = midi_dmx_cursor(events, from);
            let (states, dim_rate) = replay_midi_dmx(
                &events[..cursor],
                &playback.midi_channels,
                from,
                self.dimming_speed_modifier,
                playback.start_dim_rate,
            );
            self.restore_midi_dmx(playback.universe_id, &states, dim_rate);
            // The clear before this let go of the store's channels; one
            // straight on the universe holds whatever it was last sent, so
            // one the events write but had not by `from` goes back to 0 too
            // -- a section loop's second pass would otherwise start with
            // what the first set later on.
            if let Some(universe) = self
                .universes
                .get(&playback.universe_id)
                .filter(|_| zero_unset)
            {
                let store = self.midi_dmx_store.read();
                for channel in midi_dmx_channels(events, &playback.midi_channels) {
                    if !states.contains_key(&channel)
                        && store.lookup(playback.universe_id, channel).is_none()
                    {
                        universe.restore_channel_data(channel, 0.0, 0.0, 0.0);
                    }
                }
            }
            playback.cursor = cursor;
        }
    }

    /// Moves every MIDI DMX playback back to its first event, at the dim
    /// rate it started with, leaving the channels as they are: a loop
    /// crossfade carries them into the next pass.
    pub(super) fn rewind_midi_dmx_playbacks(&self) {
        let mut playbacks = self.midi_dmx_playbacks.lock();
        for playback in playbacks.iter_mut() {
            playback.cursor = 0;
            self.restore_midi_dmx(
                playback.universe_id,
                &BTreeMap::new(),
                playback.start_dim_rate,
            );
        }
    }

    /// Puts a universe's MIDI DMX channels and dim rate back as
    /// [`replay_midi_dmx`] found them.
    fn restore_midi_dmx(
        &self,
        universe_id: u16,
        states: &BTreeMap<u16, MidiDmxChannelState>,
        dim_rate: f64,
    ) {
        let universe = self.universes.get(&universe_id);
        if let Some(universe) = universe {
            universe.set_dim_rate(dim_rate);
        }
        let store = self.midi_dmx_store.read();
        store.set_dim_rate(universe_id, dim_rate);
        for (&channel, state) in states {
            if store.lookup(universe_id, channel).is_some() {
                store.restore(
                    universe_id,
                    channel,
                    state.current,
                    state.target,
                    state.rate,
                );
            } else if let Some(universe) = universe {
                universe.restore_channel_data(channel, state.current, state.target, state.rate);
            }
        }
    }

    /// Returns true if all MIDI DMX playbacks have finished.
    pub(super) fn midi_dmx_playbacks_finished(&self) -> bool {
        let playbacks = self.midi_dmx_playbacks.lock();
        playbacks.iter().all(|p| p.cursor >= p.precomputed.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midly::num::u7;
    use midly::MidiMessage;

    fn at(ms: u64, channel: u8, message: MidiMessage) -> TimedMidiEvent {
        TimedMidiEvent {
            time: Duration::from_millis(ms),
            channel,
            message,
        }
    }

    fn cc(controller: u8, value: u8) -> MidiMessage {
        MidiMessage::Controller {
            controller: u7::new(controller),
            value: u7::new(value),
        }
    }

    fn note(key: u8, vel: u8) -> MidiMessage {
        MidiMessage::NoteOn {
            key: u7::new(key),
            vel: u7::new(vel),
        }
    }

    fn program(program: u8) -> MidiMessage {
        MidiMessage::ProgramChange {
            program: u7::new(program),
        }
    }

    #[test]
    fn values_before_the_start_land_and_later_ones_do_not() {
        let events = [
            at(0, 0, cc(0, 127)),
            at(100, 0, note(1, 50)),
            at(200, 0, cc(0, 10)),
            at(5_000, 0, cc(2, 127)),
        ];
        let (states, dimming) =
            replay_midi_dmx(&events, &HashSet::new(), Duration::from_secs(3), 1.0, 1.0);
        let state = |c| states.get(&c).copied();
        assert_eq!(state(1).map(|s| s.current), Some(20.0), "last CC wins");
        assert_eq!(
            state(2).map(|s| s.current),
            Some(100.0),
            "a note's velocity"
        );
        assert_eq!(state(3), None, "an event after the start");
        assert_eq!(dimming, 1.0, "the rate it started from");
    }

    /// Where a restored channel is once the first frame's tick is in.
    fn ticked(state: MidiDmxChannelState) -> f64 {
        crate::dmx::universe::dim_step(state.current, state.target, state.rate)
    }

    #[test]
    fn a_dim_under_way_comes_back_part_way_and_still_moving() {
        // Dim over 2 s (program 2 at a 1 s modifier), up from 0 to 200 at
        // 1 s; at 2 s it is half way there -- once the first frame ticks:
        // the replay leaves it where the frame before left it.
        let events = [at(0, 0, program(2)), at(1_000, 0, note(0, 100))];
        let (states, dimming) =
            replay_midi_dmx(&events, &HashSet::new(), Duration::from_secs(2), 1.0, 1.0);
        let state = states[&1];
        assert!((ticked(state) - 100.0).abs() < 1e-6, "{state:?}");
        assert!(
            (state.current - 200.0 * 43.0 / 88.0).abs() < 1e-6,
            "{state:?}"
        );
        assert_eq!(state.target, 200.0);
        assert!((state.rate - 200.0 / (2.0 * TARGET_HZ)).abs() < 1e-9);
        assert_eq!(dimming, 2.0 * TARGET_HZ);

        // Past its end, it is there and still.
        let (states, _) =
            replay_midi_dmx(&events, &HashSet::new(), Duration::from_secs(4), 1.0, 1.0);
        assert_eq!(
            states[&1],
            MidiDmxChannelState {
                current: 200.0,
                target: 200.0,
                rate: 0.0
            }
        );
    }

    #[test]
    fn a_dim_starts_from_where_the_last_one_had_got_to() {
        // Up to 200 over 2 s from 0, then at 1 s (100 reached) down to 0
        // over the same 2 s: from 100, so it is at 50 at 2 s.
        let events = [
            at(0, 0, program(2)),
            at(0, 0, note(0, 100)),
            at(1_000, 0, note(0, 0)),
        ];
        let (states, _) =
            replay_midi_dmx(&events, &HashSet::new(), Duration::from_secs(2), 1.0, 1.0);
        assert!((ticked(states[&1]) - 50.0).abs() < 1e-6, "{:?}", states[&1]);
    }

    #[test]
    fn a_write_due_in_the_first_frame_is_left_to_it() {
        // Due after the frame before 1 s (at 977.27 ms): the first frame
        // dispatches it, as the live run's frame at 1 s did.
        let events = [
            at(0, 0, cc(0, 10)),
            at(977, 0, cc(0, 20)),
            at(978, 0, cc(0, 30)),
            at(1_000, 0, cc(0, 40)),
        ];
        let until = Duration::from_secs(1);
        assert_eq!(midi_dmx_cursor(&events, until), 2);
        let (states, _) = replay_midi_dmx(&events, &HashSet::new(), until, 1.0, 1.0);
        assert_eq!(states[&1].current, 40.0, "the CC at 977 ms, doubled");
        assert_eq!(midi_dmx_cursor(&events, Duration::ZERO), 0);
    }

    #[test]
    fn frames_count_back_from_the_start_frame() {
        let frame = |k: u64| Duration::from_nanos(k * 1_000_000_000 / 44);
        // On a frame, to the nanosecond the grid has it: that frame's.
        for k in [0, 1, 7, 43, 44, 1_000, 123_457] {
            assert_eq!(frames_before(frame(k), Duration::ZERO), k, "{k}");
            assert_eq!(frames_before(frame(k + 5), frame(k)), 5, "{k}");
            assert_eq!(
                frames_before(frame(k + 5), frame(k) + Duration::from_nanos(2)),
                4
            );
        }
        assert_eq!(frames_before(frame(3), frame(4)), 0, "after the start");
    }

    #[test]
    fn only_the_shows_midi_channels_replay() {
        let events = [at(0, 0, cc(0, 127)), at(0, 3, cc(1, 127))];
        let (states, _) = replay_midi_dmx(
            &events,
            &HashSet::from([3]),
            Duration::from_secs(1),
            1.0,
            1.0,
        );
        assert!(!states.contains_key(&1));
        assert!(states.contains_key(&2));
    }
}
