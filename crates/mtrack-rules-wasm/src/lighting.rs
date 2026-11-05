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
//! The player's `crate::lighting` as the light show parser sees it: the
//! effect types it builds and the parser itself, each source compiled as
//! it is in the player. Every `#[path]` here and in the two submodules
//! names a player source; the Makefile reads them for the UI build's
//! dependencies.

pub use crate::{patch, tempo};

pub mod effects;
#[path = "../../../src/lighting/findings.rs"]
pub mod findings;
#[path = "../../../src/lighting/fit/types.rs"]
mod fit_types;
#[path = "../../../src/lighting/fixture_test/types.rs"]
mod fixture_test_types;
#[path = "../../../src/lighting/geometry.rs"]
pub mod geometry;
pub mod parser;
#[path = "../../../src/lighting/stage_model.rs"]
pub mod stage_model;

pub mod fit {
    pub use super::fit_types::*;
}

pub mod fixture_test {
    pub use super::fixture_test_types::*;
}
