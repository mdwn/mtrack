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
//! The device's beat clock: its engine, and the schedule a song plays on
//! it.

use std::{
    error::Error,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

use midir::{MidiOutput, MidiOutputPort};
use midly::live::LiveEvent;
use tracing::{debug, info, warn};

use crate::{clock::PlaybackClock, config, playsync::CancelHandle};

use super::MidiSender;

/// Serializes a MIDI System Real-Time event to bytes.
pub(super) fn realtime_bytes(msg: midly::live::SystemRealtime) -> Vec<u8> {
    let event = LiveEvent::Realtime(msg);
    let mut buf = Vec::with_capacity(1);
    event
        .write_std(&mut buf)
        .expect("realtime events are always valid");
    buf
}

/// Runs the beat clock on a MIDI sender, sending Start (or Song Position
/// Pointer and Continue), timing clocks, and Stop.
///
/// Gear following the clock counts its pulses from Start, or from the
/// position a Song Position Pointer gives before a Continue; the pointer
/// counts MIDI beats (16th notes, six pulses each), so a clock starting part
/// way in goes on from the MIDI beat at the start (within half a pulse after
/// it), or the next, and the pulses between are not sent (see [`locate`]).
/// One starting there a MIDI beat or more late goes on at the next MIDI beat
/// still to come rather than sending the pulses it missed in a burst; at the
/// top of the song it sends Start and the pulses it missed, as gear counts
/// from there.
///
/// Where a loop goes back, the sections' [`config::BeatClockSeams`] say what
/// it does: run straight on through the seam (the default), or relocate the
/// gear there as at a start, Stop first.
///
/// Returns the spacing between the final two clock ticks it actually delivered —
/// i.e. the instantaneous tempo at the point playback ended (natural end or
/// cancellation). Returns `None` if fewer than two ticks were sent, in which case
/// the tempo can't be determined. Callers use this to keep the clock free-running
/// at the last known tempo between songs.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_beat_clock(
    sender: &mut dyn MidiSender,
    ticks: &[Duration],
    start_time: Duration,
    sections: Option<&BeatClockSections>,
    reference: Option<&ReferenceClock>,
    cancel_handle: &CancelHandle,
    clock: &PlaybackClock,
    rx: &mpsc::Receiver<BeatClockCommand>,
) -> ScheduleOutcome {
    use midly::live::SystemRealtime;

    let clock_bytes = realtime_bytes(SystemRealtime::TimingClock);

    // Track the spacing between consecutive delivered ticks so we can report the
    // final tempo when we stop.
    let mut prev_tick: Option<Duration> = None;
    let mut last_interval: Option<Duration> = None;

    let mut schedule = PassTicks::new(ticks, start_time);
    // How far section loops have put the song back: a tick at song time `t`
    // falls due `t + looped - start_time` into the schedule.
    let mut looped = Duration::ZERO;
    let mut discipline = Discipline::default();
    let mut follow = SectionFollow::new(sections);
    // A section with no pulse in it, whose jump came due a hair before the
    // last pulse ahead of it: the song is put back once that pulse is out.
    let mut deferred: Option<crate::section_loop::SectionJump> = None;
    // The gap across a seam, or a jump into a section with no pulse in it,
    // is no tempo: a stop just past it would hold that as the tempo to keep.
    let mut seam = false;

    let relocate =
        sections.is_some_and(|sections| sections.seams == config::BeatClockSeams::Relocate);

    // Start (with any pulses already due, sent at once), or the play-from
    // point's MIDI beat and Continue.
    let first = match midi_beat_at(ticks, start_time) {
        0 => 0,
        first => resume_at(ticks, first, clock.elapsed(), &|tick| {
            discipline.steer((tick + looped).saturating_sub(start_time))
        }),
    };
    schedule.resume(first);
    // The MIDI spec asks for a millisecond between Start (or Continue) and
    // the first clock, which with a pulse due at once would follow it back to
    // back.
    let mut transport_sent = Some(locate(sender, first, false));

    'ticks: loop {
        let Some(tick_time) = schedule.next() else {
            // Out of ticks with a section still going round (its end at or
            // past the last of them): its jump is still to come.
            loop {
                if !follow.pending(ticks) {
                    break 'ticks;
                }
                if let Some(outcome) =
                    schedule_ends(sender, cancel_handle, false, Some(rx), last_interval)
                {
                    return outcome;
                }
                // A jump committed is taken at once, however far ahead it
                // falls: no tick is left before it, and the schedule then
                // sleeps to its first tick as to any other. Waiting for it to
                // fall due polled the next pass's first pulse, up to a poll
                // late.
                if let Some(jump) = follow.due(Duration::MAX) {
                    if lands_in(ticks, &jump) {
                        let relocated;
                        (looped, relocated) = take_jump(
                            sender,
                            &mut schedule,
                            &jump,
                            start_time,
                            &discipline,
                            clock,
                            relocate,
                        );
                        transport_sent = relocated.or(transport_sent);
                        seam = true;
                        continue 'ticks;
                    }
                    // Nothing to go back to, and no pulse left to put back.
                    continue;
                }
                thread::sleep(Duration::from_millis(5));
            }
        };
        if let Some(jump) = deferred.take() {
            looped = jump.at.saturating_sub(jump.to);
            seam = true;
        }
        // A loop's jump due by this tick takes the song back, as it takes the
        // notes and audio: the tick due there is the start's. Due within half
        // a pulse of it, too: a loop's length is whole samples, so a loop of
        // whole pulses can come out a hair long, which put the pass's last
        // pulse microseconds before the next's first -- a pulse too many at
        // every seam.
        let half_pulse = schedule.pulse() / 2;
        while let Some(jump) = follow.due(tick_time + looped + half_pulse) {
            if lands_in(ticks, &jump) {
                let relocated;
                (looped, relocated) = take_jump(
                    sender,
                    &mut schedule,
                    &jump,
                    start_time,
                    &discipline,
                    clock,
                    relocate,
                );
                transport_sent = relocated.or(transport_sent);
                seam = true;
                continue 'ticks;
            }
            // A section with no pulse in it, shorter than a pulse: nothing to
            // go back to, but the song is put back all the same, and this
            // tick, past it, falls due as much later (or the next does, this
            // one lying before it).
            if tick_time < jump.from {
                deferred = Some(jump);
            } else {
                looped = jump.at.saturating_sub(jump.to);
                seam = true;
                schedule.put_back();
                continue 'ticks;
            }
        }
        // A broken loop ends the clock, as it ends the notes: the player
        // stops the song with it.
        //
        // A newer command supersedes this schedule. Nothing cancels the handle
        // when a song ends naturally -- the cleanup path only clears the join
        // and advances the playlist -- so a schedule that outlasts its audio
        // (a MIDI file longer than the song) would otherwise keep running,
        // dequeue the next song late, and shift its Start and every tick by the
        // residual. This is also how Shutdown gets observed mid-schedule, so
        // dropping the device does not block for the rest of the song.
        let broken = sections.is_some_and(|sections| sections.loop_break.load(Ordering::Relaxed));
        if let Some(outcome) = schedule_ends(sender, cancel_handle, broken, Some(rx), last_interval)
        {
            return outcome;
        }
        if std::mem::take(&mut seam) {
            prev_tick = None;
        }

        // Due on the wall clock, steered toward the playback clock the notes
        // and audio follow, so a long loop does not drift off them.
        let target_wall = discipline.steer((tick_time + looped).saturating_sub(start_time));
        let elapsed = clock.elapsed();
        if target_wall > elapsed {
            discipline.sleep_until(target_wall, clock, reference);
        }
        // A jump the audio committed to while this waited: the tick due at it
        // is the section start's.
        while let Some(jump) = follow.due(tick_time + looped + half_pulse) {
            if lands_in(ticks, &jump) {
                let relocated;
                (looped, relocated) = take_jump(
                    sender,
                    &mut schedule,
                    &jump,
                    start_time,
                    &discipline,
                    clock,
                    relocate,
                );
                transport_sent = relocated.or(transport_sent);
                seam = true;
                continue 'ticks;
            }
            if tick_time < jump.from {
                deferred = Some(jump);
            } else {
                looped = jump.at.saturating_sub(jump.to);
                seam = true;
                schedule.put_back();
                continue 'ticks;
            }
        }

        if let Some(outcome) = schedule_ends(sender, cancel_handle, false, None, last_interval) {
            return outcome;
        }

        if let Some(sent) = transport_sent.take() {
            let since = sent.elapsed();
            if since < START_TO_CLOCK {
                spin_sleep::sleep(START_TO_CLOCK - since);
            }
        }
        if let Err(e) = sender.send(&clock_bytes) {
            debug!("MIDI beat clock send failed: {:?}", e);
        }

        let due = tick_time + looped;
        if let Some(prev) = prev_tick {
            last_interval = Some(due.saturating_sub(prev));
        }
        prev_tick = Some(due);
    }

    // Send STOP when finished
    let _ = sender.send(&realtime_bytes(SystemRealtime::Stop));
    ScheduleOutcome::Finished(last_interval)
}

