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

//! The status the web UI holds: the shape of `Watch`'s hardware event.

use serde::Serialize;

use crate::player::{ControllerStatus, HardwareStatusSnapshot};

/// The build, the hardware and the controllers, as the web UI holds them.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Status {
    pub build: BuildInfo,
    pub hardware: HardwareStatusSnapshot,
    pub controllers: Vec<ControllerStatus>,
    pub locked: bool,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct BuildInfo {
    pub version: String,
    pub git_hash: String,
    pub build_time: String,
    /// The web UI this server serves; `null` when its build stamped none.
    pub ui_build: Option<String>,
}
