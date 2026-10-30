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

//! Shared section loop trigger scheduling.
//!
//! The audio, MIDI, and DMX engines all loop sections using the same
//! grid-locked trigger pattern. [`SectionLoopTrigger`] encapsulates
//! this logic so timing calculations live in one place.

use std::time::Duration;

use crate::player::SectionBounds;

/// Tracks the next trigger time for a section loop and advances it on the
/// ideal grid to prevent cumulative drift.
///
/// Each engine creates its own `SectionLoopTrigger` and calls [`check`] on
/// every poll iteration. When a transition should fire, `check` returns
/// `Some(trigger_time)` and automatically schedules the next trigger at
/// `trigger_time + section_duration` — always relative to the ideal time,
/// never to the actual elapsed time.
///
/// [`check`]: SectionLoopTrigger::check
pub struct SectionLoopTrigger {
    next_trigger: Option<Duration>,
    /// The section `next_trigger` was set for: arming another, or the same
    /// one again after a break, starts afresh from where playback is.
    armed: Option<EffectiveSection>,
    /// How much time the loops so far have put back: playback time less
    /// this is the position in the song.
    consumed: Duration,
}

/// A section loop's jump, as [`SectionLoopTrigger::jump`] gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionJump {
    /// When on the playback timeline (the start position plus the clock)
    /// the jump falls.
    pub at: Duration,
    /// Where in the song it jumps from: the section's end, or wherever
    /// playback had got to, for a section armed past its end.
    pub from: Duration,
    /// Where in the song it jumps to: the section's start.
    pub to: Duration,
    /// How long the playback it jumps from plays on under the one it jumps
    /// to: a looping song's `loop_crossfade` for a pass of the whole song
    /// (less what is left of it, for a pass begun inside it), zero for a
    /// section, or a whole song looping without one, which switch at once.
    pub overlap: Duration,
}

impl SectionJump {
    /// How much playback time the jump puts back.
    pub fn amount(&self) -> Duration {
        self.from.saturating_sub(self.to)
    }
}

/// The name the whole song goes by as a section ([`EffectiveSection`]): none
/// a song config can give one of its own.
pub const WHOLE_SONG: &str = "\u{0}whole song";

/// A looping song's passes, as they go round when no section loop is armed:
/// each starts `overlap` (its `loop_crossfade`, zero without one) before the
/// last ends, `length` after it started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WholeSongLoop {
    pub length: Duration,
    pub overlap: Duration,
}

impl WholeSongLoop {
    /// How `song` loops as a whole, `length` long (its audio's duration with
    /// audio, how long its lights play without): `None` if it does not loop.
    pub fn of(song: &crate::songs::Song, length: Duration) -> Option<WholeSongLoop> {
        song.loop_playback().then(|| WholeSongLoop {
            length,
            overlap: song.loop_crossfade().unwrap_or_default().min(length),
        })
    }
}

/// The one section a playback loops on at a time: the section loop armed,
/// or, with none (or one being broken out of), the whole song for a song
/// that loops and whose loop is not broken, starting each pass its overlap
/// before the last ends.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveSection {
    pub bounds: SectionBounds,
    /// How long the pass a jump leaves plays on under the next.
    pub overlap: Duration,
}

impl EffectiveSection {
    /// Whether this is the whole song going round rather than a section.
    pub fn is_whole_song(&self) -> bool {
        self.bounds.name == WHOLE_SONG
    }
}

/// Which section loops now: an armed section always takes precedence, so a
/// section reaching into a looping song's crossfade, or ending on its end,
/// loops on its own seam with no pass starting under it; once it is broken
/// (or with none armed), the whole song, if it loops and its loop is not
/// broken; otherwise none.
pub fn effective_section(
    armed: Option<SectionBounds>,
    section_broken: bool,
    whole_song: Option<WholeSongLoop>,
    loop_broken: bool,
) -> Option<EffectiveSection> {
    if let Some(bounds) = armed.filter(|_| !section_broken) {
        return Some(EffectiveSection {
            bounds,
            overlap: Duration::ZERO,
        });
    }
    let whole = whole_song.filter(|_| !loop_broken)?;
    let end = whole.length.saturating_sub(whole.overlap);
    (!end.is_zero()).then(|| EffectiveSection {
        bounds: SectionBounds {
            name: WHOLE_SONG.to_string(),
            start_time: Duration::ZERO,
            end_time: end,
        },
        overlap: whole.overlap,
    })
}

