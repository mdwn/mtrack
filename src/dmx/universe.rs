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

use ola::DmxBuffer;
use parking_lot::RwLock;
use spin_sleep;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::error;

use crate::config;
use crate::lighting::channels::dmx_byte;
use crate::playsync::CancelHandle;

use super::engine::DmxMessage;

/// A DMX universe is 512 channels.
const UNIVERSE_SIZE: usize = 512;

/// The frames a second the effects loop and the universe threads run at.
pub(crate) const FRAMES_PER_SECOND: u64 = 44;

/// The target number of updates per second: [`FRAMES_PER_SECOND`].
pub(crate) const TARGET_HZ: f64 = FRAMES_PER_SECOND as f64;

/// When frame `k` of a run at [`FRAMES_PER_SECOND`] falls, from the run's
/// start: to the nanosecond below, as a period of 1/44 s is not a whole
/// number of them, so `k` frames span exactly `k` / 44 s, give or take
/// that nanosecond, however many there are.
pub(crate) fn frame_time(k: u64) -> Duration {
    let nanos = u128::from(k) * 1_000_000_000 / u128::from(FRAMES_PER_SECOND);
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// When a loop running at [`FRAMES_PER_SECOND`] runs its next frame: on
/// deadlines counted from its start, so it keeps the rate on average
/// whatever each frame takes and however late its sleeps wake. Restarting
/// the count from each frame's own time made every frame's lateness the
/// next one's too: the effects loop ran at about 43 Hz, and a dim it steps
/// took 2 to 4% longer than its time. Fallen more than a frame behind (a
/// stall), it starts counting again from then, rather than run the missed
/// frames back to back.
pub(crate) struct FrameClock {
    start: Instant,
    frame: u64,
}

impl FrameClock {
    /// A clock whose first frame is due at `start`.
    pub(crate) fn new(start: Instant) -> Self {
        Self { start, frame: 0 }
    }

    /// When the next frame is due.
    pub(crate) fn due(&self) -> Instant {
        self.start + frame_time(self.frame)
    }

    /// Counts the frame due, run at `now`, and says when the next is due.
    pub(crate) fn ran(&mut self, now: Instant) -> Instant {
        self.frame += 1;
        let due = self.due();
        if now > due + frame_time(1) {
            // More than a frame behind: the next a frame on from now.
            self.start = now;
            self.frame = 1;
        } else if self.frame >= FRAMES_PER_SECOND * 3_600 {
            // An hour of frames: counted from the last deadline instead, a
            // whole second on, so the count never grows without end.
            self.start += frame_time(self.frame);
            self.frame = 0;
        }
        self.due()
    }
}

/// The ticks a full transition takes before any dimming time is set, and
/// under none: one, so a write lands next tick.
pub(crate) const DEFAULT_DIM_TICKS: f64 = 1.0;

/// A MIDI DMX dimming time as the ticks a full transition takes, at
/// [`TARGET_HZ`]: one tick for none.
pub(crate) fn dim_ticks(dimming: Duration) -> f64 {
    if dimming.is_zero() {
        DEFAULT_DIM_TICKS
    } else {
        dimming.as_secs_f64() * TARGET_HZ
    }
}

/// What a MIDI DMX channel moves a tick, dimming from `current` to `target`
/// over `ticks`; over none, infinite (or no number), which [`dim_step`] and
/// [`dimmed_at`] take to the target at once.
pub(crate) fn dim_rate(current: f64, target: f64, ticks: f64) -> f64 {
    (target - current) / ticks
}

/// Where a MIDI DMX channel at `current`, moving `rate` a tick, is a tick
/// on: at `target` once there, or at once with no rate (an instant write).
pub(crate) fn dim_step(current: f64, target: f64, rate: f64) -> f64 {
    if rate > 0.0 {
        (current + rate).min(target)
    } else if rate == 0.0 {
        target
    } else {
        (current + rate).max(target)
    }
}

/// Where a channel written at `from`, moving `rate` a tick toward `target`,
/// is `ticks` ticks on: [`dim_step`] once for each whole tick, as the store
/// and the universe thread take it a tick at a time; a part tick moves it
/// no further, so every level is one the live run sends. Stepped, not
/// `from + rate × ticks`: the sum of the steps rounds otherwise, so a
/// seek's replay landed a byte off what the live run sent (#646: 0 → 35
/// over 44 ticks, tick 22, 18 for 17). An instant write (no rate, or a
/// dim over no ticks: infinite, or no number) is there at once.
pub(crate) fn dimmed_at(from: f64, target: f64, rate: f64, ticks: f64) -> f64 {
    if rate == 0.0 || !rate.is_finite() {
        return target;
    }
    let whole = ticks.floor();
    let mut value = from;
    let mut n = 0.0;
    while n < whole && value != target {
        let next = dim_step(value, target, rate);
        if next == value {
            // A rate too small to move it; it never will.
            return value;
        }
        value = next;
        n += 1.0;
    }
    value
}

/// A fixture test's channels as a universe holds them: 0-based index and
/// value, or none.
type TestChannels = Option<Vec<(usize, u8)>>;

/// The 0-based index of a 1-based DMX channel; channel 0 is taken as 1.
fn channel_index(channel: u16) -> usize {
    usize::from(channel.max(1) - 1)
}

/// A DMX universe.
pub(crate) struct Universe {
    /// The current DMX state.
    current: Arc<RwLock<Vec<f64>>>,
    /// The target DMX state.
    target: Arc<RwLock<Vec<f64>>>,
    /// The current dimming rates.
    rates: Arc<RwLock<Vec<f64>>>,
    /// The current dim global rate.
    global_dim_rate: RwLock<f64>,
    /// Max channels
    max_channels: Arc<AtomicU16>,
    /// The configuration for this universe.
    config: config::Universe,
    /// The cancel handle for the thread attached to this universe.
    cancel_handle: CancelHandle,
    /// Used to send data to the OLA client thread.
    ola_sender: Sender<DmxMessage>,
    /// A fixture test's channels (0-based index, value), laid over the frame
    /// last, as it goes out: it wins over effects and MIDI-DMX while set,
    /// and the moment it is cleared the frame underneath goes out again.
    test_override: Arc<RwLock<TestChannels>>,
    /// Bumped on every override change, so the thread sends even when the
    /// show underneath did not move.
    test_generation: Arc<AtomicU64>,
}

impl Universe {
    /// Creates a new universe.
    pub(super) fn new(
        config: config::Universe,
        cancel_handle: CancelHandle,
        ola_sender: Sender<DmxMessage>,
    ) -> Universe {
        Universe {
            rates: Arc::new(RwLock::new(vec![0.0; UNIVERSE_SIZE])),
            current: Arc::new(RwLock::new(vec![0.0; UNIVERSE_SIZE])),
            target: Arc::new(RwLock::new(vec![0.0; UNIVERSE_SIZE])),
            global_dim_rate: RwLock::new(DEFAULT_DIM_TICKS),

            max_channels: Arc::new(AtomicU16::new(0)),
            config,
            cancel_handle,
            ola_sender,
            test_override: Arc::new(RwLock::new(None)),
            test_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Lays a fixture test's channels (1-based DMX channel, value) over this
    /// universe's output, or clears it (`None`). Channels outside 1..=512
    /// are dropped.
    pub fn set_test_override(&self, channels: Option<&[(u16, u8)]>) {
        *self.test_override.write() = channels.map(|channels| {
            channels
                .iter()
                .filter(|(channel, _)| (1..=UNIVERSE_SIZE as u16).contains(channel))
                .map(|(channel, value)| (channel_index(*channel), *value))
                .collect()
        });
        self.test_generation.fetch_add(1, Ordering::SeqCst);
    }

    /// The dim rate: ticks for a full transition.
    pub fn dim_rate(&self) -> f64 {
        *self.global_dim_rate.read()
    }

    /// Sets the dim rate, in ticks for a full transition.
    pub fn set_dim_rate(&self, ticks: f64) {
        *self.global_dim_rate.write() = ticks;
    }

    #[cfg(test)]
    pub fn get_dim_speed(&self) -> f64 {
        *self.global_dim_rate.read()
    }

    #[cfg(test)]
    pub fn get_target_value(&self, channel_index: usize) -> f64 {
        self.target.read()[channel_index]
    }

    /// Every channel lit (1-based DMX channel, value) as the universe puts
    /// it out: where a dim has got to, else its value -- whoever wrote it.
    pub fn lit_channels(&self) -> Vec<(u16, u8)> {
        let target = self.target.read();
        let rates = self.rates.read();
        let current = self.current.read();
        (0..UNIVERSE_SIZE)
            .map(|idx| {
                let value = if rates[idx] != 0.0 {
                    current[idx]
                } else {
                    target[idx]
                };
                (idx, dmx_byte(value))
            })
            .filter(|&(_, value)| value > 0)
            .map(|(idx, value)| (idx as u16 + 1, value))
            .collect()
    }

    /// Updates the dim speed.
    pub fn update_dim_speed(&self, dimming: Duration) {
        *self.global_dim_rate.write() = dim_ticks(dimming);
    }

    /// A 1-based DMX channel about to be written: the universe thread's
    /// range grows to cover it, and its 0-based index comes back.
    fn touch(&self, channel: u16) -> usize {
        let _ =
            self.max_channels
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current_channel| {
                    (channel >= current_channel).then_some(channel + 1)
                });
        channel_index(channel)
    }

    /// Updates the universe with the DMX channel/value.
    pub fn update_channel_data(&self, channel: u16, value: u8, dim: bool) {
        let channel_index = self.touch(channel);
        let value = f64::from(value);
        // The rate first, with no other lock held: an assignment evaluates
        // its value before its place, so reading `current` inside it held
        // that read while waiting for `rates`, against the order the
        // universe thread takes them in (target, rates, current).
        let rate = if dim {
            dim_rate(
                self.current.read()[channel_index],
                value,
                *self.global_dim_rate.read(),
            )
        } else {
            0.0
        };
        self.target.write()[channel_index] = value;
        self.rates.write()[channel_index] = rate;
    }

    /// Puts a channel back where the MIDI writes before a playback's start
    /// point left it: at `current`, still moving toward `target` by `rate`
    /// a tick if a dim had not finished (#583). Takes the locks in the
    /// universe thread's order (target, rates, current). A value already
    /// reached leaves `current` to the thread, which sends a channel only
    /// as it moves: set there too, it would never go out.
    pub fn restore_channel_data(&self, channel: u16, current: f64, target: f64, rate: f64) {
        let channel_index = self.touch(channel);
        self.target.write()[channel_index] = target;
        self.rates.write()[channel_index] = rate;
        if rate != 0.0 {
            self.current.write()[channel_index] = current;
        }
    }

    /// Updates the universe with effect commands (bypasses dimming).
    /// Acquires locks once for the entire batch rather than per-command, and
    /// skips a channel already at the value (and not dimming toward
    /// another). It compares with the channel itself rather than with the
    /// last effect write: MIDI DMX writes channels too, and a cache cleared
    /// to 0 between songs took a new show's 0 for the old value and left the
    /// previous song's byte on the wire (#557).
    pub fn update_effect_commands(&self, commands: Vec<(u16, u8)>) {
        let mut target = self.target.write();
        let mut rates = self.rates.write();

        for (channel, value) in commands {
            let idx = channel_index(channel);
            let value_f = f64::from(value);
            if target[idx] != value_f || rates[idx] != 0.0 {
                self.touch(channel);
                target[idx] = value_f;
                rates[idx] = 0.0; // instant (no dimming for effect commands)
            }
        }
    }

    /// Starts a thread that writes the universe data to the transmitter.
    pub fn start_thread(&self) -> JoinHandle<()> {
        let rates = self.rates.clone();
        let current = self.current.clone();
        let target = self.target.clone();
        let max_channels = self.max_channels.clone();
        let cancel_handle = self.cancel_handle.clone();
        let universe = u32::from(self.config.universe());
        let ola_sender = self.ola_sender.clone();
        let test_override = self.test_override.clone();
        let test_generation = self.test_generation.clone();

        thread::spawn(move || {
            let mut clock = FrameClock::new(Instant::now());

            let mut buffer = DmxBuffer::new();
            // `buffer` is always the show's own frame (every channel); an
            // override is laid on a copy, so clearing it sends the show's
            // values again with nothing to restore.
            let mut sent_generation = 0;

            loop {
                if cancel_handle.is_cancelled() {
                    return;
                }

                let changed = Universe::approach_target(
                    &rates,
                    &current,
                    &target,
                    &max_channels,
                    &mut buffer,
                );
                let generation = test_generation.load(Ordering::SeqCst);
                if changed || generation != sent_generation {
                    sent_generation = generation;
                    let frame = Universe::compose(&buffer, test_override.read().as_deref());
                    if let Err(e) = ola_sender.send(DmxMessage {
                        universe,
                        buffer: frame,
                    }) {
                        error!(
                            err = e.to_string(),
                            "Error sending DMX packet to universe {}", universe
                        );
                    }
                }

                let due = clock.ran(Instant::now());
                spin_sleep::sleep(due.saturating_duration_since(Instant::now()));
            }
        })
    }

    /// The frame that goes out: the show's, with a fixture test's channels
    /// laid over it when one is set.
    fn compose(show: &DmxBuffer, test: Option<&[(usize, u8)]>) -> DmxBuffer {
        let mut frame = show.clone();
        for &(i, value) in test.unwrap_or_default() {
            frame.set_channel(i, value);
        }
        frame
    }

    /// Takes the given inputs and approaches the current expected DMX values.
    /// Returns true if anything changed.
    ///
    /// Lock ordering: target → rates → current.  This matches the write-side
    /// ordering in `update_effect_commands` (target → rates) and
    /// `update_channel_data` (target → rates → current) to prevent ABBA
    /// deadlocks between the universe thread and the effects loop.
    fn approach_target(
        rates: &Arc<RwLock<Vec<f64>>>,
        current: &Arc<RwLock<Vec<f64>>>,
        target: &Arc<RwLock<Vec<f64>>>,
        max_channels: &Arc<AtomicU16>,
        buffer: &mut DmxBuffer,
    ) -> bool {
        let target = target.read();
        let rates = rates.read();
        let mut current = current.write();

        let mut changed = false;
        for i in 0..usize::from(max_channels.load(Ordering::Relaxed)) {
            // We want current == target, but due to floating points we'll test if they're close to each other.
            if (current[i] - target[i]).abs() > f64::EPSILON {
                changed = true;
                current[i] = dim_step(current[i], target[i], rates[i]);
                buffer.set_channel(i, dmx_byte(current[i]));
            }
        }
        changed
    }
}

#[cfg(test)]
mod frame_clock_test {
    use super::*;

    /// Frames a loop on `clock` runs in `secs`, each waking `late` after
    /// it is due (the sleep's overshoot) and taking `work`.
    fn frames_in(secs: u64, late: impl Fn(u64) -> Duration, work: Duration) -> u64 {
        let start = Instant::now();
        let mut clock = FrameClock::new(start);
        let end = start + Duration::from_secs(secs);
        let mut frames = 0;
        let mut now = start;
        loop {
            now = now.max(clock.due() + late(frames));
            if now >= end {
                return frames;
            }
            frames += 1;
            now += work;
            clock.ran(now);
        }
    }

    #[test]
    fn a_loop_on_the_clock_runs_at_the_rate_whatever_its_frames_take() {
        // Restarting the count from each frame ran the effects loop at
        // about 43 Hz: every wake's lateness carried into the next frame.
        for secs in [1, 10, 600] {
            for (late, work) in [
                (Duration::ZERO, Duration::ZERO),
                (Duration::from_micros(1_100), Duration::from_micros(300)),
                (Duration::from_millis(5), Duration::from_millis(10)),
            ] {
                let frames = frames_in(secs, |_| late, work);
                assert!(
                    frames.abs_diff(FRAMES_PER_SECOND * secs) <= 1,
                    "{frames} frames in {secs} s, {late:?} late, {work:?} work"
                );
            }
            // Jittered wakes too.
            let frames = frames_in(
                secs,
                |k| Duration::from_micros((k * 7_919) % 3_000),
                Duration::from_micros(200),
            );
            assert!(frames.abs_diff(FRAMES_PER_SECOND * secs) <= 1, "{frames}");
        }
    }

    #[test]
    fn a_stalled_loop_goes_on_a_frame_later_rather_than_catch_up() {
        let start = Instant::now();
        let mut clock = FrameClock::new(start);
        clock.ran(start);
        let stalled = start + Duration::from_secs(1);
        let due = clock.ran(stalled);
        assert_eq!(due, stalled + frame_time(1));
        // Late by less than a frame, it keeps its deadlines.
        let late = due + frame_time(1) / 2;
        assert_eq!(clock.ran(late), stalled + frame_time(2));
    }

    #[test]
    fn frame_times_span_whole_seconds_exactly() {
        assert_eq!(frame_time(0), Duration::ZERO);
        assert_eq!(frame_time(44), Duration::from_secs(1));
        assert_eq!(frame_time(44 * 3_600), Duration::from_secs(3_600));
        assert_eq!(frame_time(1), Duration::from_nanos(22_727_272));
    }
}

#[cfg(test)]
mod dimming_test {
    use super::*;

    /// The byte a channel's value goes out as, as the universe thread
    /// always sent it; what `lit_channels` reports is the same byte.
    #[test]
    fn dmx_byte_rounds_within_a_byte() {
        assert_eq!(dmx_byte(0.0), 0);
        assert_eq!(dmx_byte(-0.4), 0);
        assert_eq!(dmx_byte(-7.0), 0);
        assert_eq!(dmx_byte(127.5), 128);
        assert_eq!(dmx_byte(254.6), 255);
        assert_eq!(dmx_byte(255.4), 255);
        assert_eq!(dmx_byte(300.0), 255);
        assert_eq!(dmx_byte(f64::NAN), 0);
    }

    /// A tick at a time lands exactly where [`dimmed_at`] says, whatever
    /// the rate: the store and the universe thread step, a replay jumps
    /// there. (At the write itself `dimmed_at` has an instant write landed
    /// already; the step lands it a tick on.)
    #[test]
    fn stepping_a_dim_tick_by_tick_agrees_with_its_closed_form() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut random = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..2000 {
            let from = (random() * 255.0).round();
            let target = (random() * 255.0).round();
            let ticks = match (random() * 4.0) as u8 {
                0 => 0.0,
                1 => 1.0,
                2 => (random() * 10.0).round() + 1.0,
                _ => random() * 4400.0 + 1.0,
            };
            let rate = dim_rate(from, target, ticks);
            let steps = (random() * 80.0) as u32;
            let mut current = from;
            for n in 1..=steps {
                current = dim_step(current, target, rate);
                let closed = dimmed_at(from, target, rate, f64::from(n));
                assert_eq!(
                    current, closed,
                    "from {from} to {target} over {ticks}: tick {n}"
                );
            }
        }
    }

    /// #646: 0 → 35 over 44 ticks, stepped, is 17.499999999999996 at tick
    /// 22, which goes out as 17; `from + rate × ticks` made it 17.5, 18.
    /// The replay and the live run send the same byte at every tick, and a
    /// part tick is where its whole tick left it: no level the live run
    /// never sends.
    #[test]
    fn a_replayed_dim_sends_the_bytes_the_live_run_sent() {
        let rate = dim_rate(0.0, 35.0, 44.0);
        let mut current = 0.0;
        for n in 1..=50 {
            current = dim_step(current, 35.0, rate);
            let replayed = dimmed_at(0.0, 35.0, rate, f64::from(n));
            assert_eq!(dmx_byte(replayed), dmx_byte(current), "tick {n}");
        }
        assert_eq!(dmx_byte(dimmed_at(0.0, 35.0, rate, 22.0)), 17);
        assert_eq!(
            dimmed_at(0.0, 35.0, rate, 21.5),
            dimmed_at(0.0, 35.0, rate, 21.0)
        );
        assert_eq!(dimmed_at(0.0, 35.0, rate, 1e6), 35.0);
    }

    #[test]
    fn a_dim_of_no_ticks_is_instant_both_ways() {
        let rate = dim_rate(10.0, 200.0, 0.0);
        assert_eq!(dim_step(10.0, 200.0, rate), 200.0);
        assert_eq!(dimmed_at(10.0, 200.0, rate, 0.0), 200.0);
        let stay = dim_rate(50.0, 50.0, 0.0);
        assert_eq!(dim_step(50.0, 50.0, stay), 50.0);
        assert_eq!(dimmed_at(50.0, 50.0, stay, 3.0), 50.0);
    }

    #[test]
    fn dimming_time_to_ticks() {
        assert_eq!(dim_ticks(Duration::ZERO), 1.0);
        assert_eq!(dim_ticks(Duration::from_secs(2)), 2.0 * TARGET_HZ);
    }
}

#[cfg(test)]
mod test {
    use std::{
        error::Error,
        sync::mpsc::{self, Receiver},
        thread,
        time::Duration,
    };

    use ola::DmxBuffer;

    use crate::{
        config,
        dmx::{engine::DmxMessage, universe::TARGET_HZ},
        playsync::CancelHandle,
    };

    use super::Universe;

    fn new_universe() -> (Universe, Receiver<DmxMessage>) {
        let (sender, receiver) = mpsc::channel();
        (
            Universe::new(
                config::Universe::new(1, "universe".to_string()),
                CancelHandle::new(),
                sender,
            ),
            receiver,
        )
    }

    #[test]
    fn lit_channels_reads_a_dim_where_it_has_got_to() {
        let (universe, _receiver) = new_universe();
        universe.update_effect_commands(vec![(3, 200)]);
        universe.update_dim_speed(Duration::from_secs(1));
        // Dimming up from 0, not yet on its way: not lit, though its target
        // is 200.
        universe.update_channel_data(10, 200, true);
        assert_eq!(universe.lit_channels(), vec![(3, 200)]);
    }

    #[test]
    fn a_restored_channel_goes_out() -> Result<(), Box<dyn Error>> {
        // #583: a value the MIDI before a seek point had reached was
        // written as reached, so the thread, which sends what moves, never
        // sent it.
        let (universe, receiver) = new_universe();
        let handle = universe.start_thread();
        universe.restore_channel_data(1, 254.0, 254.0, 0.0);
        // And a dim on its way, half there.
        universe.restore_channel_data(2, 100.0, 200.0, 1.0);
        let mut frame = Vec::new();
        for _ in 0..50 {
            let message = receiver.recv_timeout(crate::testutil::PATIENCE)?;
            frame = message.buffer.as_slice()[0..2].to_vec();
            if frame[0] == 254 && frame[1] > 100 {
                break;
            }
        }
        universe.cancel_handle.cancel();
        handle
            .join()
            .map_err(|_| "Error waiting for join".to_string())?;
        assert_eq!(frame[0], 254, "the restored value never went out");
        assert!(frame[1] > 100 && frame[1] < 200, "the dim: {frame:?}");
        Ok(())
    }

    #[test]
    fn test_thread() -> Result<(), Box<dyn Error>> {
        // We just want to make sure that the thread vaguely does what we think it should.
        let (universe, receiver) = new_universe();

        let receiver_handle = thread::spawn(move || receiver.recv());

        let handle = universe.start_thread();
        universe.update_channel_data(0, 0, false);
        universe.update_channel_data(1, 50, false);

        let result = receiver_handle
            .join()
            .map_err(|_| "Error waiting for join".to_string())??;

        assert_eq!([50u8, 0u8], result.buffer.as_slice()[0..2]);

        universe.cancel_handle.cancel();
        handle
            .join()
            .map_err(|_| "Error waiting for join".to_string())?;

        Ok(())
    }

    #[test]
    fn a_test_override_wins_over_the_show_and_releases_back_to_it() -> Result<(), Box<dyn Error>> {
        let (universe, receiver) = new_universe();
        let handle = universe.start_thread();
        // The show: channel 1 at 50, channel 3 at 7.
        universe.update_effect_commands(vec![(1, 50), (3, 7)]);
        let next = |want: &dyn Fn(&[u8]) -> bool| -> Result<Vec<u8>, Box<dyn Error>> {
            loop {
                let message = receiver.recv_timeout(crate::testutil::PATIENCE)?;
                let frame = message.buffer.as_slice()[0..4].to_vec();
                if want(&frame) {
                    return Ok(frame);
                }
            }
        };
        assert_eq!(next(&|f| f[0] == 50)?, vec![50, 0, 7, 0]);

        // A test on channels 1-2 (and one past the universe, dropped).
        universe.set_test_override(Some(&[(1, 255), (2, 128), (600, 1)]));
        assert_eq!(next(&|f| f[0] == 255)?, vec![255, 128, 7, 0]);

        // The show moves underneath; the test still wins where it is.
        universe.update_effect_commands(vec![(1, 60), (3, 9)]);
        assert_eq!(next(&|f| f[2] == 9)?, vec![255, 128, 9, 0]);

        // Released: the show's own values go out at once.
        universe.set_test_override(None);
        assert_eq!(next(&|f| f[0] == 60)?, vec![60, 0, 9, 0]);

        universe.cancel_handle.cancel();
        handle
            .join()
            .map_err(|_| "Error waiting for join".to_string())?;
        Ok(())
    }

    #[test]
    fn test_no_dimming() {
        let (universe, _) = new_universe();

        universe.update_channel_data(1, 0, true);
        universe.update_channel_data(1, 50, true);
        universe.update_channel_data(2, 100, true);
        universe.update_channel_data(3, 150, true);
        universe.update_channel_data(4, 200, true);

        let mut buffer = DmxBuffer::new();

        Universe::approach_target(
            &universe.rates,
            &universe.current,
            &universe.target,
            &universe.max_channels,
            &mut buffer,
        );

        assert_eq!([50u8, 100u8, 150u8, 200u8, 0u8], buffer.as_slice()[0..5]);
    }

    #[test]
    fn test_ignore_dimming() {
        let (universe, _) = new_universe();

        // Dim over 2 seconds. This will be ignored.
        universe.update_dim_speed(Duration::from_secs(2));

        universe.update_channel_data(1, 0, false);
        universe.update_channel_data(2, 50, false);
        universe.update_channel_data(3, 100, false);
        universe.update_channel_data(4, 150, false);
        universe.update_channel_data(5, 200, false);

        let mut buffer = DmxBuffer::new();

        Universe::approach_target(
            &universe.rates,
            &universe.current,
            &universe.target,
            &universe.max_channels,
            &mut buffer,
        );

        assert_eq!([0u8, 50u8, 100u8, 150u8, 200u8], buffer.as_slice()[0..5]);

        // We found a bug with dimming back down, so let's test that here.
        universe.update_channel_data(2, 50u8, false);
        universe.update_channel_data(3, 200u8, false);
        universe.update_channel_data(4, 0, false);

        Universe::approach_target(
            &universe.rates,
            &universe.current,
            &universe.target,
            &universe.max_channels,
            &mut buffer,
        );

        assert_eq!([0u8, 50u8, 200u8, 0u8, 200u8], buffer.as_slice()[0..5]);
    }

    #[test]
    fn test_dimming_over_two_seconds() {
        let (universe, _) = new_universe();

        // Dim over 2 seconds.
        universe.update_dim_speed(Duration::from_secs(2));

        universe.update_channel_data(1, 0, true);
        universe.update_channel_data(2, 50, true);
        universe.update_channel_data(3, 100, true);
        universe.update_channel_data(4, 150, true);
        universe.update_channel_data(5, 200, true);

        let mut buffer = DmxBuffer::new();

        // There are TARGET_HZ updates per second.
        for _ in 0..(TARGET_HZ as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After one second, we should be halfway there.
        // Channel 1 (index 0): 0 -> 0, Channel 2 (index 1): 0 -> 25, Channel 3 (index 2): 0 -> 50, etc.
        assert_eq!([0u8, 25u8, 50u8, 75u8, 100u8], buffer.as_slice()[0..5]);

        for _ in 0..(TARGET_HZ as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After two seconds, we should be all the way there.
        assert_eq!([0u8, 50u8, 100u8, 150u8, 200u8], buffer.as_slice()[0..5]);

        for _ in 0..(TARGET_HZ as usize) {
            assert!(!Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After another second, nothing should have changed.
        assert_eq!([0u8, 50u8, 100u8, 150u8, 200u8], buffer.as_slice()[0..5]);
    }

    #[test]
    fn test_separate_dimming() {
        let (universe, _) = new_universe();

        // Dim over 1 second.
        universe.update_dim_speed(Duration::from_secs(1));
        universe.update_channel_data(1, 100, true);

        let mut buffer = DmxBuffer::new();

        // Progress one tick.
        let _ = Universe::approach_target(
            &universe.rates,
            &universe.current,
            &universe.target,
            &universe.max_channels,
            &mut buffer,
        );

        // Dim over 2 seconds.
        universe.update_dim_speed(Duration::from_secs(2));

        // The two channels should dim at different rates.
        universe.update_channel_data(1, 100, true);

        // There are TARGET_HZ updates per second.
        for _ in 0..(TARGET_HZ as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After one second (+ 1 tick), channel 1 (index 0) should be at ~51
        // (started at 0, +1 tick at 1s rate = ~2, then +44 ticks at 2s rate = ~51)
        assert!(buffer.as_slice()[0] >= 50 && buffer.as_slice()[0] <= 52);

        for _ in 0..(TARGET_HZ as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After two seconds (+ 1 tick), we should be all the way there.
        assert_eq!([100u8], buffer.as_slice()[0..1]);
    }

    #[test]
    fn test_dimming_override() {
        let (universe, _) = new_universe();

        // Dim over 1 second.
        universe.update_dim_speed(Duration::from_secs(1));
        universe.update_channel_data(1, 100, true);

        let mut buffer = DmxBuffer::new();

        // There are TARGET_HZ updates per second.
        for _ in 0..((TARGET_HZ / 2.0) as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After half of a second, we should be halfway there.
        assert_eq!([50u8], buffer.as_slice()[0..1]);

        // Dim over 2 seconds and update the channel data again.
        universe.update_dim_speed(Duration::from_secs(2));
        universe.update_channel_data(1, 100, true);

        for _ in 0..(TARGET_HZ as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After 1.5 seconds, we should be halfway with the new dimming speed.
        assert_eq!([75u8], buffer.as_slice()[0..1]);

        for _ in 0..(TARGET_HZ as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After 2.5 seconds, we should be all the way there.
        assert_eq!([100u8], buffer.as_slice()[0..1]);
    }

    #[test]
    fn test_update_dim_speed_zero_duration() {
        let (universe, _) = new_universe();

        // Zero duration should set global_dim_rate to 1.0 (instant)
        universe.update_dim_speed(Duration::ZERO);
        assert_eq!(universe.get_dim_speed(), 1.0);
    }

    #[test]
    fn test_update_dim_speed_one_second() {
        let (universe, _) = new_universe();

        universe.update_dim_speed(Duration::from_secs(1));
        // 1.0 second * 44.0 Hz = 44.0
        assert_eq!(universe.get_dim_speed(), TARGET_HZ);
    }

    #[test]
    fn test_update_channel_data_channel_zero() {
        let (universe, _) = new_universe();

        // Channel 0 should map to index 0
        universe.update_channel_data(0, 100, false);
        assert_eq!(universe.get_target_value(0), 100.0);
    }

    #[test]
    fn test_update_effect_commands_channel_zero() {
        let (universe, _) = new_universe();

        // Channel 0 edge case — should map to index 0
        let commands = vec![(0u16, 128u8)];
        universe.update_effect_commands(commands);
        assert_eq!(universe.get_target_value(0), 128.0);
    }

    #[test]
    fn test_update_effect_commands_basic() {
        let (universe, _) = new_universe();

        let commands = vec![(1u16, 200u8), (2, 100), (3, 50)];
        universe.update_effect_commands(commands);

        assert_eq!(universe.get_target_value(0), 200.0);
        assert_eq!(universe.get_target_value(1), 100.0);
        assert_eq!(universe.get_target_value(2), 50.0);
    }

    #[test]
    fn test_update_effect_commands_deduplicates() {
        let (universe, _) = new_universe();

        // First batch: set values
        let commands = vec![(1u16, 200u8), (2, 100)];
        universe.update_effect_commands(commands);

        // Second batch with same values: should be a no-op
        let commands = vec![(1u16, 200u8), (2, 100)];
        universe.update_effect_commands(commands);

        // Values should still be the same
        assert_eq!(universe.get_target_value(0), 200.0);
        assert_eq!(universe.get_target_value(1), 100.0);
    }

    #[test]
    fn an_effect_writes_zero_over_another_writers_value() {
        // #557: a channel some other writer left lit -- the last song's
        // effects, before a song change; MIDI DMX -- is set to 0 when an
        // effect writes 0, not taken for unchanged.
        let (universe, _) = new_universe();
        universe.update_effect_commands(vec![(3u16, 226u8)]);
        universe.update_channel_data(4, 90, false);
        universe.update_effect_commands(vec![(3u16, 0u8), (4, 0)]);
        assert_eq!(universe.get_target_value(2), 0.0);
        assert_eq!(universe.get_target_value(3), 0.0);
    }

    #[test]
    fn an_effect_write_stops_a_dim_in_progress() {
        // At the value but still dimming toward it: the effect snaps it.
        let (universe, _) = new_universe();
        universe.update_effect_commands(vec![(1u16, 100u8)]);
        universe.update_channel_data(1, 100, true);
        universe.update_effect_commands(vec![(1u16, 100u8)]);
        assert_eq!(universe.rates.read()[0], 0.0);
    }

    #[test]
    fn test_approach_target_no_change_returns_false() {
        let (universe, _) = new_universe();

        // Don't set any targets, so everything is at default (0.0)
        // Ensure max_channels covers at least 1 channel
        universe.update_channel_data(1, 0, false);

        // First approach brings current to target
        let mut buffer = DmxBuffer::new();
        Universe::approach_target(
            &universe.rates,
            &universe.current,
            &universe.target,
            &universe.max_channels,
            &mut buffer,
        );

        // Second approach should return false (no change)
        let changed = Universe::approach_target(
            &universe.rates,
            &universe.current,
            &universe.target,
            &universe.max_channels,
            &mut buffer,
        );
        assert!(!changed);
    }

    #[test]
    fn test_dimming_down_from_higher_to_lower() {
        let (universe, _) = new_universe();

        // Dim over 1 second.
        universe.update_dim_speed(Duration::from_secs(1));

        // First, dim up to 200
        universe.update_channel_data(1, 200, true);

        let mut buffer = DmxBuffer::new();

        // There are TARGET_HZ updates per second.
        for _ in 0..(TARGET_HZ as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After one second, we should be at 200.
        assert_eq!([200u8], buffer.as_slice()[0..1]);

        // Now dim down from 200 to 100
        universe.update_channel_data(1, 100, true);

        // After half a second, we should be halfway (at 150)
        for _ in 0..((TARGET_HZ / 2.0) as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // Should be around 150 (halfway between 200 and 100)
        let value = buffer.as_slice()[0];
        assert!(
            (148..=152).contains(&value),
            "Expected ~150 when dimming down from 200 to 100, got {}",
            value
        );

        // Continue for another half second to reach target
        for _ in 0..((TARGET_HZ / 2.0) as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After one second total, we should be at 100.
        assert_eq!([100u8], buffer.as_slice()[0..1]);
    }
}
