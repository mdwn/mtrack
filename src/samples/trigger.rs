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

//! Source-agnostic trigger types for sample playback.
//!
//! These types decouple the sample engine from any specific trigger source
//! (MIDI, audio triggers, etc.).

/// A source-agnostic trigger event that fires a sample.
#[derive(Debug, Clone)]
pub struct TriggerEvent {
    /// The name of the sample to trigger (references a SampleDefinition).
    pub sample_name: String,
    /// Velocity value (0-127) controlling volume/layer selection.
    pub velocity: u8,
    /// Optional release group — voices created by this trigger can be
    /// released later by matching on this group name.
    pub release_group: Option<String>,
}

/// An action produced by a trigger source.
#[derive(Debug, Clone)]
pub enum TriggerAction {
    /// Fire a sample.
    Trigger(TriggerEvent),
    /// Release all voices in the named group.
    Release { group: String },
}