impl SectionLoopTrigger {
    /// Creates a new trigger with no scheduled time.
    ///
    /// The first call to [`check`] will initialise the trigger from
    /// `section.end_time`.
    ///
    /// [`check`]: SectionLoopTrigger::check
    pub fn new() -> Self {
        Self {
            next_trigger: None,
            armed: None,
            consumed: Duration::ZERO,
        }
    }

    /// Returns `Some(trigger_time)` when a loop transition should fire; see
    /// [`SectionLoopTrigger::jump`].
    pub fn check(
        &mut self,
        section: &SectionBounds,
        elapsed: Duration,
        margin: Duration,
    ) -> Option<Duration> {
        self.jump(section, elapsed, margin).map(|jump| jump.at)
    }

    /// Returns the jump when a loop transition should fire.
    ///
    /// Armed for a section (first, or another than before, or again after a
    /// [`reset`]), the trigger is set to when playback reaches the section's
    /// end: its end plus the time loops so far have put back. When
    /// `elapsed` plus `margin` reaches it, the jump fires and the next is
    /// set a section's length later (grid-locked). A section armed with playback already past
    /// its end fires at once, from where playback is.
    ///
    /// Returns `None` if the section has zero duration or the trigger time
    /// has not been reached.
    ///
    /// [`reset`]: SectionLoopTrigger::reset
    pub fn jump(
        &mut self,
        section: &SectionBounds,
        elapsed: Duration,
        margin: Duration,
    ) -> Option<SectionJump> {
        self.jump_effective(
            Some(&EffectiveSection {
                bounds: section.clone(),
                overlap: Duration::ZERO,
            }),
            elapsed,
            margin,
        )
    }

    /// Returns the jump of the [`effective_section`] when it should fire, as
    /// [`SectionLoopTrigger::jump`] does for a section; with none, forgets
    /// the one armed ([`reset`]). The whole song's jump carries its overlap:
    /// all of it at the grid, what is left of the song for a pass that
    /// starts late, inside the crossfade (after a section loop broken there).
    ///
    /// [`reset`]: SectionLoopTrigger::reset
    pub fn jump_effective(
        &mut self,
        section: Option<&EffectiveSection>,
        elapsed: Duration,
        margin: Duration,
    ) -> Option<SectionJump> {
        let at = self.arm(section, elapsed)?;
        let section = self.armed.clone()?;
        if elapsed + margin < at {
            return None;
        }
        let bounds = &section.bounds;
        let from = at.saturating_sub(self.consumed);
        let jump = SectionJump {
            at,
            from,
            to: bounds.start_time,
            overlap: section
                .overlap
                .min((bounds.end_time + section.overlap).saturating_sub(from)),
        };
        self.consumed += jump.amount();
        self.next_trigger = Some(at + bounds.end_time.saturating_sub(bounds.start_time));
        Some(jump)
    }

    /// Arms the trigger for `section` (see [`SectionLoopTrigger::jump`]),
    /// giving when on the playback timeline its next jump falls: `None` with
    /// no section, or one of no length, which never loops.
    pub fn arm(
        &mut self,
        section: Option<&EffectiveSection>,
        elapsed: Duration,
    ) -> Option<Duration> {
        let Some(section) = section else {
            self.reset();
            return None;
        };
        let bounds = &section.bounds;
        if bounds.end_time.saturating_sub(bounds.start_time).is_zero() {
            return None;
        }
        if self.armed.as_ref() != Some(section) {
            self.armed = Some(section.clone());
            // Armed with playback past the end already: now. Only here: a
            // poll that comes late to a trigger set on the grid keeps the
            // grid.
            self.next_trigger = Some((bounds.end_time + self.consumed).max(elapsed));
        }
        self.next_trigger
    }

    /// How much playback time the loops so far have put back: playback time
    /// less this is the position in the song.
    pub fn consumed(&self) -> Duration {
        self.consumed
    }

    /// Takes back a jump given but not made (the audio had nothing ready to
    /// loop to): the time it put back is uncounted, and the section is set
    /// again from where playback is, so it goes round a moment late rather
    /// than not at all.
    pub fn retract(&mut self, jump: &SectionJump) {
        self.consumed = self.consumed.saturating_sub(jump.amount());
        self.reset();
    }

