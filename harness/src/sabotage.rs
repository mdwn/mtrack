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
//! Deliberately breaking a check's premise, to prove the check can fail.
//!
//! A check that cannot fail is worse than no check: it reports success forever
//! and nobody looks at it again. This harness has shipped three.
//! `bogus_*_device_degrades_gracefully` read a subsystem status milliseconds
//! after spawn, when everything reads `initializing` whatever mtrack does;
//! `stop_halts_playback` used a four-second song that ended on its own inside
//! the ten-second stop deadline; and `active_playlist_persists_across_restart`
//! asserted the opposite of documented behaviour. All three passed every
//! review; none could fail.
//!
//! Each check therefore names one thing it depends on and asks here whether to
//! break it. Running with the flag set is that check's negative control, and
//! `--self-test` requires every check to stop passing under it.
//!
//! Deliberately *not* an external source-mutating script. That approach was
//! tried and removed: its mutations were pinned to exact source strings, so it
//! rotted the moment a check was edited, and it had no idea the registry held
//! checks it did not cover. Here the control is compiled next to the assertion
//! it guards, and a check with no break point simply passes the self-test --
//! which the self-test reports as the defect it is.

use std::sync::atomic::{AtomicBool, Ordering};

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether the running check should break its own premise.
pub fn active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

/// Turns sabotage on for the next check.
///
/// Paired with [`disable`] by the self-test runner rather than being a scope
/// guard: `runner::execute` catches unwinds, so a panicking control cannot
/// leave the flag set. Checks execute one at a time, so a plain flag suffices.
pub fn enable() {
    ACTIVE.store(true, Ordering::SeqCst);
}

pub fn disable() {
    ACTIVE.store(false, Ordering::SeqCst);
}

/// Picks between the real value and one that should make the check fail.
///
/// The idiom for most break points:
///
/// ```ignore
/// let duration = sabotage::pick(120.0, 0.3);   // too short to observe
/// ```
pub fn pick<T>(real: T, broken: T) -> T {
    if active() {
        broken
    } else {
        real
    }
}

/// Whether a step should be performed at all.
///
/// For break points that are an *omission* rather than a substitution, such as
/// never sending the Stop the check is meant to be verifying.
pub fn perform() -> bool {
    !active()
}

/// Declines to sabotage, because this machine cannot supply the failure
/// condition the control needs.
///
/// Distinct from a check being unrunnable: the check itself runs and passes
/// here, only its *control* is impossible -- a rig with one loopback link
/// cannot permute a mapping, and one with no MIDI cannot make a MIDI-less
/// profile declare a device. Reported separately so the two are not confused.
#[macro_export]
macro_rules! no_control_here {
    ($($arg:tt)+) => {
        return Err($crate::outcome::CheckError::no_control_here(format!($($arg)+)))
    };
}
