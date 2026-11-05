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
//! The modules of the player's `src/lighting/effects` the parser needs:
//! colours, tempo-aware rates and the effect types. The player's
//! `effects.rs` also carries the fixture, state and pointing modules,
//! which build on the venue and are not the parser's.

#[path = "../../../../src/lighting/effects/color.rs"]
mod color;
#[path = "../../../../src/lighting/effects/error.rs"]
mod error;
#[path = "../../../../src/lighting/effects/tempo_aware.rs"]
mod tempo_aware;
#[path = "../../../../src/lighting/effects/types.rs"]
mod types;

pub use color::{Color, NAMED_COLORS};
pub use error::EffectError;
pub use tempo_aware::{TempoAwareFrequency, TempoAwareSpeed, TempoAwareValue};
pub use types::{
    BlendMode, ChaseDirection, ChasePattern, CycleDirection, CycleTransition, DimmerCurve, Easing,
    EffectLayer, EffectType, MoveTarget, DEFAULT_BLEND, DEFAULT_LAYER,
};