    /// Forgets the section armed, for a break or a section cleared: the next
    /// one armed, even the same one, is set from where playback is then. The
    /// time loops have put back is kept, as playback keeps it.
    pub fn reset(&mut self) {
        self.next_trigger = None;
        self.armed = None;
    }
}

impl Default for SectionLoopTrigger {
    fn default() -> Self {
        Self::new()
    }
}

/// Maps an ideal loop-boundary time to the exact mixer sample at which the
/// crossfade should complete.
///
/// `now_sample` is the mixer's current global sample and `elapsed` the
/// playback-clock reading taken alongside it; `trigger_time` is the ideal
/// boundary (>= `elapsed` whenever the trigger fires early). The audio engine
/// uses the returned sample to schedule a sample-accurate handoff, so the
/// audible loop point is independent of polling jitter.
///
/// Because consecutive `trigger_time`s are grid-locked
/// (`prev + section_duration`, see [`SectionLoopTrigger::check`]) and the
/// audio clock is itself derived from the mixer sample counter, the returned
/// samples are spaced exactly `section_duration * sample_rate` apart. That is
/// the invariant behind "one measure loops in exactly one measure" — beat-to-
/// beat spacing across iterations stays constant.
pub fn loop_boundary_sample(
    now_sample: u64,
    elapsed: Duration,
    trigger_time: Duration,
    sample_rate: u32,
) -> u64 {
    let remaining = trigger_time.saturating_sub(elapsed);
    let samples_until = (remaining.as_secs_f64() * sample_rate as f64).round() as u64;
    now_sample.saturating_add(samples_until)
}

/// Result of polling a [`SectionLoopMonitor`].
///
/// Each variant tells the caller what happened so it can run the
/// subsystem-specific restart or cleanup logic.
/// How far ahead of a jump [`publish_jumps`] commits to it, as the audio
/// does: the followers wait until it falls due.
const PUBLISH_LOOKAHEAD: Duration = Duration::from_millis(30);

/// What [`publish_jumps`] needs from a playback.
pub struct JumpPublisher {
    pub clock: crate::clock::PlaybackClock,
    pub start_time: Duration,
    pub cancel: crate::playsync::CancelHandle,
    /// Set once the playback's subsystems have finished.
    pub done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// For a playback with audio, set once the audio has stopped: if it
    /// stopped without taking on the jumps (it failed at the start), this
    /// does. `None` for a playback without, which this commits from the
    /// start.
    pub audio_gone: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub loop_control: crate::playsync::LoopControl,
    /// The song: how it loops as a whole, and where it ends. Without one
    /// (in tests), nothing loops as a whole, and the log closes only once
    /// the playback is done.
    pub song: Option<std::sync::Arc<crate::songs::Song>>,
}

/// Where a song played without audio ends, for [`publish_jumps`] to close
/// the log there: its lights' end, or, with none, its MIDI's last event.
fn end_without_audio(song: &crate::songs::Song) -> Duration {
    let length = song.length();
    if !length.is_zero() {
        return length;
    }
    song.midi_playback()
        .and_then(|midi| midi.midi_sheet().ok())
        .and_then(|sheet| sheet.precomputed.events().last().map(|event| event.time))
        .unwrap_or_default()
}

