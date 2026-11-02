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
//! A song's notes: played over their streams, on the clock the audio
//! keeps (see [`NoteClock`]).

use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use midly::live::LiveEvent;
use tracing::debug;

use crate::{clock::PlaybackClock, playsync::CancelHandle};

use super::MidiSender;

/// Serializes a MIDI event to bytes, filtering out excluded channels.
/// Returns `Some(bytes)` if the event should be sent, `None` if it should be skipped.
pub(super) fn serialize_midi_event(
    event: &crate::midi::playback::TimedMidiEvent,
    exclude_channels: &HashSet<u8>,
    buf: &mut Vec<u8>,
) -> Option<Vec<u8>> {
    if !exclude_channels.is_empty() && exclude_channels.contains(&event.channel) {
        return None;
    }
    let live_event = midly::live::LiveEvent::Midi {
        channel: event.channel.into(),
        message: event.message,
    };
    buf.clear();
    if live_event.write_std(&mut *buf).is_ok() {
        Some(buf.clone())
    } else {
        None
    }
}

/// Parameters for MIDI playback synchronization and timing.
pub(super) struct PlaybackContext<'a> {
    pub(super) precomputed: &'a crate::midi::playback::PrecomputedMidi,
    pub(super) start_time: Duration,
    pub(super) playback_delay: Duration,
    pub(super) cancel_handle: &'a CancelHandle,
    pub(super) ready_tx: crate::playsync::ReadyGuard,
    pub(super) finished: Arc<AtomicBool>,
    pub(super) exclude_channels: &'a HashSet<u8>,
    pub(super) clock: &'a PlaybackClock,
    pub(super) loop_break: Arc<AtomicBool>,
    pub(super) active_section: Arc<parking_lot::RwLock<Option<crate::player::SectionBounds>>>,
    /// The playback's section loop jumps, which the notes follow.
    pub(super) section_jumps: Arc<crate::playsync::SectionJumpLog>,
    /// Set once the song's beat clock has sent Start (or Continue), if it
    /// has one: the notes wait for it, so none goes out before it.
    pub(super) transport: Option<Arc<AtomicBool>>,
}

/// Runs the MIDI playback thread body: signals readiness, waits for the clock
/// to start, then plays events, the first the playback delay in.
pub(super) fn run_playback(sender: &mut dyn MidiSender, mut ctx: PlaybackContext<'_>) {
    ctx.ready_tx.send();

    // Wait for the clock to start (the "go" signal from play_files).
    ctx.clock.wait_for_start_or_cancel(ctx.cancel_handle);

    // The beat clock engine (if any) keys off this same shared clock's start
    // signal, so it begins in sync with us without an explicit rendezvous.

    if ctx.cancel_handle.is_cancelled() {
        ctx.finished.store(true, Ordering::Relaxed);
        ctx.cancel_handle.notify();
        return;
    }
    let started_at = ctx
        .clock
        .started_at()
        .unwrap_or_else(std::time::Instant::now);

    // The song plays from where it starts, and on from each loop jump
    // committed to -- by the audio, or the playback's own publisher without
    // it -- sections and a looping song's passes alike: the notes go back
    // where and when the audio and lights do, the pass a jump leaves playing
    // on under the next for its overlap, and play on once a loop is broken.
    // While the log is open they follow it, even past their last note: a
    // section armed then, or one looping past it, plays them again (#606).
    //
    // Kept to the playback clock (see NoteClock), the playback delay
    // included: that is waited on it too, from the clock's start, so the
    // first notes come when the audio reaches the delay.
    let time = RealTime {
        wall: PlaybackClock::wall_from(started_at),
        // A wall clock's readings need no reading between.
        reference: ctx.clock.is_audio().then_some(ctx.clock),
    };
    let mut jumps = crate::playsync::JumpCursor::new(ctx.section_jumps.clone());
    play_streams(
        &NoteStreams {
            precomputed: ctx.precomputed,
            start_time: ctx.start_time,
            cancel_handle: ctx.cancel_handle,
            loop_break: &ctx.loop_break,
            active_section: &ctx.active_section,
            exclude_channels: ctx.exclude_channels,
            time: &time,
            delay: ctx.playback_delay,
            transport: ctx.transport.as_deref(),
        },
        &mut jumps,
        sender,
    );

    ctx.finished.store(true, Ordering::Relaxed);
    ctx.cancel_handle.notify();
}