/// Whether a schedule ends before its last tick: stopped (cancelled, or
/// `broken`, its loop broken), or, given the engine's channel, superseded by
/// a newer command there (or the channel closed: the engine is going away).
/// If it ends, it sends Stop, and gives how it ended.
fn schedule_ends(
    sender: &mut dyn MidiSender,
    cancel_handle: &CancelHandle,
    broken: bool,
    rx: Option<&mpsc::Receiver<BeatClockCommand>>,
    last_interval: Option<Duration>,
) -> Option<ScheduleOutcome> {
    let outcome = if broken || cancel_handle.is_cancelled() {
        ScheduleOutcome::Finished(last_interval)
    } else {
        match rx?.try_recv() {
            Ok(command) => ScheduleOutcome::Preempted(command, last_interval),
            Err(mpsc::TryRecvError::Disconnected) => ScheduleOutcome::Finished(last_interval),
            Err(mpsc::TryRecvError::Empty) => return None,
        }
    };
    let _ = sender.send(&realtime_bytes(midly::live::SystemRealtime::Stop));
    Some(outcome)
}

/// Pulses to a MIDI beat (a 16th note): what Song Position Pointer counts.
pub(super) const PULSES_PER_MIDI_BEAT: usize = 6;

/// The furthest a Song Position Pointer reaches: 14 bits of MIDI beats
/// (4,096 quarter notes, about 34 minutes at 120 BPM).
pub(super) const MAX_SONG_POSITION: usize = 0x3FFF;

/// A Song Position Pointer to `midi_beats` into the song. One past its reach
/// is clamped to the furthest it gives: gear following it is put there, and
/// counts on from there.
pub(super) fn song_position_bytes(midi_beats: usize) -> [u8; 3] {
    let position = midi_beats.min(MAX_SONG_POSITION);
    [0xF2, (position & 0x7F) as u8, (position >> 7) as u8]
}

