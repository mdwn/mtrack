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
}
