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
use std::collections::HashMap;

use serde::Deserialize;

/// The mappings of tracks to output channels.
#[derive(Deserialize)]
pub struct TrackMappings {
    // The individual track mappings.
    #[serde(flatten)]
    pub track_mappings: HashMap<String, Vec<u16>>,
}
