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

//! Stage space, as the venue and the stage views place things in it: light,
//! so the web UI's WebAssembly module compiles it as it is.

/// A stage-space triple: meters, right-handed Z-up, origin downstage-center
/// on the deck, +x stage-left, +y upstage, +z up. (For a rotation, degrees.)
#[cfg_attr(feature = "tsify", tsify_next::declare)]
pub type Vec3 = [f64; 3];