/// What [`play_streams`] plays.
pub(super) struct NoteStreams<'a> {
    pub(super) precomputed: &'a crate::midi::playback::PrecomputedMidi,
    /// Where in the song playback began; the jumps are timed on the
    /// playback timeline, which starts here.
    pub(super) start_time: Duration,
    pub(super) cancel_handle: &'a CancelHandle,
    pub(super) loop_break: &'a AtomicBool,
    pub(super) active_section: &'a parking_lot::RwLock<Option<crate::player::SectionBounds>>,
    pub(super) exclude_channels: &'a HashSet<u8>,
    /// The time the notes keep to (see [`NoteClock`]).
    pub(super) time: &'a dyn NoteTime,
    /// The playback delay: the notes' zero, on that clock.
    pub(super) delay: Duration,
    /// Set once the song's beat clock has sent Start (or Continue), if it
    /// has one: the first note waits for it.
    pub(super) transport: Option<&'a AtomicBool>,
}

/// Where [`NoteClock`] reads the time, and sleeps: the wall clock and the
/// playback clock, or, in a test, a time of its own.
pub(super) trait NoteTime {
    /// The wall clock, in seconds from the playback clock's start.
    fn wall(&self) -> f64;
    /// The playback clock's reading, in seconds, if it is an audio device's
    /// (else the notes keep to the wall alone).
    fn reading(&self) -> Option<f64>;
    /// Sleeps `length` on the wall clock: to the moment, if a note is due
    /// then (`precise`), or as an ordinary sleep, to read or to wait.
    fn sleep(&self, length: Duration, precise: bool);
}

/// The time on a rig.
pub(super) struct RealTime<'a> {
    /// The wall clock from the playback clock's start.
    pub(super) wall: PlaybackClock,
    /// The playback clock, if it is an audio device's.
    pub(super) reference: Option<&'a PlaybackClock>,
}

impl NoteTime for RealTime<'_> {
    fn wall(&self) -> f64 {
        self.wall.elapsed().as_secs_f64()
    }

    fn reading(&self) -> Option<f64> {
        let reference = self.reference?;
        #[cfg(test)]
        NOTE_CLOCK_READS.with(|reads| reads.set(reads.get() + 1));
        Some(reference.elapsed().as_secs_f64())
    }

    fn sleep(&self, length: Duration, precise: bool) {
        if precise {
            spin_sleep::sleep(length);
        } else {
            thread::sleep(length);
        }
    }
}

/// The clock a song's notes keep to: the audio's, on the beat clock's zero.
///
/// On a rig with audio the playback clock counts the interface's samples. It
/// reads zero until the first audio buffer after the start and then moves a
/// period at a time, its callbacks coming unevenly; timed on it directly, the
/// notes took that first buffer for their zero, a buffer behind the beat
/// clock, and went out only as it moved, up to a buffer late again.
///
/// - **Calibration.** The beat clock runs on the wall clock from the
///   playback clock's start (plus the delay). Over the first half second the
///   counter's moves are set against the wall, and the offset between the two
///   taken once, so the notes and the beat clock share a zero. Until then the
///   notes run on the wall clock alone, so the first goes out with Start,
///   held only by a stall: the counter not moving for three periods (30ms
///   at least), longer than callbacks come late.
/// - **Tracking.** A delay-locked loop (Fons Adriaensen, "Using a DLL to
///   filter time", as JACK does) follows the counter's rate and phase: each
///   move, when it came (between the reads either side of it, which are
///   timed to fall just before and after each move due), is set against when
///   the loop predicted it, and both corrected by the error.
/// - **Bounds.** The clock runs no more than a period (the commonest move)
///   past the counter's reading, so an xrun or a stall makes notes at most a
///   period early; and it never goes back. A move that comes late as no
///   ordinary jitter would (half a period and more since the last, or a
///   period and a half without one) is the audio held up: the loop's phase
///   is put on it, its rate kept, so the clock goes on from where the audio
///   really is, and holds until the audio passes it. On a device whose moves
///   come that far off as a rule, they are filtered like any other.
pub(super) struct NoteClock<'a> {
    /// The wall clock and the playback clock.
    time: &'a dyn NoteTime,
    /// Whether the playback clock is an audio device's (it has been read).
    audio: bool,
    /// The last reading; when on `wall` it was last read, and when its last
    /// move was seen; the earliest the last move can have come. In seconds.
    reading: f64,
    polled: f64,
    moved_at: f64,
    earliest: f64,
    /// How often the reading has moved by each amount (in 0.1ms), for a
    /// period: the commonest.
    steps: Vec<(u32, u32)>,
    /// Calibration: when the first move came, the moves since (as when
    /// seen, reading), and the offset found: the counter's reading at the
    /// wall's zero.
    first: Option<f64>,
    samples: Vec<(f64, f64)>,
    offset: Option<f64>,
    /// The loop: locked since `since`; the counter read `value` at `at` on
    /// `wall`, and moves a second in `period` seconds of it. In seconds.
    locked: bool,
    since: f64,
    at: f64,
    value: f64,
    period: f64,
    /// Wall time the clock has held over stalls before the offset was
    /// measured.
    paused: f64,
    /// Counter time lost to the audio held up, for calibrating across it:
    /// the moves after it are measured as if it had not been.
    slipped: f64,
    /// Moves still to pull the phase in fast, after it was put on one.
    fresh: u32,
    /// How far off its prediction a move comes, as a rule (rms).
    spread: f64,
    /// What it last gave.
    now: f64,
}