/// Tells the gear following the clock where it goes on from: pulse `index`
/// of the schedule, the first of a MIDI beat. Start at the top of the song;
/// otherwise a Song Position Pointer there, and Continue. Stopped first if it
/// is `playing`, as the spec has the pointer sent only to stopped gear. When
/// it was sent, for the millisecond the first clock waits after it.
fn locate(sender: &mut dyn MidiSender, index: usize, playing: bool) -> std::time::Instant {
    use midly::live::SystemRealtime;
    let mut send = |bytes: &[u8]| {
        if let Err(e) = sender.send(bytes) {
            debug!("MIDI beat clock transport send failed: {:?}", e);
        }
    };
    if playing {
        send(&realtime_bytes(SystemRealtime::Stop));
    }
    if index == 0 {
        send(&realtime_bytes(SystemRealtime::Start));
    } else {
        send(&song_position_bytes(index / PULSES_PER_MIDI_BEAT));
        send(&realtime_bytes(SystemRealtime::Continue));
    }
    std::time::Instant::now()
}

/// Takes a loop's jump that lands on a pulse: back to the first pulse in
/// the stretch it goes back over, the clock running straight on through the
/// seam. Relocating, to its target's MIDI beat instead (or the next still to
/// come, for a jump taken late), the gear stopped and put there: unless that
/// lies at or past the jump's end (a section with no MIDI beat in it), when
/// it runs through as well. How far the song is then put back, and when the
/// gear was told, if it was.
#[allow(clippy::too_many_arguments)]
fn take_jump(
    sender: &mut dyn MidiSender,
    schedule: &mut PassTicks<'_>,
    jump: &crate::section_loop::SectionJump,
    start_time: Duration,
    discipline: &Discipline,
    clock: &PlaybackClock,
    relocate: bool,
) -> (Duration, Option<std::time::Instant>) {
    let looped = schedule.go_back(jump);
    if !relocate {
        return (looped, None);
    }
    let ticks = schedule.ticks;
    let index = resume_at(
        ticks,
        midi_beat_at(ticks, jump.to),
        clock.elapsed(),
        &|tick| discipline.steer((tick + looped).saturating_sub(start_time)),
    );
    if index >= ticks.partition_point(|tick| *tick < jump.from) {
        return (looped, None);
    }
    schedule.resume(index);
    (looped, Some(locate(sender, index, true)))
}

/// The first pulse of the MIDI beat at song time `at`: the first at or after
/// it, or one up to half a pulse before it (a section's start, say, taken
/// from a beat grid, a hair after the pulse on its downbeat, which is rounded
/// to the microsecond).
pub(super) fn midi_beat_at(ticks: &[Duration], at: Duration) -> usize {
    let index = ticks.partition_point(|tick| *tick < at);
    let pulse = match (
        index.checked_sub(1).and_then(|i| ticks.get(i)),
        ticks.get(index),
    ) {
        (Some(before), Some(after)) => after.saturating_sub(*before),
        _ => ticks
            .get(1)
            .zip(ticks.first())
            .map_or(Duration::ZERO, |(second, first)| {
                second.saturating_sub(*first)
            }),
    };
    let index = ticks.partition_point(|tick| *tick + pulse / 2 < at);
    index.div_ceil(PULSES_PER_MIDI_BEAT) * PULSES_PER_MIDI_BEAT
}

/// Whether the clock, at `now`, is a MIDI beat or more behind pulse `index`:
/// the pulse a MIDI beat after it is already due (`due` gives when a pulse at
/// a song time falls due).
pub(super) fn late(
    ticks: &[Duration],
    index: usize,
    now: Duration,
    due: &dyn Fn(Duration) -> Duration,
) -> bool {
    ticks
        .get(index + PULSES_PER_MIDI_BEAT)
        .is_some_and(|tick| due(*tick) <= now)
}

/// The pulse to go on from: `index`, the first of a MIDI beat, or, if that
/// is a MIDI beat or more behind at `now`, the first of the next still to
/// come. Past the last pulse, the MIDI beat after it.
fn resume_at(
    ticks: &[Duration],
    mut index: usize,
    now: Duration,
    due: &dyn Fn(Duration) -> Duration,
) -> usize {
    if !late(ticks, index, now, due) {
        return index;
    }
    while ticks.get(index).is_some_and(|tick| due(*tick) < now) {
        index += PULSES_PER_MIDI_BEAT;
    }
    index
}

/// The least time between a Start or Continue and the first clock after it.
pub(super) const START_TO_CLOCK: Duration = Duration::from_millis(1);

/// When each of a schedule's ticks falls due, as time on the song's timeline
/// from a start at `start` (so due `time - start` after it). A loop's jumps,
/// sections and a looping song's passes alike, take it back
/// ([`PassTicks::go_back`]): it keeps no passes of its own, but follows the
/// playback's log, as the notes and lights do.
#[derive(Clone)]
pub(super) struct PassTicks<'a> {
    ticks: &'a [Duration],
    /// Ticks before this song time are skipped: the start, until a seek.
    skip_before: Duration,
    /// Its next tick.
    index: usize,
}

impl<'a> PassTicks<'a> {
    pub(super) fn new(ticks: &'a [Duration], start: Duration) -> Self {
        PassTicks {
            ticks,
            skip_before: start,
            index: 0,
        }
    }
}

