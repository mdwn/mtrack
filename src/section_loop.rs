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
}

impl SectionLoopTrigger {
    /// Creates a new trigger with no scheduled time.
    ///
    /// The first call to [`check`] will initialise the trigger from
    /// `section.end_time`.
    ///
    /// [`check`]: SectionLoopTrigger::check
    pub fn new() -> Self {
        Self { next_trigger: None }
    }

    /// Returns `Some(trigger_time)` when a loop transition should fire.
    ///
    /// On the first call the trigger is initialised to `section.end_time`.
    /// When `elapsed + margin >= trigger`, the method fires and schedules
    /// the next trigger at `trigger + section_duration` (grid-locked).
    ///
    /// Returns `None` if the section has zero duration or the trigger time
    /// has not been reached.
    pub fn check(
        &mut self,
        section: &SectionBounds,
        elapsed: Duration,
        margin: Duration,
    ) -> Option<Duration> {
        let section_duration = section.end_time.saturating_sub(section.start_time);
        if section_duration.is_zero() {
            return None;
        }

        let trigger = *self.next_trigger.get_or_insert(section.end_time);

        if elapsed + margin >= trigger {
            self.next_trigger = Some(trigger + section_duration);
            Some(trigger)
        } else {
            None
        }
    }

    /// Clears the trigger state so the next [`check`] re-initialises from
    /// the section end time.
    ///
    /// [`check`]: SectionLoopTrigger::check
    pub fn reset(&mut self) {
        self.next_trigger = None;
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
#[derive(Debug, PartialEq)]
pub enum LoopPoll {
    /// No active section — the caller should idle.
    NoSection,
    /// A section is active but the trigger time has not been reached yet.
    /// The contained bounds are the current section.
    Waiting(SectionBounds),
    /// The trigger fired — the caller should restart playback from
    /// `section.start_time`.
    Triggered(SectionBounds),
    /// A previously active section was cleared (no break requested).
    /// The caller should reset any section-specific state.
    SectionCleared,
}

/// Combines [`SectionLoopTrigger`] with shared section-reading and caching
/// so each engine only needs to act on the [`LoopPoll`] result.
///
/// Engines that need section looping create one of these and call [`poll`]
/// on every iteration. The actual restart mechanism (MIDI cursor, DMX
/// timeline, etc.) remains in the caller.
///
/// [`poll`]: SectionLoopMonitor::poll
pub struct SectionLoopMonitor {
    trigger: SectionLoopTrigger,
    /// Cached section bounds for use after `active_section` is cleared.
    cached_section: Option<SectionBounds>,
}

impl SectionLoopMonitor {
    /// Creates a new monitor with no cached state.
    pub fn new() -> Self {
        Self {
            trigger: SectionLoopTrigger::new(),
            cached_section: None,
        }
    }

    /// Returns the currently cached section bounds, if any.
    pub fn cached_section(&self) -> Option<&SectionBounds> {
        self.cached_section.as_ref()
    }

    /// Polls the section loop state.
    ///
    /// `active_section` is the shared lock from [`LoopControl`].
    /// `elapsed` is the current clock time.
    ///
    /// The return value tells the caller what action to take. See
    /// [`LoopPoll`] for the full set of outcomes.
    ///
    /// [`LoopControl`]: crate::playsync::LoopControl
    pub fn poll(
        &mut self,
        active_section: &parking_lot::RwLock<Option<SectionBounds>>,
        elapsed: Duration,
    ) -> LoopPoll {
        let section = active_section.read().clone();
        if let Some(section) = section {
            self.cached_section = Some(section.clone());
            let crossfade_margin = crate::audio::crossfade::DEFAULT_CROSSFADE_DURATION;
            if self
                .trigger
                .check(&section, elapsed, crossfade_margin)
                .is_some()
            {
                LoopPoll::Triggered(section)
            } else {
                LoopPoll::Waiting(section)
            }
        } else {
            if self.cached_section.take().is_some() {
                // Had a section, now cleared — notify caller.
                self.trigger.reset();
                LoopPoll::SectionCleared
            } else {
                self.trigger.reset();
                LoopPoll::NoSection
            }
        }
    }

    /// Resets the trigger and cached section state.
    pub fn reset(&mut self) {
        self.trigger.reset();
        self.cached_section = None;
    }
}

impl Default for SectionLoopMonitor {
    fn default() -> Self {
        Self::new()
    }
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

        // Reset and verify it re-initialises from section.end_time.
        trigger.reset();
        let elapsed = section.end_time - margin;
        assert_eq!(
            trigger.check(&section, elapsed, margin),
            Some(section.end_time),
            "After reset, trigger should re-initialise from section.end_time"
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

    #[test]
    fn monitor_no_section() {
        let active = parking_lot::RwLock::new(None);
        let mut monitor = SectionLoopMonitor::new();
        assert_eq!(monitor.poll(&active, Duration::ZERO), LoopPoll::NoSection);
    }

    #[test]
    fn monitor_waiting_then_triggered() {
        let section = make_section(10, 18);
        let active = parking_lot::RwLock::new(Some(section.clone()));
        let mut monitor = SectionLoopMonitor::new();

        // Well before trigger time — should be Waiting.
        let result = monitor.poll(&active, Duration::from_secs(5));
        assert_eq!(result, LoopPoll::Waiting(section.clone()));

        // At trigger time — should fire.
        let margin = crate::audio::crossfade::DEFAULT_CROSSFADE_DURATION;
        let elapsed = section.end_time - margin;
        let result = monitor.poll(&active, elapsed);
        assert_eq!(result, LoopPoll::Triggered(section));
    }

    #[test]
    fn monitor_section_cleared() {
        let section = make_section(10, 18);
        let active = parking_lot::RwLock::new(Some(section.clone()));
        let mut monitor = SectionLoopMonitor::new();

        // Establish cached section.
        let _ = monitor.poll(&active, Duration::from_secs(5));
        assert!(monitor.cached_section().is_some());

        // Clear active section.
        *active.write() = None;
        let result = monitor.poll(&active, Duration::from_secs(5));
        assert_eq!(result, LoopPoll::SectionCleared);

        // Second poll with no section — back to NoSection.
        let result = monitor.poll(&active, Duration::from_secs(5));
        assert_eq!(result, LoopPoll::NoSection);
    }
}
