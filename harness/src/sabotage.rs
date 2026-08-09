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
//! and nobody looks at it again. This harness has shipped two.
//! `bogus_*_device_degrades_gracefully` read a subsystem status milliseconds
//! after spawn, when everything reads `initializing` whatever mtrack does, and
//! `stop_halts_playback` used a four-second song that ended on its own inside
//! the ten-second stop deadline. Both passed every review; neither could fail.
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

/// Runs `f` with sabotage enabled. Checks execute one at a time, so a plain
/// flag is sufficient.
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