impl PassTicks<'_> {
    /// Goes back (or on) to the first tick at or after `position`, for a
    /// loop: what comes next is played however early it lies in the song.
    pub(super) fn seek(&mut self, position: Duration) {
        self.index = self.ticks.partition_point(|tick| *tick < position);
        self.skip_before = Duration::ZERO;
    }

    /// Takes a loop's jump: back to its target, giving how far the song is
    /// then put back (`looped`), so that the target falls due at the jump's
    /// time. A pass's overlap is no matter to the clock: its pulses switch to
    /// the new pass at the jump.
    fn go_back(&mut self, jump: &crate::section_loop::SectionJump) -> Duration {
        self.seek(jump.to);
        jump.at.saturating_sub(jump.to)
    }

    /// Goes on from tick `index`, however early or late in the song.
    fn resume(&mut self, index: usize) {
        self.index = index;
        self.skip_before = Duration::ZERO;
    }

    /// Gives the tick it last gave again.
    fn put_back(&mut self) {
        self.index = self.index.saturating_sub(1);
    }

    /// The gap from the tick last given to the next (or, at the end, from the
    /// one before it).
    fn pulse(&self) -> Duration {
        let index = self.index.max(1);
        match (self.ticks.get(index - 1), self.ticks.get(index)) {
            (Some(last), Some(next)) => next.saturating_sub(*last),
            (Some(last), None) if index >= 2 => last.saturating_sub(self.ticks[index - 2]),
            _ => Duration::ZERO,
        }
    }
}

impl Iterator for PassTicks<'_> {
    type Item = Duration;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let tick = *self.ticks.get(self.index)?;
            self.index += 1;
            if tick >= self.skip_before {
                return Some(tick);
            }
        }
    }
}

/// How a schedule ended, and the tempo it established.
pub(super) enum ScheduleOutcome {
    /// Played to its end, or stopped by cancellation.
    Finished(Option<Duration>),
    /// Superseded by a command that arrived mid-schedule. The command has been
    /// taken off the channel and must be acted on by the caller.
    Preempted(BeatClockCommand, Option<Duration>),
}

impl ScheduleOutcome {
    /// The tempo this schedule established, if it delivered enough ticks to
    /// determine one.
    pub(super) fn interval(&self) -> Option<Duration> {
        match self {
            ScheduleOutcome::Finished(interval) => *interval,
            ScheduleOutcome::Preempted(_, interval) => *interval,
        }
    }
}

/// A song's beat clock schedule, submitted to the [`BeatClockEngine`].
pub(super) struct BeatClockPlay {
    /// Absolute tick timestamps for the song (from the song's tempo map).
    pub(super) ticks: Arc<Vec<Duration>>,
    /// Position in the tick timeline to begin from (for seeks/section starts).
    pub(super) start_time: Duration,
    /// Delay before the first clock is sent, matching note playback.
    pub(super) playback_delay: Duration,
    /// Cancellation for this song.
    pub(super) cancel: CancelHandle,
    /// The shared playback clock whose `start()` is the "go" signal. The engine
    /// waits on it (cancel-aware) so its `Start` lands with the first note.
    pub(super) clock: PlaybackClock,
    /// The song's section loop, which the clock follows as its notes do.
    pub(super) sections: Option<BeatClockSections>,
    /// Set once it has sent Start (or Continue), for the notes to wait on.
    pub(super) transport: Option<Arc<AtomicBool>>,
}

/// A song's section loop, which the clock follows as its notes and lights do:
/// the player's handles, and the playback's log of the jumps committed to.
#[derive(Clone)]
pub(super) struct BeatClockSections {
    pub(super) active_section: Arc<parking_lot::RwLock<Option<crate::player::SectionBounds>>>,
    pub(super) section_loop_break: Arc<AtomicBool>,
    pub(super) jumps: Arc<crate::playsync::SectionJumpLog>,
    /// The song's loop broken: the clock ends, as the notes do.
    pub(super) loop_break: Arc<AtomicBool>,
    /// Whether the song loops as a whole: out of ticks, its next pass is to
    /// come while the log is open.
    pub(super) loops: bool,
    /// What the clock does where a loop goes back.
    pub(super) seams: config::BeatClockSeams,
}

impl BeatClockSections {
    /// The section looping now: armed, and not being broken out of.
    fn looping(&self) -> Option<crate::player::SectionBounds> {
        if self.section_loop_break.load(Ordering::Relaxed) {
            return None;
        }
        self.active_section.read().clone()
    }
}

/// A schedule's way through the playback's section loop jumps.
struct SectionFollow<'a> {
    sections: Option<&'a BeatClockSections>,
    cursor: Option<crate::playsync::JumpCursor>,
}

impl<'a> SectionFollow<'a> {
    fn new(sections: Option<&'a BeatClockSections>) -> Self {
        SectionFollow {
            sections,
            cursor: sections
                .map(|sections| crate::playsync::JumpCursor::new(sections.jumps.clone())),
        }
    }

    /// The jump falling due by `at` on the playback timeline, if one does.
    fn due(&mut self, at: Duration) -> Option<crate::section_loop::SectionJump> {
        self.cursor.as_mut()?.due(at)
    }

    /// Whether a jump the schedule could follow may yet come: one committed,
    /// or, while the log is open, a looping song's next pass, or a section
    /// looping with a tick in it to go back to.
    fn pending(&self, ticks: &[Duration]) -> bool {
        let Some(cursor) = self.cursor.as_ref() else {
            return false;
        };
        if cursor.peek().is_some() {
            return true;
        }
        if cursor.log().is_closed() {
            return false;
        }
        self.sections
            .is_some_and(|sections| sections.loops && !sections.loop_break.load(Ordering::Relaxed))
            || self
                .sections
                .and_then(BeatClockSections::looping)
                .is_some_and(|section| {
                    lands_in(
                        ticks,
                        &crate::section_loop::SectionJump {
                            at: Duration::ZERO,
                            from: section.end_time,
                            to: section.start_time,
                            overlap: Duration::ZERO,
                            pass: false,
                        },
                    )
                })
    }
}

