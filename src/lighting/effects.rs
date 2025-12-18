// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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
mod error;
mod fixture;
mod instance;
mod state;
mod tempo_aware;
mod types;

#[cfg(test)]
mod tests;

// Re-export public items
pub use color::Color;
pub use error::EffectError;
pub use fixture::{FixtureCapabilities, FixtureInfo, FixtureProfile, StrobeStrategy};
pub use instance::EffectInstance;
pub use state::{is_multiplier_channel, ChannelState, DmxCommand, FixtureState};
pub use tempo_aware::{TempoAwareFrequency, TempoAwareSpeed};
pub use types::{
    BlendMode, ChaseDirection, ChasePattern, CycleDirection, CycleTransition, DimmerCurve,
    EffectLayer, EffectType,
};
