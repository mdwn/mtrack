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
//! Where headless log lines are written.
//!
//! To stderr: stdout is for what a command was asked for (`mtrack devices`,
//! `mtrack songs`), which scripts read, and under systemd the journal takes
//! stderr just as it takes stdout.

use std::io::IsTerminal;

/// Whether log lines should carry ANSI colour: only when stderr is a
/// terminal, and not when `NO_COLOR` is set. Under systemd the escape codes
/// went into every journal line, which the journal then stores as binary data
/// rather than text.
pub fn stderr_wants_colour() -> bool {
    std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
}