/// Whether a jump lands on a tick: one in the stretch it goes back over. A
/// section with none in it (shorter than a pulse, between two) is not
/// followed: going back to it would find no tick to play, for ever.
fn lands_in(ticks: &[Duration], jump: &crate::section_loop::SectionJump) -> bool {
    let index = ticks.partition_point(|tick| *tick < jump.to);
    ticks.get(index).is_some_and(|tick| *tick < jump.from)
}

/// The playback clock the notes and audio follow (on a rig with audio, the
/// interface's sample counter), and the playback delay between its zero and
/// the schedule's.
pub(super) struct ReferenceClock {
    clock: PlaybackClock,
    delay: Duration,
}

impl ReferenceClock {
    /// How far ahead of `wall` it reads, in seconds, for a schedule timed on
    /// `wall`.
    fn offset_from(&self, wall: &PlaybackClock) -> f64 {
        let reference = self.clock.elapsed().as_secs_f64() - self.delay.as_secs_f64();
        reference - wall.elapsed().as_secs_f64()
    }
}

/// Steers a schedule timed on the wall clock toward the playback clock, which
/// on a rig with audio counts the interface's samples and so runs at its
/// crystal's rate, not the computer's: tens of ppm apart, a few ms a minute,
/// which a loop played for minutes builds up against the audio.
///
/// The playback clock moves only once an audio buffer, so a reading of it
/// lags by up to a buffer; the most it reads ahead over a window is a fair
/// measure. The first window sets where it started, and from then on the
/// schedule follows how far that has moved, a little at a time so no tick
/// jumps: at most [`Discipline::MAX_STEP`] a window, 250ppm. That keeps the
/// tempo gear sees steady, but an audio device whose clock is further off
/// (one resampling, say) leaves the beat clock drifting from the audio; the
/// notes follow it at its own rate (see [`NoteClock`]).
///
/// [`NoteClock`]: super::notes::NoteClock
#[derive(Default)]
pub(super) struct Discipline {
    window_start: Option<Duration>,
    window_max: Option<f64>,
    baseline: Option<f64>,
    pub(super) correction: f64,
    /// For where in each wait a reading is taken (xorshift; 0 until seeded).
    rng: u64,
}

impl Discipline {
    const WINDOW: Duration = Duration::from_secs(2);
    /// The most the correction moves a window: 0.5ms in 2s, 250ppm.
    pub(super) const MAX_STEP: f64 = 0.0005;

    /// A pseudo-random fraction in [0, 1): where in a wait the next reading
    /// is taken.
    pub(super) fn next_fraction(&mut self) -> f64 {
        if self.rng == 0 {
            self.rng = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0x9E37_79B9_7F4A_7C15, |since| since.as_nanos() as u64)
                | 1;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Sleeps until `target` on `wall`, taking a reading of `reference` at a
    /// random point on the way. Readings taken at the ticks themselves fell
    /// at one place in the playback clock's steps for a tempo whose pulse is
    /// a whole number of audio buffers (117.1875 BPM at 48kHz), so the most
    /// a window read was not the top of a step, and wandered as the two
    /// drifted: the steering then put the clock off by up to a buffer.
    fn sleep_until(
        &mut self,
        target: Duration,
        wall: &PlaybackClock,
        reference: Option<&ReferenceClock>,
    ) {
        let now = wall.elapsed();
        if target <= now {
            return;
        }
        let Some(reference) = reference else {
            spin_sleep::sleep(target - now);
            return;
        };
        spin_sleep::sleep((target - now).mul_f64(self.next_fraction()));
        self.observe(wall.elapsed(), reference.offset_from(wall));
        let now = wall.elapsed();
        if target > now {
            spin_sleep::sleep(target - now);
        }
    }

    /// Takes a reading, at `wall` time into the schedule, of how far ahead of
    /// it the playback clock reads.
    pub(super) fn observe(&mut self, wall: Duration, ahead: f64) {
        if !ahead.is_finite() {
            return;
        }
        self.window_max = Some(self.window_max.map_or(ahead, |max| max.max(ahead)));
        let window_start = *self.window_start.get_or_insert(wall);
        if wall.saturating_sub(window_start) < Self::WINDOW {
            return;
        }
        let measured = self.window_max.take().unwrap_or(ahead);
        self.window_start = Some(wall);
        match self.baseline {
            None => self.baseline = Some(measured),
            Some(baseline) => {
                let drift = measured - baseline;
                self.correction += (drift - self.correction).clamp(-Self::MAX_STEP, Self::MAX_STEP);
            }
        }
    }

    /// When a tick due `due` into the schedule goes out on the wall clock:
    /// as much earlier as the playback clock has run ahead of it, or later as
    /// it has fallen behind.
    fn steer(&self, due: Duration) -> Duration {
        let correction = Duration::from_secs_f64(self.correction.abs());
        if self.correction >= 0.0 {
            due.saturating_sub(correction)
        } else {
            due + correction
        }
    }
}

/// A command sent to the beat clock engine thread.
pub(super) enum BeatClockCommand {
    /// Play a song's schedule, then hold its final tempo.
    Play(BeatClockPlay),
    /// Terminate the engine thread.
    Shutdown,
}

/// A device-owned, always-on MIDI beat clock.
///
/// A single long-lived thread owns the clock output for the device's lifetime.
/// Songs don't spawn their own clock threads — they submit a [`BeatClockPlay`]
/// and the engine plays it (see [`run_beat_clock`]), then, if `persist_tempo` is
/// set, keeps emitting timing clocks (0xF8) at the song's final tempo so
/// downstream gear holds its tempo until the next song retunes the engine. With
/// `persist_tempo` off it stays silent between songs, matching the original
/// send-Start/clocks/Stop-then-silent behavior.
pub(super) struct BeatClockEngine {
    pub(super) tx: mpsc::Sender<BeatClockCommand>,
    pub(super) join: Mutex<Option<thread::JoinHandle<()>>>,
    /// The tempo it holds between songs, as the engine thread last set it.
    pub(super) tempo: Arc<Mutex<Option<Duration>>>,
}

impl BeatClockEngine {
    /// Spawns the engine thread, which opens its own output connection to `port`
    /// and waits for schedules, holding `held` until one sets another tempo.
    pub(super) fn new(port: MidiOutputPort, persist_tempo: bool, held: Option<Duration>) -> Self {
        let (tx, rx) = mpsc::channel::<BeatClockCommand>();
        let tempo = Arc::new(Mutex::new(held));
        let engine_tempo = tempo.clone();
        let join = thread::spawn(move || {
            let output = match MidiOutput::new("mtrack beat clock output") {
                Ok(output) => output,
                Err(e) => {
                    warn!("Unable to create beat clock output: {:?}", e);
                    return;
                }
            };
            let mut connection = match output.connect(&port, "mtrack beat clock") {
                Ok(connection) => connection,
                Err(e) => {
                    warn!("Unable to connect beat clock output: {:?}", e);
                    return;
                }
            };

            // Elevate to real-time thread priority to minimize scheduling jitter
            // on clock tick delivery.
            promote_to_realtime_thread();

            beat_clock_engine_loop(&mut connection, persist_tempo, &engine_tempo, &rx);
        });

        BeatClockEngine {
            tx,
            join: Mutex::new(Some(join)),
            tempo,
        }
    }

