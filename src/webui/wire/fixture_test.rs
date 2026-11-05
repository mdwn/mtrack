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

//! The fixture test's payloads (`/api/lighting/fixture-test`): what a type's
//! modes let a test do, the test asked for, and what it sent.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::lighting::fixture_test::{ControlSupport, FrameChannel};

/// `GET /api/lighting/fixture-test/options`: the type's modes, the running
/// profile's universes, and whether a test can run now.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TestOptions {
    pub fixture_type: String,
    /// Whether the type comes from a GDTF, whose modes it has.
    pub gdtf: bool,
    pub modes: Vec<TestMode>,
    /// The first drivable mode.
    pub default_mode: Option<String>,
    pub universes: Vec<TestUniverse>,
    pub olad: TestOlad,
    pub available: bool,
    pub unavailable: Option<TestUnavailable>,
    /// The server's own sentence for `unavailable`.
    pub unavailable_message: Option<String>,
}

/// A mode of the type, and what a test can do in it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TestMode {
    /// `null` for a hand-written type, which has one way of being driven.
    pub name: Option<String>,
    pub drivable: bool,
    /// Why the mode cannot be driven.
    pub error: Option<String>,
    /// `null` when the mode cannot be driven; so are `controls`.
    pub footprint: Option<u16>,
    pub controls: Option<ControlSupport>,
    pub channels: Vec<TestChannel>,
}

/// A channel of a mode, by its 1-based offset.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TestChannel {
    pub offset: u16,
    pub name: String,
}

/// A universe of the running profile.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TestUniverse {
    pub universe: u16,
    pub name: Option<String>,
    /// Whether olad has an output patched; `null` when it was not reached.
    pub patched: Option<bool>,
}

/// Whether olad answered (`null`: not asked), and its port.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TestOlad {
    pub reachable: Option<bool>,
    pub port: Option<u16>,
}

/// Why a test cannot run now.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TestUnavailable {
    Locked,
    Playing,
    NoDmx,
    NoUniverses,
}

/// `POST /api/lighting/fixture-test`: a test to send. The controls are read
/// leniently, each refused by name when it is not what it should be.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct TestRequest {
    pub fixture_type: String,
    #[serde(default)]
    pub mode: Option<String>,
    pub universe: u16,
    pub address: u16,
    /// `color` (`#rrggbb`), `dimmer`, `strobe` (Hz, `null` for off), `pan`,
    /// `tilt` and further levels by channel name.
    #[serde(default)]
    #[cfg_attr(
        feature = "tsify",
        tsify(type = "Record<string, string | number | null>")
    )]
    pub controls: serde_json::Map<String, serde_json::Value>,
    /// Raw channel values, by offset or channel name.
    #[serde(default)]
    #[cfg_attr(feature = "tsify", tsify(type = "Record<string, number>"))]
    pub raw: BTreeMap<String, serde_json::Value>,
}

/// What a test sent, and what it should be told.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TestAnswer {
    /// Always `true`: the test is live.
    pub active: bool,
    pub universe: u16,
    pub address: u16,
    pub footprint: u16,
    pub fixture_type: String,
    pub mode: Option<String>,
    /// How long it holds without another send.
    pub expires_in_secs: f64,
    pub frame: Vec<FrameChannel>,
    pub warnings: Vec<TestWarning>,
}

/// Something a test should be told.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TestWarning {
    /// Fixtures of the current venue the test is lit over.
    VenueOverlap {
        fixtures: Vec<String>,
        message: String,
    },
    /// The span runs past address 512.
    PastUniverseEnd { message: String },
    /// olad does not answer.
    OladUnreachable { message: String },
    /// olad has no output patched for the universe.
    UniverseUnpatched { message: String },
    /// A control the mode has no channel for.
    UnsupportedControl { control: String, message: String },
}
