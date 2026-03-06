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

use thread_priority::{set_current_thread_priority, ThreadPriority, ThreadPriorityValue};
use tracing::info;

/// Default priority for the audio callback thread when MTRACK_THREAD_PRIORITY is unset.
const DEFAULT_CALLBACK_THREAD_PRIORITY: u8 = 70;

/// Reads MTRACK_THREAD_PRIORITY (0-99) once; used when building the callback so we don't touch env in the hot path.
pub fn callback_thread_priority() -> ThreadPriorityValue {
    parse_thread_priority(std::env::var("MTRACK_THREAD_PRIORITY").ok().as_deref())
}

/// Parses a thread priority value from an optional string (0-99).
/// Returns the default priority if the value is missing, unparseable, or out of range.
fn parse_thread_priority(value: Option<&str>) -> ThreadPriorityValue {
    value
        .and_then(|v| {
            let n = v.parse::<u8>().ok()?;
            (n < 100).then(|| ThreadPriorityValue::try_from(n).ok())?
        })
        .unwrap_or_else(|| ThreadPriorityValue::try_from(DEFAULT_CALLBACK_THREAD_PRIORITY).unwrap())
}

pub(crate) fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| {
            v == "1"
                || v.eq_ignore_ascii_case("true")
                || v.eq_ignore_ascii_case("yes")
                || v.eq_ignore_ascii_case("on")
        })
        .unwrap_or(false)
}

/// Returns whether we should attempt RT (SCHED_FIFO) scheduling for the audio callback thread.
/// Default: enabled. Advanced users can opt out with MTRACK_DISABLE_RT_AUDIO=1.
pub fn rt_audio_enabled() -> bool {
    !env_flag("MTRACK_DISABLE_RT_AUDIO")
}

pub fn configure_audio_thread_priority(
    priority: ThreadPriorityValue,
    rt_audio: bool,
    priority_set: &mut bool,
) {
    if *priority_set {
        return;
    }
    let tp = ThreadPriority::Crossplatform(priority);
    let _ = set_current_thread_priority(tp);

    #[cfg(unix)]
    if rt_audio {
        use thread_priority::unix::{
            set_thread_priority_and_policy, thread_native_id, RealtimeThreadSchedulePolicy,
            ThreadSchedulePolicy,
        };
        let tid = thread_native_id();
        match set_thread_priority_and_policy(
            tid,
            tp,
            ThreadSchedulePolicy::Realtime(RealtimeThreadSchedulePolicy::Fifo),
        ) {
            Ok(()) => {
                info!("Enabled RT SCHED_FIFO for audio callback thread");
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Failed to set RT SCHED_FIFO for audio callback thread"
                );
            }
        }
    }

    *priority_set = true;
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn env_flag_true_values() {
        for val in &[
            "1", "true", "TRUE", "True", "yes", "YES", "Yes", "on", "ON", "On",
        ] {
            std::env::set_var("MTRACK_TEST_FLAG", val);
            assert!(env_flag("MTRACK_TEST_FLAG"), "expected true for {:?}", val);
        }
        std::env::remove_var("MTRACK_TEST_FLAG");
    }

    #[test]
    fn env_flag_false_values() {
        for val in &["0", "false", "no", "off", "", "maybe"] {
            std::env::set_var("MTRACK_TEST_FLAG_F", val);
            assert!(
                !env_flag("MTRACK_TEST_FLAG_F"),
                "expected false for {:?}",
                val
            );
        }
        std::env::remove_var("MTRACK_TEST_FLAG_F");
    }

    #[test]
    fn env_flag_unset() {
        std::env::remove_var("MTRACK_TEST_FLAG_UNSET");
        assert!(!env_flag("MTRACK_TEST_FLAG_UNSET"));
    }

    #[test]
    fn parse_priority_default() {
        let prio = parse_thread_priority(None);
        assert_eq!(
            prio,
            ThreadPriorityValue::try_from(DEFAULT_CALLBACK_THREAD_PRIORITY).unwrap()
        );
    }

    #[test]
    fn parse_priority_custom() {
        let prio = parse_thread_priority(Some("50"));
        assert_eq!(prio, ThreadPriorityValue::try_from(50u8).unwrap());
    }

    #[test]
    fn parse_priority_zero() {
        let prio = parse_thread_priority(Some("0"));
        assert_eq!(prio, ThreadPriorityValue::try_from(0u8).unwrap());
    }

    #[test]
    fn parse_priority_max_valid() {
        let prio = parse_thread_priority(Some("99"));
        assert_eq!(prio, ThreadPriorityValue::try_from(99u8).unwrap());
    }

    #[test]
    fn parse_priority_out_of_range() {
        // 100 is out of range (0-99), should fall back to default.
        let prio = parse_thread_priority(Some("100"));
        assert_eq!(
            prio,
            ThreadPriorityValue::try_from(DEFAULT_CALLBACK_THREAD_PRIORITY).unwrap()
        );
    }

    #[test]
    fn parse_priority_invalid_string() {
        let prio = parse_thread_priority(Some("not_a_number"));
        assert_eq!(
            prio,
            ThreadPriorityValue::try_from(DEFAULT_CALLBACK_THREAD_PRIORITY).unwrap()
        );
    }

    #[test]
    fn parse_priority_empty_string() {
        let prio = parse_thread_priority(Some(""));
        assert_eq!(
            prio,
            ThreadPriorityValue::try_from(DEFAULT_CALLBACK_THREAD_PRIORITY).unwrap()
        );
    }

    #[test]
    fn configure_audio_thread_priority_idempotent() {
        let prio = ThreadPriorityValue::try_from(50u8).unwrap();
        let mut priority_set = false;

        configure_audio_thread_priority(prio, false, &mut priority_set);
        assert!(priority_set);

        // Second call should be a no-op (the flag is already set).
        configure_audio_thread_priority(prio, false, &mut priority_set);
        assert!(priority_set);
    }

    #[test]
    fn callback_thread_priority_respects_env() {
        // Run sequentially in one test to avoid racing on MTRACK_THREAD_PRIORITY.
        std::env::remove_var("MTRACK_THREAD_PRIORITY");
        let _prio = callback_thread_priority(); // Should not panic with default.

        std::env::set_var("MTRACK_THREAD_PRIORITY", "42");
        let prio = callback_thread_priority();
        std::env::remove_var("MTRACK_THREAD_PRIORITY");
        assert_eq!(prio, ThreadPriorityValue::try_from(42u8).unwrap());
    }

    #[test]
    fn rt_audio_enabled_respects_env() {
        // Run both checks sequentially in one test to avoid racing on
        // MTRACK_DISABLE_RT_AUDIO with parallel test threads.
        std::env::remove_var("MTRACK_DISABLE_RT_AUDIO");
        assert!(rt_audio_enabled(), "should be enabled when env var unset");

        std::env::set_var("MTRACK_DISABLE_RT_AUDIO", "1");
        assert!(!rt_audio_enabled(), "should be disabled when env var is 1");

        std::env::remove_var("MTRACK_DISABLE_RT_AUDIO");
    }

    #[test]
    fn configure_with_rt_audio() {
        let prio = ThreadPriorityValue::try_from(50u8).unwrap();
        let mut priority_set = false;

        // Exercise the RT audio path (may fail to set RT policy in test env, that's fine)
        configure_audio_thread_priority(prio, true, &mut priority_set);
        assert!(priority_set);
    }
}
