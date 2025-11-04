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

pub mod consistency_tests;
pub mod effects;
pub mod engine;
pub mod layering_tests;
pub mod parser;
pub mod system;
pub mod tempo;
pub mod timeline;
pub mod types;
pub mod visual_consistency_tests;

// Re-export the main types for convenience
// These are exported for external use of the lighting module
pub use effects::EffectInstance;
pub use engine::EffectEngine;