    /// The tempo it holds between songs: the last a song set, if any.
    pub(super) fn held_tempo(&self) -> Option<Duration> {
        *self.tempo.lock().expect("beat clock tempo lock")
    }

    /// Stops the engine thread and waits for it to end. The tempo it held
    /// stays readable.
    pub(super) fn stop(&self) {
        let _ = self.tx.send(BeatClockCommand::Shutdown);
        if let Some(handle) = self.join.lock().expect("beat clock engine lock").take() {
            let _ = handle.join();
        }
    }

    /// Submits a song's schedule to the engine.
    pub(super) fn play(&self, play: BeatClockPlay) {
        let _ = self.tx.send(BeatClockCommand::Play(play));
    }
}

impl Drop for BeatClockEngine {
    fn drop(&mut self) {
        let _ = self.tx.send(BeatClockCommand::Shutdown);
        if let Some(handle) = self.join.lock().expect("beat clock engine lock").take() {
            let _ = handle.join();
        }
    }
}

/// The engine's main loop: alternately plays a submitted schedule and (when
/// `persist_tempo` is set) free-runs the last tempo until the next schedule
/// arrives. `tempo` holds that tempo, from before the engine started too: a
/// device opened afresh goes on with the one its predecessor held.
pub(super) fn beat_clock_engine_loop(
    sender: &mut dyn MidiSender,
    persist_tempo: bool,
    tempo: &Mutex<Option<Duration>>,
    rx: &mpsc::Receiver<BeatClockCommand>,
) {
    // The inter-tick interval of the last schedule we played, used to hold tempo.
    let mut last_interval: Option<Duration> = *tempo.lock().expect("beat clock tempo lock");
    // The live free-run. Kept across the whole idle-to-startup stretch so its
    // tick phase is continuous: rebuilding it at each handover throws away the
    // part of the interval already elapsed and delays the next tick.
    let mut hold: Option<TempoHold> = None;
    // A command taken off the channel by a schedule it interrupted, waiting to
    // be acted on in place of the next receive.
    let mut pending: Option<BeatClockCommand> = None;

    loop {
        let command = match pending.take() {
            Some(command) => command,
            None => match next_command(sender, persist_tempo, last_interval, &mut hold, rx) {
                Some(command) => command,
                // Sender dropped: the device is going away.
                None => return,
            },
        };

        let play = match command {
            BeatClockCommand::Play(play) => play,
            BeatClockCommand::Shutdown => return,
        };

        // The tempo to hold through this song's startup, if there is one.
        let held = last_interval.filter(|_| persist_tempo);

        // Wait for the shared "go" signal so our Start lands with the first
        // note, holding the tempo across the wait rather than going silent for
        // however long the rest of the player takes to get ready. Cancel-aware,
        // so an early stop can't wedge the engine.
        let interrupted = wait_for_start_holding_tempo(
            sender,
            &play,
            held.map(|interval| hold_at(&mut hold, interval)),
            rx,
            &mut pending,
        );
        if interrupted || play.cancel.is_cancelled() {
            continue;
        }
        let started_at = play
            .clock
            .started_at()
            .unwrap_or_else(std::time::Instant::now);

        // Sleep the playback delay, matching the note thread -- still holding
        // the tempo, for the same reason as above, and hearing a command as
        // the wait for the start does: a stop for a reopen joins this thread.
        if !play.playback_delay.is_zero() {
            const COMMAND_CHECK: Duration = Duration::from_millis(50);
            let mut hold = held.map(|interval| hold_at(&mut hold, interval));
            // From the playback clock's start, as the notes count it, not
            // from when this thread noticed it (#600).
            let delay = PlaybackClock::wall_from(started_at);
            let mut interrupted = false;
            while delay.elapsed() < play.playback_delay && !play.cancel.is_cancelled() {
                let left = play.playback_delay.saturating_sub(delay.elapsed());
                let step = hold
                    .as_ref()
                    .map_or(COMMAND_CHECK, |hold| hold.until_next().min(COMMAND_CHECK));
                spin_sleep::sleep(step.min(left));
                if heard_command(rx, &mut pending) {
                    interrupted = true;
                    break;
                }
                if let Some(hold) = hold.as_deref_mut() {
                    hold.tick_if_due(sender);
                }
            }
            if interrupted {
                continue;
            }
        }
        if play.cancel.is_cancelled() {
            continue;
        }

        // The schedule owns the clock from here; the next idle stretch starts a
        // fresh hold from whatever tempo this song establishes.
        hold = None;

        // Use a dedicated wall clock for tick timing instead of the audio-derived
        // playback clock: the audio clock only updates once per buffer callback
        // (~23ms at 1024/44.1kHz), which would make the beat clock's spin_sleep
        // overshoot by variable amounts. A wall clock is smooth and sub-ms.
        //
        // Its zero is the playback clock's start plus the delay: the notes'
        // zero, not wherever this thread got to after noticing the start.
        let wall = PlaybackClock::wall_from(started_at + play.playback_delay);

        let reference = ReferenceClock {
            clock: play.clock.clone(),
            delay: play.playback_delay,
        };
        let mut announcing = Announcing {
            inner: &mut *sender,
            transport: play.transport.as_deref(),
        };
        let outcome = run_beat_clock(
            &mut announcing,
            &play.ticks,
            play.start_time,
            play.sections.as_ref(),
            Some(&reference),
            &play.cancel,
            &wall,
            rx,
        );

        // Only replace the held tempo when this schedule actually established
        // one. `run_beat_clock` reports `None` when it delivered fewer than two
        // ticks -- a stop within ~42ms at 120 BPM, or a seek leaving one tick to
        // play -- and overwriting with that dropped a tempo the engine was
        // successfully holding, silencing gear mid-set for no reason. Keep the
        // last tempo we actually know.
        if let Some(interval) = outcome.interval() {
            last_interval = Some(interval);
            *tempo.lock().expect("beat clock tempo lock") = last_interval;
        }
        if let ScheduleOutcome::Preempted(command, _) = outcome {
            pending = Some(command);
        }
    }
}

/// A beat clock's sender that tells the notes once it has sent Start or
/// Continue.
pub(super) struct Announcing<'a> {
    pub(super) inner: &'a mut dyn MidiSender,
    pub(super) transport: Option<&'a AtomicBool>,
}