impl<'a> NoteClock<'a> {
    /// The longest it sleeps while waiting with audio.
    const SAMPLE: Duration = Duration::from_millis(10);
    /// The loop's bandwidth: from this, narrowing with this time constant,
    /// to this.
    const BANDWIDTH_START: f64 = 3.0;
    const BANDWIDTH_TAU: f64 = 0.25;
    const BANDWIDTH: f64 = 0.5;
    /// How long the offset is measured over.
    const CALIBRATE: f64 = 0.5;
    /// A fit of the rate this near 1 over the calibration is taken for 1: a
    /// sound card's is, to a few hundred ppm, and the fit is good only to a
    /// percent or so.
    const CALIBRATE_RATE: f64 = 0.02;
    /// Before the offset is measured, how long without a move holds the
    /// clock: this many periods, and this long at least.
    const STALL_PERIODS: f64 = 3.0;
    const STALL_LEAST: f64 = 0.03;
    /// How far past a period beyond the reading the clock may run.
    const MARGIN: f64 = 0.0015;
    /// Periods without a move that make the audio held up; a move this late,
    /// or this many periods off, too; and this many times the moves' spread.
    const GAP: f64 = 1.5;
    const LATE: f64 = 0.75;
    const LATE_LEAST: f64 = 0.006;
    const OFF: f64 = 3.0;
    const SPREAD: f64 = 4.0;
    /// After the phase is put on a move: this many moves pull it in by this
    /// share of their error.
    const FRESH: u32 = 6;
    const FRESH_GAIN: f64 = 0.5;
    /// The reads timed about each move due: this long before and after.
    const BEFORE: f64 = 0.002;
    const AFTER: f64 = 0.001;
    /// A period, before one is known.
    const DEFAULT_STEP: f64 = 0.02;

    fn new(time: &'a dyn NoteTime) -> Self {
        NoteClock {
            time,
            audio: false,
            reading: 0.0,
            polled: 0.0,
            moved_at: 0.0,
            earliest: 0.0,
            steps: Vec::new(),
            first: None,
            samples: Vec::new(),
            offset: None,
            locked: false,
            since: 0.0,
            at: 0.0,
            value: 0.0,
            period: 1.0,
            paused: 0.0,
            slipped: 0.0,
            fresh: 0,
            spread: 0.001,
            now: 0.0,
        }
    }

    /// The commonest move of the reading, in counter seconds: a period.
    fn step(&self) -> f64 {
        self.steps
            .iter()
            .max_by_key(|(size, count)| (*count, std::cmp::Reverse(*size)))
            .map_or(Self::DEFAULT_STEP, |(size, _)| f64::from(*size) / 10_000.0)
    }

