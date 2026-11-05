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

//! Where a mover is pointing, as the stage views draw it: light, so the
//! web UI's WebAssembly module compiles it as it is.

use serde::Serialize;

use crate::lighting::geometry::Vec3;

/// Where a mover is pointing, for the stage plot's beam. Sent by fixture
/// name, so the name itself is not.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PoseSnapshot {
    #[serde(skip)]
    pub name: String,
    pub pan: f64,
    pub tilt: f64,
    /// The beam direction in stage space, unit length.
    pub aim: Vec3,
    /// Where the beam meets the deck (stage x, y), when it points down and
    /// the venue places the fixture; a beam pointing up has no footprint.
    pub floor: Option<[f64; 2]>,
}