impl MidiSender for Announcing<'_> {
    fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
        let sent = self.inner.send(bytes);
        if matches!(bytes, [0xFA] | [0xFB]) {
            if let Some(transport) = self.transport {
                transport.store(true, Ordering::Release);
            }
        }
        sent
    }
}

/// Returns the next command to act on. If persistence is enabled and a tempo is
/// known, free-runs the clock at that tempo while waiting; otherwise blocks
/// silently. Returns `None` if the sender is dropped.
pub(super) fn next_command(
    sender: &mut dyn MidiSender,
    persist_tempo: bool,
    last_interval: Option<Duration>,
    hold: &mut Option<TempoHold>,
    rx: &mpsc::Receiver<BeatClockCommand>,
) -> Option<BeatClockCommand> {
    // A schedule may already be queued (e.g. a rapid stop→play); take it now so we
    // don't emit an idle tempo blip between back-to-back songs.
    match rx.try_recv() {
        Ok(command) => return Some(command),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => return None,
        Err(std::sync::mpsc::TryRecvError::Empty) => {}
    }

    match (persist_tempo, last_interval) {
        (true, Some(interval)) => run_idle_until_command(sender, hold_at(hold, interval), rx),
        // Not persisting (or no tempo yet): wait silently for the next schedule.
        _ => {
            *hold = None;
            rx.recv().ok()
        }
    }
}

/// Paces free-running timing clocks at a fixed interval against a wall clock,
/// so a held tempo neither drifts nor bursts.
pub(super) struct TempoHold {
    clock: PlaybackClock,
    interval: Duration,
    /// When the next tick is due, measured from `clock`'s start.
    next_tick: Duration,
}

impl TempoHold {
    pub(super) fn new(interval: Duration) -> TempoHold {
        let clock = PlaybackClock::wall();
        clock.start();
        TempoHold {
            clock,
            interval,
            next_tick: interval,
        }
    }

    /// How long until the next tick is due, for use as a wait timeout.
    fn until_next(&self) -> Duration {
        self.until_next_at(self.clock.elapsed())
    }

    /// [`TempoHold::until_next`] against a supplied elapsed time.
    ///
    /// The pacing decisions are split from the clock that feeds them so they can
    /// be tested by handing over the times to reason about, with no threads and
    /// no sleeping. Timing-derived tests of this logic were flaky on a loaded
    /// runner and proved nothing a table of elapsed values does not.
    pub(super) fn until_next_at(&self, elapsed: Duration) -> Duration {
        self.next_tick.saturating_sub(elapsed)
    }

    /// Emits a tick if one is due.
    ///
    /// When more than a full interval late, the schedule is re-anchored to now
    /// instead of the backlog being replayed. A thread that loses the CPU (or a
    /// host that suspends) can come back seconds behind, and repaying that debt
    /// means firing every missed tick back to back with no spacing -- ten
    /// seconds at 120 BPM is ~480 clocks in a burst, which downstream gear
    /// reads as an enormous tempo spike. A held tempo is a tempo, not a debt.
    fn tick_if_due(&mut self, sender: &mut dyn MidiSender) {
        self.tick_if_due_at(sender, self.clock.elapsed());
    }