    /// The time now, in seconds from the playback clock's start.
    fn now(&mut self) -> f64 {
        let wall = self.time.wall();
        let Some(reading) = self.time.reading() else {
            self.now = self.now.max(wall);
            return self.now;
        };
        self.audio = true;
        if reading != self.reading {
            let moved = reading - self.reading;
            let first = self.reading == 0.0;
            if !first {
                let size = (moved * 10_000.0).round().clamp(0.0, f64::from(u32::MAX)) as u32;
                match self.steps.iter_mut().find(|(known, _)| *known == size) {
                    Some((_, count)) => *count += 1,
                    None => self.steps.push((size, 1)),
                }
            }
            // It moved since the read before: in the middle of that, or of
            // the last period, if shorter.
            let window = (wall - self.polled).min(self.step() * self.period);
            let seen = wall - window / 2.0;
            self.earliest = wall - window;
            // How long it surely went without moving.
            let gap = self.polled - self.moved_at;
            if self.offset.is_none() && !first {
                // Held over a stall until the offset is measured: from where
                // it held, not jumping on by the stall.
                self.paused += (wall - self.moved_at - self.stall_margin()).max(0.0);
            }
            self.reading = reading;
            self.moved_at = wall;
            if first {
                // That says only that the audio has begun: the loop locks
                // from the next.
                self.first = Some(wall);
            } else {
                self.observe(seen, reading, gap);
                if self.offset.is_none()
                    && self.locked
                    && gap <= Self::GAP * self.step() * self.period
                {
                    self.samples.push((seen, reading + self.slipped));
                }
            }
        }
        if self.offset.is_none()
            && !self.samples.is_empty()
            && self
                .first
                .is_some_and(|first| wall - first >= Self::CALIBRATE)
        {
            self.calibrate();
        }
        self.polled = wall;
        let estimate = match self.offset.filter(|_| self.locked) {
            Some(offset) => {
                let ahead = self.reading + self.step() + Self::MARGIN;
                let line = self.value + (wall - self.at) / self.period;
                line.min(ahead) - offset
            }
            // The wall clock, as the beat clock's, until the offset is
            // measured: neither the slips the loop has seen nor a bound of a
            // period on the reading, both of which on a jittery device held
            // the first notes up to a couple of periods behind the beat
            // clock's Start. Only a stall holds it: the counter not moving
            // for longer than any jitter keeps it (see `stall_margin`).
            None if self.reading > 0.0 => {
                wall.min(self.moved_at + self.stall_margin()) - self.paused
            }
            None => wall,
        };
        self.now = self.now.max(estimate);
        self.now
    }

    /// How long the counter may go without moving, before the offset is
    /// measured, before the clock holds: three periods, and 30ms at least,
    /// more than a device's callbacks come late as a rule.
    fn stall_margin(&self) -> f64 {
        (Self::STALL_PERIODS * self.step() * self.period).max(Self::STALL_LEAST)
    }

    /// Takes the offset from the moves so far: where the counter's line over
    /// them, by least squares, crosses the wall's zero.
    fn calibrate(&mut self) {
        let n = self.samples.len() as f64;
        let (mean_at, mean_value) = self.samples.iter().fold((0.0, 0.0), |(a, v), (at, value)| {
            (a + at / n, v + value / n)
        });
        let (covariance, variance) = self.samples.iter().fold((0.0, 0.0), |(c, v), (at, value)| {
            (
                c + (at - mean_at) * (value - mean_value),
                v + (at - mean_at) * (at - mean_at),
            )
        });
        let mut rate = if variance > 0.0 && self.samples.len() >= 3 {
            covariance / variance
        } else {
            1.0
        };
        if (rate - 1.0).abs() < Self::CALIBRATE_RATE {
            rate = 1.0;
        }
        self.offset = Some(mean_value - rate * mean_at);
        self.samples.clear();
    }

