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

pub use mtrack_core::lighting::{
    channels, effects, engine, findings, fixture_data, geometry, lint, parser, timeline, types,
};

pub mod analyze;
pub mod asset_store;
pub mod brightness_changes;
#[cfg(test)]
mod consistency_tests;
pub mod diff;
pub mod distill;
#[cfg(test)]
mod engine_tests;
pub mod evaluate;
pub mod export;
pub mod files;
pub mod fit;
pub mod fixture_patch;
pub mod fixture_test;
pub mod gdtf;
#[cfg(test)]
mod golden_tests;
pub mod import;
#[cfg(test)]
mod layering_tests;
pub mod library;
pub mod mvr;
pub mod mvr_types;
#[cfg(test)]
mod parser_tests;
pub mod patch;
pub mod project_files;
pub mod readiness;
pub mod stage_model;
pub mod system;
// Tempo lives at the crate root (shared with the metronome and song config);
// re-exported here so existing `lighting::tempo::…` paths keep working.
pub use crate::tempo;
pub mod validation;
pub mod venue_patch;
pub mod venue_rows;
#[cfg(test)]
mod visual_consistency_tests;
pub mod writes;

// Re-export the main types for convenience
// These are exported for external use of the lighting module
pub use effects::EffectInstance;
pub use engine::EffectEngine;
