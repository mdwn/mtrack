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

//! `POST /api/lighting/evaluate`: a song's shows evaluated offline, the 3D
//! view's Preview mode, in the shape the live `state` message has.

use serde::Serialize;

use crate::webui::wire::messages::StageValues;

/// What a song's shows do at each asked time.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Preview {
    pub song: String,
    pub evaluations: Vec<PreviewEvaluation>,
    /// Fixtures no cue in the shows targets: the same at every instant, so
    /// sent once.
    pub untouched: Vec<String>,
    /// Why the current venue does not register, when it does not: what is
    /// evaluated is then an empty rig, which this says.
    pub venue_error: Option<String>,
}

/// The stage at one asked time.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PreviewEvaluation {
    /// Seconds.
    pub time: f64,
    #[serde(flatten)]
    pub frame: StageValues,
    pub active_effects: Vec<PreviewEffect>,
}

/// An effect running at the evaluated time.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PreviewEffect {
    pub id: String,
    /// The logical groups the cue named; `null` when the engine did not
    /// say.
    pub groups: Option<Vec<String>>,
    /// The effect's kind as the engine names it: `Static`, `Move`, ...
    pub kind: String,
    pub layer: String,
    /// Seconds into the effect.
    pub elapsed: f64,
    /// The effect's authored length, seconds.
    pub duration: f64,
    pub fixtures: Vec<String>,
}