    /// The reading moved to `value`, which it reached about `at` on the
    /// wall, `gap` after the move before at least.
    fn observe(&mut self, at: f64, value: f64, gap: f64) {
        if !self.locked {
            self.locked = true;
            self.since = at;
            self.at = at;
            self.value = value;
            return;
        }
        let moved = value - self.value;
        let predicted = self.at + moved * self.period;
        let error = at - predicted;
        let period = self.step() * self.period;
        let held = gap > Self::GAP * period
            || error > (Self::LATE * period).max(Self::LATE_LEAST)
            || error.abs() > Self::OFF * period;
        if held && error.abs() > Self::SPREAD * self.spread || moved <= 0.0 {
            // The audio held up: the phase put on it, the rate kept -- on the
            // earliest it can have come, the next moves pulling it in.
            let at = self.earliest;
            self.slipped += (at - predicted) / self.period;
            self.at = at;
            self.value = value;
            self.fresh = Self::FRESH;
            return;
        }
        if self.fresh > 0 {
            self.fresh -= 1;
            self.at = predicted + Self::FRESH_GAIN * error;
            self.value = value;
            self.slipped += Self::FRESH_GAIN * error / self.period;
            return;
        }
        self.spread = (0.98 * self.spread * self.spread + 0.02 * error * error).sqrt();
        let bandwidth = Self::BANDWIDTH
            .max(Self::BANDWIDTH_START * (-(at - self.since) / Self::BANDWIDTH_TAU).exp());
        let omega = (2.0 * std::f64::consts::PI * bandwidth * moved * self.period).min(0.5);
        self.at = predicted + std::f64::consts::SQRT_2 * omega * error;
        self.value = value;
        self.period = (self.period + omega * omega * error / moved).clamp(0.5, 2.0);
    }

    /// When next to read the counter, to see its next move closely: just
    /// before and just after it is due (or the one after, none seen).
    fn next_read(&self, wall: f64) -> Option<f64> {
        if !self.locked {
            return None;
        }
        let period = self.step() * self.period;
        let soon = wall + 0.0002;
        let mut due = self.at + (self.reading + self.step() - self.value) * self.period;
        if due + Self::AFTER <= soon && period > 0.0 {
            due += ((soon - due - Self::AFTER) / period).ceil() * period;
        }
        [due - Self::BEFORE, due + Self::AFTER]
            .into_iter()
            .find(|at| *at > soon)
    }

    /// Sleeps for about `length` of its time: with audio, no longer than
    /// [`NoteClock::SAMPLE`], nor past the next read timed about a move.
    fn sleep(&self, length: Duration) {
        if !self.audio {
            self.time.sleep(length, true);
            return;
        }
        let wait = length.mul_f64(self.period).min(Self::SAMPLE);
        let wall = self.time.wall();
        match self
            .next_read(wall)
            .map(|at| Duration::from_secs_f64((at - wall).max(0.0)))
        {
            // Woken to read, not to send: no need to spin for it.
            Some(read) if read < wait => self.time.sleep(read, false),
            _ => self.time.sleep(wait, true),
        }
    }
}