    /// [`TempoHold::tick_if_due`] against a supplied elapsed time. Returns
    /// whether a tick was emitted.
    pub(super) fn tick_if_due_at(
        &mut self,
        sender: &mut dyn MidiSender,
        elapsed: Duration,
    ) -> bool {
        use midly::live::SystemRealtime;

        if elapsed < self.next_tick {
            return false;
        }
        if let Err(e) = sender.send(&realtime_bytes(SystemRealtime::TimingClock)) {
            debug!("MIDI idle beat clock send failed: {:?}", e);
        }
        self.next_tick = if elapsed.saturating_sub(self.next_tick) > self.interval {
            elapsed + self.interval
        } else {
            self.next_tick + self.interval
        };
        true
    }
}

/// Returns a hold running at `interval`, reusing `hold` if it already is.
///
/// Reused rather than rebuilt so the tick phase survives the handover from
/// waiting-for-a-command to waiting-for-the-go-signal. A fresh hold restarts the
/// count from now, which discards however much of the current interval had
/// already elapsed and pushes the next tick up to a full interval late -- worth
/// two missing pulses at the song boundary, measured, for no reason.
fn hold_at(hold: &mut Option<TempoHold>, interval: Duration) -> &mut TempoHold {
    match hold {
        Some(existing) if existing.interval == interval => {}
        _ => *hold = Some(TempoHold::new(interval)),
    }
    hold.as_mut().expect("just populated")
}

/// Free-runs the clock, emitting timing clocks until a command arrives
/// (returned) or the sender is dropped (`None`). Timing is anchored to a wall
/// clock so the tempo does not drift.
pub(super) fn run_idle_until_command(
    sender: &mut dyn MidiSender,
    hold: &mut TempoHold,
    rx: &mpsc::Receiver<BeatClockCommand>,
) -> Option<BeatClockCommand> {
    use std::sync::mpsc::RecvTimeoutError;

    loop {
        match rx.recv_timeout(hold.until_next()) {
            Ok(command) => return Some(command),
            Err(RecvTimeoutError::Timeout) => hold.tick_if_due(sender),
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// Waits for the shared playback clock's go signal, holding the tempo while it
/// waits.
///
/// The engine takes a queued `Play` as soon as it arrives, which ends the idle
/// free-run, and then has to wait for every subsystem -- audio device, sample
/// loading, DMX validation -- to report ready. Blocking silently through that
/// window drops the clock for exactly as long as startup takes, at the one
/// boundary `persist_tempo` is supposed to cover. So the wait is done in
/// tick-sized slices with the tempo still running underneath it.
///
/// Without a tempo to hold (persistence off, or no song has established one
/// yet) it sends nothing while it waits. Either way it waits in slices of at
/// most 50 ms, taking any command off the channel between them: a command
/// gives the wait up (see [`heard_command`]). True if one did.
fn wait_for_start_holding_tempo(
    sender: &mut dyn MidiSender,
    play: &BeatClockPlay,
    mut hold: Option<&mut TempoHold>,
    rx: &mpsc::Receiver<BeatClockCommand>,
    pending: &mut Option<BeatClockCommand>,
) -> bool {
    // A command arriving while the start is awaited gives the wait up for
    // it, as one arriving mid-schedule does: a shutdown must not wait on a
    // start that may never come -- stopping the clock for a reopen joins
    // this thread -- and a newer song replaces this one.
    const COMMAND_CHECK: Duration = Duration::from_millis(50);
    loop {
        let wait = hold
            .as_ref()
            .map_or(COMMAND_CHECK, |hold| hold.until_next().min(COMMAND_CHECK));
        if play
            .clock
            .wait_for_start_or_cancel_until(&play.cancel, wait)
        {
            return false;
        }
        if play.cancel.is_cancelled() {
            return false;
        }
        if heard_command(rx, pending) {
            return true;
        }
        if let Some(hold) = hold.as_deref_mut() {
            hold.tick_if_due(sender);
        }
    }
}

/// Takes a command off the channel for the engine loop to act on in place of
/// the next receive, if one has come; a channel closed counts as a shutdown,
/// as it does there. True if there was one.
fn heard_command(
    rx: &mpsc::Receiver<BeatClockCommand>,
    pending: &mut Option<BeatClockCommand>,
) -> bool {
    match rx.try_recv() {
        Ok(command) => *pending = Some(command),
        Err(mpsc::TryRecvError::Disconnected) => *pending = Some(BeatClockCommand::Shutdown),
        Err(mpsc::TryRecvError::Empty) => return false,
    }
    true
}

/// Promotes the current thread to real-time priority for low-jitter MIDI clock output.
///
/// Uses the shared thread priority utility that sets a high crossplatform priority
/// and, on Unix systems (Linux and macOS), requests SCHED_FIFO real-time scheduling.
/// Failures are logged but not fatal — the beat clock will still work, just with
/// potentially more jitter from OS thread scheduling.
fn promote_to_realtime_thread() {
    use crate::thread_priority::{callback_thread_priority, promote_to_realtime, rt_audio_enabled};

    let priority = callback_thread_priority();
    let rt_enabled = rt_audio_enabled();
    let mut priority_set = false;

    promote_to_realtime(priority, rt_enabled, &mut priority_set);

    if priority_set {
        info!("Elevated MIDI beat clock thread priority");
    }
}