/// Commits a playback's loop jumps when it plays no audio, which otherwise
/// commits to them, deciding as the audio would from the one
/// [`effective_section`]: an armed section, not broken, loops at its end; a
/// looping song with none goes round as a whole, each pass its
/// `loop_crossfade` before the last ends. The notes, lights and beat clock
/// follow the jumps from the log, as does the player's position. Once the
/// song is over (nothing looping, and played to its end), the log is closed,
/// which lets the followers end.
pub fn publish_jumps(publisher: JumpPublisher) {
    use std::sync::atomic::Ordering;
    let JumpPublisher {
        clock,
        start_time,
        cancel,
        done,
        audio_gone,
        loop_control,
        song,
    } = publisher;
    let finished = || cancel.is_cancelled() || done.load(Ordering::Relaxed);
    while !clock.wait_for_start_or_cancel_until(&cancel, Duration::from_millis(10)) {
        if finished() {
            return;
        }
    }
    // With audio, it commits the jumps once it plays; this only if it never
    // does.
    let log = &loop_control.section_jumps;
    loop {
        if finished() || log.claimed_by_audio() {
            return;
        }
        let audio_failed = audio_gone
            .as_ref()
            .is_none_or(|gone| gone.load(Ordering::Acquire));
        if audio_failed && log.claim(crate::playsync::JumpPublisherRole::Fallback) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let end = song.as_deref().map(end_without_audio);
    let whole_song = song
        .as_deref()
        .and_then(|song| WholeSongLoop::of(song, end.unwrap_or_default()));
    let mut trigger = SectionLoopTrigger::new();
    while !finished() {
        let elapsed = start_time + clock.elapsed();
        let effective = effective_section(
            loop_control.active_section.read().clone(),
            loop_control.section_loop_break.load(Ordering::Relaxed),
            whole_song,
            loop_control.loop_break.load(Ordering::Relaxed),
        );
        if let Some(jump) = trigger.jump_effective(effective.as_ref(), elapsed, PUBLISH_LOOKAHEAD) {
            log.push(jump);
        }
        // Played to its end with nothing to take it back: the song is over.
        if effective.is_none()
            && (end.is_some_and(|end| elapsed.saturating_sub(trigger.consumed()) >= end)
                || loop_control.loop_break.load(Ordering::Relaxed))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    log.close();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_section(start_secs: u64, end_secs: u64) -> SectionBounds {
        SectionBounds {
            name: "test".to_string(),
            start_time: Duration::from_secs(start_secs),
            end_time: Duration::from_secs(end_secs),
        }
    }

    /// Validates that trigger times remain locked to the ideal metronomic
    /// grid over many iterations — the core invariant that prevents timing
    /// drift. See also `crossfade::tests::section_loop_triggers_stay_on_grid`
    /// for the mathematical rationale.
    #[test]
    fn stays_on_grid() {
        let section = make_section(10, 18); // 8-second section
        let section_duration = Duration::from_secs(8);
        let margin = crate::audio::crossfade::DEFAULT_CROSSFADE_DURATION;
        let mut trigger = SectionLoopTrigger::new();

        let iterations = 100u32;
        for i in 0..iterations {
            let expected_trigger = section.end_time + section_duration * i;
            // Simulate detecting slightly before trigger (as engines do).
            let elapsed = expected_trigger - margin;

            let result = trigger.check(&section, elapsed, margin);
            assert_eq!(
                result,
                Some(expected_trigger),
                "Iteration {}: expected trigger at {:?}",
                i,
                expected_trigger
            );
        }

        // Final trigger should land at exactly end_time + 100 * 8s = 818s.
        let expected_next = section.end_time + section_duration * iterations;
        // Peek at internal state via one more check that should NOT fire.
        let too_early = expected_next - margin - Duration::from_secs(1);
        assert_eq!(trigger.check(&section, too_early, margin), None);
    }

    /// The core timing guarantee: for a 4/4 song at 120 BPM, a one-measure
    /// section (2.0s) loops with its boundary landing on the same sample
    /// spacing every iteration, regardless of when the polling monitor
    /// actually detects the trigger. This is the unit-level equivalent of
    /// measuring "beat 2 → beat 2" across loop iterations.
    #[test]
    fn loop_boundaries_are_evenly_spaced_despite_jitter() {
        let sample_rate = 48_000u32;
        let section_duration = Duration::from_secs(2); // one measure @ 120 BPM
        let section_samples = (section_duration.as_secs_f64() * sample_rate as f64) as u64;
        let clock_start_sample = 12_345u64; // arbitrary, non-zero epoch

        // Models the audio-backed clock: elapsed maps to a sample offset from
        // the epoch. The mixer counter and the clock are the same source, so
        // now_sample and elapsed are always consistent.
        let sample_at = |elapsed: Duration| -> u64 {
            clock_start_sample + (elapsed.as_secs_f64() * sample_rate as f64).round() as u64
        };

        let mut next_trigger = section_duration; // first boundary at 2.0s
        let mut prev_boundary: Option<u64> = None;

        for iteration in 0..100u64 {
            // The monitor fires early, anywhere within the look-ahead window.
            // Every jitter value must resolve to the *same* boundary sample.
            let expected_boundary = sample_at(next_trigger);
            for jitter_ms in [0u64, 1, 7, 23, 30] {
                let elapsed = next_trigger.saturating_sub(Duration::from_millis(jitter_ms));
                let now_sample = sample_at(elapsed);
                let boundary = loop_boundary_sample(now_sample, elapsed, next_trigger, sample_rate);
                assert_eq!(
                    boundary, expected_boundary,
                    "iteration {iteration}: {jitter_ms}ms detection jitter moved the boundary"
                );
            }

            if let Some(prev) = prev_boundary {
                assert_eq!(
                    expected_boundary - prev,
                    section_samples,
                    "iteration {iteration}: loop boundary spacing drifted"
                );
            }
            prev_boundary = Some(expected_boundary);
            next_trigger += section_duration;
        }
    }

    #[test]
    fn no_fire_before_time() {
        let section = make_section(10, 18);
        let margin = Duration::from_millis(5);
        let mut trigger = SectionLoopTrigger::new();

        // Well before section end.
        assert_eq!(
            trigger.check(&section, Duration::from_secs(5), margin),
            None
        );

        // Just barely too early (1ms short).
        let just_short = section.end_time - margin - Duration::from_millis(1);
        assert_eq!(trigger.check(&section, just_short, margin), None);

        // Exactly at threshold.
        let at_threshold = section.end_time - margin;
        assert_eq!(
            trigger.check(&section, at_threshold, margin),
            Some(section.end_time)
        );
    }

    #[test]
    fn resets_cleanly() {
        let section = make_section(10, 18);
        let margin = Duration::from_millis(5);
        let mut trigger = SectionLoopTrigger::new();

        // Fire first trigger.
        let elapsed = section.end_time - margin;
        assert!(trigger.check(&section, elapsed, margin).is_some());

        // Reset, and armed again it is set from where playback is: the
        // section's end plus the 8s the loop put back, not its end alone,
        // which playback passed a loop ago (and re-armed, jumped at once).
        trigger.reset();
        let elapsed = section.end_time + Duration::from_secs(8) - margin;
        assert_eq!(
            trigger.check(&section, elapsed, margin),
            Some(section.end_time + Duration::from_secs(8)),
            "After reset, trigger should re-initialise from where playback reaches the end"
        );
    }

    /// Another section armed while one loops is set from where playback
    /// reaches its end, counting the time the loops so far put back, and the
    /// jumps say how much each put back.
    #[test]
    fn another_section_armed_is_set_from_where_playback_is() {
        let a = make_section(10, 18);
        let b = make_section(20, 30);
        let mut trigger = SectionLoopTrigger::new();
        let jump = trigger
            .jump(&a, Duration::from_secs(18), Duration::ZERO)
            .unwrap();
        assert_eq!(
            jump,
            SectionJump {
                at: Duration::from_secs(18),
                from: Duration::from_secs(18),
                to: Duration::from_secs(10),
                overlap: Duration::ZERO,
            }
        );
        // 8s put back: B's end is reached at 38s, not at A's next boundary
        // (26s) nor B's own end (30s).
        assert_eq!(
            trigger.jump(&b, Duration::from_secs(26), Duration::ZERO),
            None
        );
        assert_eq!(
            trigger.jump(&b, Duration::from_secs(30), Duration::ZERO),
            None
        );
        let jump = trigger
            .jump(&b, Duration::from_secs(38), Duration::ZERO)
            .unwrap();
        assert_eq!(
            (jump.from, jump.to, jump.amount()),
            (
                Duration::from_secs(30),
                Duration::from_secs(20),
                Duration::from_secs(10)
            )
        );
    }

    /// Armed past its end, a section jumps at once, from where playback is,
    /// and goes round its full length from there.
    #[test]
    fn a_section_armed_past_its_end_jumps_at_once() {
        let a = make_section(10, 18);
        let mut trigger = SectionLoopTrigger::new();
        let jump = trigger
            .jump(&a, Duration::from_secs(20), Duration::ZERO)
            .unwrap();
        assert_eq!(
            (jump.at, jump.from, jump.amount()),
            (
                Duration::from_secs(20),
                Duration::from_secs(20),
                Duration::from_secs(10)
            )
        );
        assert_eq!(
            trigger.jump(&a, Duration::from_secs(27), Duration::ZERO),
            None
        );
        assert_eq!(
            trigger.check(&a, Duration::from_secs(28), Duration::ZERO),
            Some(Duration::from_secs(28))
        );
    }

    /// A poll that comes late to a trigger on the grid keeps the grid.
    #[test]
    fn a_late_poll_keeps_the_grid() {
        let a = make_section(10, 18);
        let mut trigger = SectionLoopTrigger::new();
        assert_eq!(
            trigger.check(&a, Duration::from_secs(5), Duration::ZERO),
            None
        );
        let late = Duration::from_millis(18_003);
        assert_eq!(
            trigger.check(&a, late, Duration::ZERO),
            Some(Duration::from_secs(18))
        );
        assert_eq!(
            trigger.check(&a, Duration::from_millis(26_004), Duration::ZERO),
            Some(Duration::from_secs(26))
        );
    }

    /// A jump taken back leaves the time unchanged and fires again at once,
    /// from where playback has got to.
    #[test]
    fn a_retracted_jump_fires_again_from_where_playback_is() {
        let a = make_section(10, 18);
        let mut trigger = SectionLoopTrigger::new();
        let jump = trigger
            .jump(&a, Duration::from_secs(18), Duration::ZERO)
            .unwrap();
        trigger.retract(&jump);
        assert_eq!(trigger.consumed(), Duration::ZERO);
        let again = trigger
            .jump(&a, Duration::from_millis(18_010), Duration::ZERO)
            .unwrap();
        assert_eq!(
            (again.from, again.to),
            (Duration::from_millis(18_010), Duration::from_secs(10))
        );
    }

    /// The publisher commits jumps only when the audio does not: not while
    /// the audio may yet play, and not once it has taken them on; but if the
    /// audio stops without taking them on (it failed at the start), it does.
    #[test]
    fn the_publisher_stands_in_only_for_audio_that_never_plays() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let run = |audio_takes_them: bool| {
            let mut loop_control = crate::playsync::LoopControl::new();
            loop_control.active_section = Arc::new(parking_lot::RwLock::new(Some(SectionBounds {
                name: "a".to_string(),
                start_time: Duration::ZERO,
                end_time: Duration::from_millis(50),
            })));
            let clock = crate::clock::PlaybackClock::wall();
            let audio_gone = Arc::new(AtomicBool::new(false));
            let done = Arc::new(AtomicBool::new(false));
            let log = loop_control.section_jumps.clone();
            let publisher = JumpPublisher {
                clock: clock.clone(),
                start_time: Duration::ZERO,
                cancel: crate::playsync::CancelHandle::new(),
                done: done.clone(),
                audio_gone: Some(audio_gone.clone()),
                loop_control,
                song: None,
            };
            let thread = std::thread::spawn(move || publish_jumps(publisher));
            clock.start();
            std::thread::sleep(Duration::from_millis(150));
            let while_pending = crate::playsync::JumpCursor::new(log.clone())
                .peek()
                .is_some();
            if audio_takes_them {
                assert!(log.claim(crate::playsync::JumpPublisherRole::Audio));
            }
            audio_gone.store(true, Ordering::Release);
            std::thread::sleep(Duration::from_millis(150));
            done.store(true, Ordering::Relaxed);
            thread.join().unwrap();
            (
                while_pending,
                crate::playsync::JumpCursor::new(log).peek().is_some(),
            )
        };
        assert_eq!(
            run(false),
            (false, true),
            "the audio failed: the publisher commits"
        );
        assert_eq!(
            run(true),
            (false, false),
            "the audio commits: the publisher does not"
        );
    }

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    /// A 100s song looping with a 4s crossfade.
    fn whole_song() -> Option<WholeSongLoop> {
        Some(WholeSongLoop {
            length: secs(100),
            overlap: secs(4),
        })
    }

    /// With nothing armed, a looping song's passes are jumps in the log
    /// like a section's: each from where the outgoing pass is, its overlap
    /// before the end, back to the top, the outgoing pass playing on for the
    /// overlap; every period (length less overlap) on.
    #[test]
    fn a_whole_song_pass_is_a_jump_with_its_overlap() {
        let mut trigger = SectionLoopTrigger::new();
        let effective = effective_section(None, false, whole_song(), false);
        assert!(effective
            .as_ref()
            .is_some_and(EffectiveSection::is_whole_song));
        assert_eq!(
            trigger.jump_effective(effective.as_ref(), secs(95), Duration::ZERO),
            None
        );
        let pass = trigger
            .jump_effective(effective.as_ref(), secs(96), Duration::ZERO)
            .unwrap();
        assert_eq!(
            pass,
            SectionJump {
                at: secs(96),
                from: secs(96),
                to: Duration::ZERO,
                overlap: secs(4),
            }
        );
        assert_eq!(
            trigger.jump_effective(effective.as_ref(), secs(191), Duration::ZERO),
            None
        );
        assert_eq!(
            trigger
                .jump_effective(effective.as_ref(), secs(192), Duration::ZERO)
                .map(|jump| (jump.at, jump.from, jump.overlap)),
            Some((secs(192), secs(96), secs(4)))
        );
        // A song that loops without a crossfade switches at its end.
        let plain = effective_section(
            None,
            false,
            Some(WholeSongLoop {
                length: secs(100),
                overlap: Duration::ZERO,
            }),
            false,
        );
        let mut trigger = SectionLoopTrigger::new();
        assert_eq!(
            trigger
                .jump_effective(plain.as_ref(), secs(100), Duration::ZERO)
                .map(|jump| (jump.from, jump.to, jump.overlap)),
            Some((secs(100), Duration::ZERO, Duration::ZERO))
        );
        // Nor does a broken loop, or a song that does not loop, go round.
        assert_eq!(effective_section(None, false, whole_song(), true), None);
        assert_eq!(effective_section(None, false, None, false), None);
    }

    /// An armed section takes precedence over the whole song: one reaching
    /// into the crossfade, or ending on the song's end, loops on its own
    /// seam, and no pass starts under it at the crossfade's start.
    #[test]
    fn an_armed_section_keeps_the_whole_song_pass_from_firing() {
        for (start, end) in [(90, 98), (90, 100)] {
            let section = make_section(start, end);
            let mut trigger = SectionLoopTrigger::new();
            let mut jumps = Vec::new();
            let mut elapsed = secs(80);
            while elapsed < secs(200) {
                let effective =
                    effective_section(Some(section.clone()), false, whole_song(), false);
                if let Some(jump) =
                    trigger.jump_effective(effective.as_ref(), elapsed, Duration::ZERO)
                {
                    jumps.push(jump);
                }
                elapsed += Duration::from_millis(10);
            }
            assert!(!jumps.is_empty());
            for jump in &jumps {
                assert_eq!(
                    (jump.from, jump.to, jump.overlap),
                    (secs(end), secs(start), Duration::ZERO),
                    "section {start}-{end}s: only its own jumps"
                );
            }
        }
    }

    /// Broken out of a section past the crossfade's start, the whole song
    /// goes round at once from where playback is, crossfading over what is
    /// left of it; broken before, at the crossfade's start.
    #[test]
    fn after_a_break_the_pass_fires_where_playback_is() {
        let section = make_section(90, 99);
        let mut trigger = SectionLoopTrigger::new();
        let armed = effective_section(Some(section.clone()), false, whole_song(), false);
        let jump = trigger
            .jump_effective(armed.as_ref(), secs(99), Duration::ZERO)
            .unwrap();
        assert_eq!(jump.amount(), secs(9));
        // At 106s on the timeline: 97s into the song, past the crossfade's
        // start (96s).
        let broken = effective_section(Some(section), true, whole_song(), false);
        let pass = trigger
            .jump_effective(broken.as_ref(), secs(106), Duration::ZERO)
            .unwrap();
        assert_eq!(
            pass,
            SectionJump {
                at: secs(106),
                from: secs(97),
                to: Duration::ZERO,
                overlap: secs(3),
            }
        );
        // Broken earlier in the song, the pass waits for the crossfade.
        let section = make_section(10, 20);
        let mut trigger = SectionLoopTrigger::new();
        let armed = effective_section(Some(section.clone()), false, whole_song(), false);
        trigger
            .jump_effective(armed.as_ref(), secs(20), Duration::ZERO)
            .unwrap();
        let broken = effective_section(Some(section), true, whole_song(), false);
        assert_eq!(
            trigger.jump_effective(broken.as_ref(), secs(25), Duration::ZERO),
            None
        );
        assert_eq!(
            trigger
                .jump_effective(broken.as_ref(), secs(106), Duration::ZERO)
                .map(|jump| (jump.from, jump.overlap)),
            Some((secs(96), secs(4)))
        );
    }

    #[test]
    fn handles_zero_duration_section() {
        let section = make_section(10, 10); // zero duration
        let margin = Duration::from_millis(5);
        let mut trigger = SectionLoopTrigger::new();

        // Should never fire for a zero-duration section.
        assert_eq!(
            trigger.check(&section, Duration::from_secs(10), margin),
            None
        );
        assert_eq!(
            trigger.check(&section, Duration::from_secs(100), margin),
            None
        );
    }
}
