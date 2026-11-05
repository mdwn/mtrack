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

//! A profile's `track_mappings:` as written: light, so the web UI's WebAssembly module compiles it as it is
//! and the UI edits it by the generated types. What it means is `trackmappings`'s.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// The mappings of tracks to output channels.
///
/// Uses `IndexMap` so insertion order is preserved through serialize/deserialize
/// round-trips — important for the web UI config editor.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Deserialize, Serialize, Clone)]
pub struct TrackMappings {
    // The individual track mappings.
    #[serde(flatten)]
    #[cfg_attr(feature = "tsify", tsify(type = "Record<string, number[]>"))]
    pub track_mappings: IndexMap<String, Vec<u16>>,
}
