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
//! The arithmetic of a loop's crossfade that a light show shares with the
//! player's section loops.

use std::time::Duration;

/// How far through a crossfade of `overlap` it is `into` it: from 0 at its
/// start, past 1 once it is over. One with no overlap is over at once,
/// however little time has gone by.
pub fn crossfade_progress(into: Duration, overlap: Duration) -> f64 {
    if overlap.is_zero() {
        1.0
    } else {
        into.as_secs_f64() / overlap.as_secs_f64()
    }
}
