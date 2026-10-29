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

use std::collections::{BTreeMap, HashSet};
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
/// up: at the first event, starting at 0, else past every event up to and
/// including `from`, which [`replay_midi_dmx`] puts back -- the live loop
/// dispatches an event once its time is reached, so one at `from` could
/// otherwise go out ahead of the store's clear and be lost to it.
pub(super) fn midi_dmx_cursor(events: &[TimedMidiEvent], from: Duration) -> usize {
    if from.is_zero() {
        0
    } else {
        events.partition_point(|e| e.time <= from)
    }
}

/// What a playback's start puts back on one universe (see
/// [`Engine::midi_dmx_restore`]).
pub(super) struct MidiDmxRestore {
    universe_id: u16,
    states: BTreeMap<u16, MidiDmxChannelState>,
    /// The last dimming speed the events set.
    dimming: Option<Duration>,
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
/// `dim_rate` is the dim rate (ticks) the replay starts from; also returns
/// the last dimming speed the events set, for the playback to carry on with.
pub(super) fn replay_midi_dmx(
    events: &[TimedMidiEvent],
    midi_channels: &HashSet<u8>,
    until: Duration,
    dimming_speed_modifier: f64,
    mut dim_rate: f64,
) -> (BTreeMap<u16, MidiDmxChannelState>, Option<Duration>) {
    let mut channels: BTreeMap<u16, Written> = BTreeMap::new();
    let mut dimming = None;
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
                    dimming = Some(duration);
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
    (states, dimming)
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

    /// Where the events before a playback's start point leave a
    /// universe's MIDI DMX channels (see [`replay_midi_dmx`]), to put them
    /// back with [`Self::apply_midi_dmx_restore`]. Nothing for a start at 0.
    pub(super) fn midi_dmx_restore(
        &self,
        universe_id: u16,
        events: &[TimedMidiEvent],
        midi_channels: &HashSet<u8>,
        until: Duration,
    ) -> Option<MidiDmxRestore> {
        if until.is_zero() {
            return None;
        }
        // The rate the universe dims at now, which is the store's too where
        // it has the universe: both are set together.
        let dim_rate = self
            .universes
            .get(&universe_id)
            .map_or(1.0, |universe| universe.dim_rate());
        let (states, dimming) = replay_midi_dmx(
            events,
            midi_channels,
            until,
            self.dimming_speed_modifier,
            dim_rate,
        );
        Some(MidiDmxRestore {
            universe_id,
            states,
            dimming,
        })
    }

    /// Puts a universe's MIDI DMX channels back as [`Self::midi_dmx_restore`]
    /// found them. Run once the start has cleared the store, under the
    /// engine's lock that clear was made under, so no frame renders the
    /// store empty in between: a black flash at every seek (#581).
    pub(super) fn apply_midi_dmx_restore(&self, restore: &MidiDmxRestore) {
        let universe_id = restore.universe_id;
        if let Some(duration) = restore.dimming {
            self.update_dimming_by_id(universe_id, duration);
        }
        let store = self.midi_dmx_store.read();
        for (&channel, state) in &restore.states {
            if store.lookup(universe_id, channel).is_some() {
                store.restore(
                    universe_id,
                    channel,
                    state.current,
                    state.target,
                    state.rate,
                );
            } else if let Some(universe) = self.universes.get(&universe_id) {
                universe.restore_channel_data(channel, state.current, state.target, state.rate);
            }
        }
    }

    /// Moves every MIDI DMX playback's cursor to `time` (in the delayed
    /// lighting time base) and returns what the events before it leave the
    /// channels at, for the start that clears the store to put back.
    pub(super) fn seek_midi_dmx_playbacks(&self, time: Duration) -> Vec<MidiDmxRestore> {
        let mut playbacks = self.midi_dmx_playbacks.lock();
        playbacks
            .iter_mut()
            .filter_map(|playback| {
                let events = playback.precomputed.events();
                let cursor = midi_dmx_cursor(events, time);
                let restore = self.midi_dmx_restore(
                    playback.universe_id,
                    &events[..cursor],
                    &playback.midi_channels,
                    time,
                );
                playback.cursor = cursor;
                restore
            })
            .collect()
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
        assert_eq!(dimming, None);
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
        assert_eq!(dimming, Some(Duration::from_secs(2)));

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