#[cfg(test)]
thread_local! {
    /// How often a note clock on this thread has read the playback clock.
    pub(super) static NOTE_CLOCK_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One run through the song's notes: from where playback began, or from a
/// jump's target.
struct NoteStream {
    /// When the stream's song time 0 falls on the notes' clock, in seconds
    /// (negative for one that began part way in).
    origin: f64,
    /// Its next event.
    index: usize,
    /// Where in the song it stops, once a section's jump has left it: its
    /// `from`, or, playing on under the next time round, its `from` plus the
    /// overlap. A whole-song pass plays all its notes, to the end of its
    /// MIDI.
    end: Option<Duration>,
    /// The notes it has on, by channel and key, and the channels it holds
    /// the sustain pedal down on: let go if it is cut.
    held: Vec<(u8, u8)>,
    pedals: Vec<u8>,
}

/// The sustain pedal's controller.
const SUSTAIN: u8 = 64;

impl NoteStream {
    /// Notes what the stream sends of `event`, held or let go.
    fn sent(&mut self, event: &crate::midi::playback::TimedMidiEvent) {
        use midly::MidiMessage;
        let note = match event.message {
            MidiMessage::NoteOn { key, vel } if vel > 0 => Some((key.as_int(), true)),
            MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                Some((key.as_int(), false))
            }
            _ => None,
        };
        if let Some((key, on)) = note {
            self.held.retain(|&held| held != (event.channel, key));
            if on {
                self.held.push((event.channel, key));
            }
        }
        if let MidiMessage::Controller { controller, value } = event.message {
            if controller.as_int() == SUSTAIN {
                self.pedals.retain(|&channel| channel != event.channel);
                if value.as_int() >= 64 {
                    self.pedals.push(event.channel);
                }
            }
        }
    }

    /// Whether it has anything on to let go of.
    fn holds(&self) -> bool {
        !self.held.is_empty() || !self.pedals.is_empty()
    }

    /// When the stream next does something, in song time, and whether that
    /// is to let go of its notes where it is cut; `None` once it is done.
    fn next(&self, events: &[crate::midi::playback::TimedMidiEvent]) -> Option<(Duration, bool)> {
        match events.get(self.index) {
            Some(event) if self.end.is_none_or(|end| event.time < end) => Some((event.time, false)),
            _ => self.end.filter(|_| self.holds()).map(|end| (end, true)),
        }
    }

    /// Lets go of the notes the stream has on, and its sustain pedal, cut
    /// where it ends (or where playback stops).
    fn let_go(&mut self, sender: &mut dyn MidiSender) {
        let notes = self.held.drain(..).map(|(channel, key)| {
            (
                channel,
                midly::MidiMessage::NoteOff {
                    key: key.into(),
                    vel: 0.into(),
                },
            )
        });
        let pedals = self.pedals.drain(..).map(|channel| {
            (
                channel,
                midly::MidiMessage::Controller {
                    controller: SUSTAIN.into(),
                    value: 0.into(),
                },
            )
        });
        for (channel, message) in notes.chain(pedals).collect::<Vec<_>>() {
            let off = LiveEvent::Midi {
                channel: channel.into(),
                message,
            };
            let mut bytes = Vec::with_capacity(3);
            if off.write_std(&mut bytes).is_ok() {
                if let Err(e) = sender.send(&bytes) {
                    debug!("MIDI send failed: {:?}", e);
                }
            }
        }
    }
}

