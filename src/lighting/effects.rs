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

mod color;
mod composite;
mod error;
mod fixture;
mod instance;
mod physical;
mod pointing;
mod state;
mod tempo_aware;
mod types;

#[cfg(test)]
mod tests;

// Re-export public items
pub use color::Color;
pub(crate) use composite::resolve_fixture;
pub use error::EffectError;
pub use fixture::{
    cell_fixture_name, cell_name, has_cmy, has_dimmer_channel, FixtureCapabilities, FixtureInfo,
    FixtureProfile, StrobeStrategy,
};
pub use instance::EffectInstance;
pub use physical::{
    degree_span, degrees_from_bytes, fanout, resolve_degrees, resolve_normalized, resolve_physical,
    Intent, PhysicalParameter, PhysicalState, Resolved,
};
pub(crate) use pointing::mat_vec;
pub use pointing::{
    aim, aim_solutions, direction, lerp, nearest_pan, out_of_frame, AimCalibration, Pose,
};
pub use state::{ChannelState, DmxCommand, FixtureState};
pub use tempo_aware::{TempoAwareFrequency, TempoAwareSpeed, TempoAwareValue};
pub use types::{
    BlendMode, ChaseDirection, ChasePattern, CycleDirection, CycleTransition, DimmerCurve, Easing,
    EffectLayer, EffectType, MoveTarget,
};
