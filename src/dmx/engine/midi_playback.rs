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
use crate::dmx::universe::TARGET_HZ;
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
/// up: at the first event, starting at 0 -- inside the playback delay too,
/// where the live loop holds them back -- else past every event up to and
/// including `from`, which [`replay_midi_dmx`] puts back, as the live loop
/// would have dispatched them by then.
pub(super) fn midi_dmx_cursor(events: &[TimedMidiEvent], from: Duration) -> usize {
    if from.is_zero() {
        0
    } else {
        events.partition_point(|e| e.time <= from)
    }
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

/// A channel as the last write left it, `since` the time of that write.
struct Written {
    from: f64,
    target: f64,
    rate: f64,
    since: Duration,
}

impl Written {
    /// Where the channel has got to at `time`, dimming as the store and the
    /// universe do: `rate` a tick, at TARGET_HZ, up to the target.
    fn at(&self, time: Duration) -> f64 {
        if self.rate == 0.0 {
            return self.target;
        }
        let ticks = time.saturating_sub(self.since).as_secs_f64() * TARGET_HZ;
        let value = self.from + self.rate * ticks;
        if self.rate > 0.0 {
            value.min(self.target)
        } else {
            value.max(self.target)
        }
    }
}

/// The state the MIDI DMX events up to `until` leave each DMX channel in,
/// replayed without their timing: a playback starting part way through a
/// song skips them, and since nothing holds a channel no frame writes
/// (#581), a look a MIDI show set once and held stayed dark after a seek
/// (#583). Notes and CCs land with their values, a dim at the level it has
/// reached by `until` and still on its way if it has not finished.
/// `dim_rate` is the dim rate (ticks for a full transition) the replay
/// starts from; also returns the one the events leave, for the playback to
/// carry on with.
pub(super) fn replay_midi_dmx(
    events: &[TimedMidiEvent],
    midi_channels: &HashSet<u8>,
    until: Duration,
    dimming_speed_modifier: f64,
    mut dim_rate: f64,
) -> (BTreeMap<u16, MidiDmxChannelState>, f64) {
    let mut channels: BTreeMap<u16, Written> = BTreeMap::new();
    for event in events.iter().take_while(|e| e.time <= until) {
        if !midi_channels.is_empty() && !midi_channels.contains(&event.channel) {
            continue;
        }
        let (channel, value, dim) =
            match classify_midi_dmx_action(event.message, dimming_speed_modifier) {
                MidiDmxAction::KeyVelocity { channel, value } => (channel, value, true),
                MidiDmxAction::Controller { channel, value } => (channel, value, false),
                MidiDmxAction::Dimming { duration } => {
                    dim_rate = if duration.is_zero() {
                        1.0
                    } else {
                        duration.as_secs_f64() * TARGET_HZ
                    };
                    continue;
                }
                MidiDmxAction::Unrecognized => continue,
            };
        let from = channels.get(&channel).map_or(0.0, |w| w.at(event.time));
        let target = f64::from(value);
        let rate = if dim && dim_rate > f64::EPSILON {
            (target - from) / dim_rate
        } else {
            0.0
        };
        channels.insert(
            channel,
            Written {
                from,
                target,
                rate,
                since: event.time,
            },
        );
    }
    let states = channels
        .into_iter()
        .map(|(channel, written)| {
            let current = written.at(until);
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
            if let Some(universe) = self.universes.get(&playback.universe_id) {
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

    #[test]
    fn a_dim_under_way_comes_back_part_way_and_still_moving() {
        // Dim over 2 s (program 2 at a 1 s modifier), up from 0 to 200 at
        // 1 s; at 2 s it is half way there.
        let events = [at(0, 0, program(2)), at(1_000, 0, note(0, 100))];
        let (states, dimming) =
            replay_midi_dmx(&events, &HashSet::new(), Duration::from_secs(2), 1.0, 1.0);
        let state = states[&1];
        assert!((state.current - 100.0).abs() < 1e-6, "{state:?}");
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
        assert!((states[&1].current - 50.0).abs() < 1e-6, "{:?}", states[&1]);
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