/// Plays the song's notes as one merged run over its streams: the first from
/// where playback began, and another from each jump's target as it falls
/// due. A jump ends the stream it leaves at its `from`, or, with an overlap
/// (a looping song's pass crossfading into the next), lets it play on to
/// `from + overlap`, its notes in time with the new stream's. Events of two
/// streams due at the same moment go older stream first. Returns on cancel
/// or loop break, or when the notes have run out and no more jumps can come.
pub(super) fn play_streams(
    params: &NoteStreams<'_>,
    jumps: &mut crate::playsync::JumpCursor,
    sender: &mut dyn MidiSender,
) {
    // Waits on the clock in steps this long while a jump may be committed.
    const JUMP_CHECK: Duration = Duration::from_millis(10);
    // Otherwise in steps still shorter than the 30ms a jump is committed
    // ahead, so one armed meanwhile is not missed.
    const IDLE_CHECK: Duration = Duration::from_millis(25);
    let events = params.precomputed.events();
    let song_start = params.start_time.as_secs_f64();
    let mut streams = vec![NoteStream {
        origin: -song_start,
        index: events.partition_point(|e| e.time < params.start_time),
        end: None,
        held: Vec::new(),
        pedals: Vec::new(),
    }];
    let mut buf = Vec::with_capacity(8);
    // The newest stream's origin, kept for a jump that comes once every
    // stream has run out of notes.
    let mut last_origin = streams[0].origin;
    let mut clock = NoteClock::new(params.time);
    let delay = params.delay.as_secs_f64();
    // The beat clock's Start (or Continue) goes before the first note: a
    // note due with it went out a hair before it, its thread waking first.
    // Not for long: a beat clock that never starts holds no note up.
    let mut transport = params.transport;

    loop {
        // Stopped, or the loop broken: nothing is left sounding.
        if params.cancel_handle.is_cancelled() || params.loop_break.load(Ordering::Relaxed) {
            for stream in &mut streams {
                stream.let_go(sender);
            }
            return;
        }
        if let Some(current) = streams.last() {
            last_origin = current.origin;
        }
        // A stream is done once out of notes, or at its end, cut there once
        // it has let go of the notes it has on. The newest, out of notes with
        // some still on, stays for a section's jump to cut.
        let newest = streams.len().saturating_sub(1);
        let mut i = 0;
        streams.retain(|stream| {
            let keep = stream.next(events).is_some()
                || (i == newest && stream.end.is_none() && stream.holds());
            i += 1;
            keep
        });
        // The earliest note due, older stream first at the same moment (to
        // the microsecond, past float rounding); `streams` is in age order.
        let next_event = streams
            .iter()
            .enumerate()
            .filter_map(|(i, stream)| {
                stream
                    .next(events)
                    .map(|(at, _)| (stream.origin + at.as_secs_f64(), i))
            })
            .min_by_key(|(at, i)| ((at * 1e6).round() as i64, *i));
        let next_jump = jumps.peek();
        // A jump falling due before the next note (or with it): the song goes
        // on from there, at its time -- on the schedule of the stream it
        // leaves, where its `from` falls, so the streams keep one grid (the
        // playback delay included) rather than each starting a poll late.
        // That is `jump.at` less the start, as the log commits jumps, but
        // counted from the stream's origin a note at `from` ties with it to
        // the bit, and is cut rather than sent: from `jump.at` the two come
        // out a rounding apart, either way, as often as not.
        let jump_at = next_jump.as_ref().map(|jump| match streams.last() {
            Some(current) => current.origin + jump.from.as_secs_f64(),
            None => last_origin + jump.from.as_secs_f64(),
        });
        let now = clock.now() - delay;
        match (next_event, jump_at) {
            (_, Some(at)) if next_event.is_none_or(|(due, _)| at <= due) => {
                if now >= at {
                    let jump = jumps.due(Duration::MAX).expect("the jump peeked at is due");
                    debug!(
                        from = ?jump.from,
                        to = ?jump.to,
                        overlap = ?jump.overlap,
                        "MIDI loop: going back"
                    );
                    if let Some(current) = streams.last() {
                        last_origin = current.origin;
                    }
                    // A section's jump cuts the stream it leaves; a pass of
                    // the whole song plays out its own notes, a note-off on
                    // or past the loop point included.
                    if let Some(current) = streams.last_mut().filter(|_| !jump.pass) {
                        let end = jump.from + jump.overlap;
                        current.end = Some(current.end.map_or(end, |e| e.min(end)));
                    }
                    streams.push(NoteStream {
                        origin: at - jump.to.as_secs_f64(),
                        index: events.partition_point(|e| e.time < jump.to),
                        end: None,
                        held: Vec::new(),
                        pedals: Vec::new(),
                    });
                    continue;
                }
                clock.sleep(Duration::from_secs_f64(at - now).min(JUMP_CHECK));
            }
            (Some((due, stream)), _) => {
                if now >= due {
                    if let Some(started) = transport.take() {
                        const TRANSPORT_WAIT: f64 = 0.02;
                        let waiting = params.time.wall();
                        while !started.load(Ordering::Acquire)
                            && params.time.wall() - waiting < TRANSPORT_WAIT
                            && !params.cancel_handle.is_cancelled()
                        {
                            params.time.sleep(Duration::from_micros(500), false);
                        }
                    }
                    let stream = &mut streams[stream];
                    if stream.next(events).is_some_and(|(_, cut)| cut) {
                        stream.let_go(sender);
                        continue;
                    }
                    let event = &events[stream.index];
                    if let Some(bytes) =
                        serialize_midi_event(event, params.exclude_channels, &mut buf)
                    {
                        if let Err(e) = sender.send(&bytes) {
                            debug!("MIDI send failed: {:?}", e);
                        }
                        stream.sent(event);
                    }
                    stream.index += 1;
                    continue;
                }
                // In short steps while a jump could come (one committed, a
                // section armed, or the log open); otherwise longer, still
                // short of the time a jump is committed ahead.
                let step = if next_jump.is_some()
                    || params.active_section.read().is_some()
                    || !jumps.log().is_closed()
                {
                    JUMP_CHECK
                } else {
                    IDLE_CHECK
                };
                clock.sleep(Duration::from_secs_f64(due - now).min(step));
            }
            // Out of notes: on while a jump may yet come, which plays them
            // again.
            (None, _) => {
                if !jumps.may_jump() {
                    return;
                }
                params.time.sleep(JUMP_CHECK / 2, false);
            }
        }
    }
}
