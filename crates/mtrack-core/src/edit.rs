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
//! A project's edits, as operations: each one change to a project's files,
//! made by `mtrack-core` and not by whoever asks for it, that says how to
//! take it back. The web UI, a hub, or anything else that edits a project
//! names an operation and its arguments; the operation reads, changes and
//! checks the file as mtrack does.
//!
//! Each area of a project has its operations in a module of its own, and
//! [`OPERATIONS`] lists every one by name, so that something offering them
//! can see which it offers.

pub mod light_show;

use serde::Serialize;

/// An operation, as [`OPERATIONS`] lists it.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operation {
    /// Its name: `area.operation`, as it is asked for by.
    pub name: &'static str,
    /// The part of a project it edits.
    pub area: &'static str,
}

/// Every operation there is.
pub const OPERATIONS: &[Operation] = light_show::OPERATIONS;
