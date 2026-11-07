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

//! The hardware and controllers as status surfaces report them (`/api/status`,
//! the MCP status tool): light, so the web UI's WebAssembly module compiles
//! it as it is.

use serde::Serialize;

use crate::audio::output_status::OutputStatus;

/// Status of a single hardware subsystem.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SubsystemStatus {
    /// `connected`, `initializing`, `failed`, or `not_connected`.
    pub status: String,
    pub name: Option<String>,
    /// Why init gave up, present only for `failed`.
    ///
    /// `failed` without a reason would be no better than the perpetual retry it
    /// replaced: the operator still would not know what to change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What the audio output callback is actually doing, as distinct from whether the
/// device opened.
///
/// `connected` on the audio subsystem only ever meant "we constructed a device
/// object at startup". These are the runtime facts on top of that.
///
/// Note what this cannot tell you: a device can accept every buffer and produce no
/// sound, and nothing here will notice. `writing_signal` true with a silent room
/// means the fault is downstream of mtrack — which is the question worth answering
/// quickly when a rig goes quiet.
///
/// Not a meter. There is no level here on purpose: a level sampled by a status
/// poller sees one callback in several hundred, which makes a poor meter and adds
/// nothing to the health question that the two booleans don't already answer.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AudioOutputHealth {
    /// One-word verdict: `healthy`, `recovering`, `stalled`, or `never_started`.
    ///
    /// The fields below are the evidence behind it. This exists because "is the
    /// audio all right?" is the question being asked, and making a reader derive
    /// it from four fields is how an operator ends up trusting `connected`.
    pub status: OutputStatus,
    /// The callback has run recently. False means the stream is open but stalled.
    pub callback_alive: bool,
    /// Non-silent audio was handed to the device recently. False is normal and
    /// expected whenever nothing is playing, and can also go false during a
    /// genuinely quiet passage or with output gains at zero.
    pub writing_signal: bool,
    /// Milliseconds since the callback last ran, absent if it never has.
    pub since_last_callback_ms: Option<u64>,
    /// Milliseconds since non-silent audio was last written, absent if it never was.
    pub since_last_signal_ms: Option<u64>,
    /// Total callbacks served since the device was opened.
    pub callbacks: u64,
    /// Streams rebuilt after a backend error since the device was opened. A rig
    /// recovering from a flaky cable several times a set looks fine at any single
    /// instant; this is the only thing that would show it.
    pub recoveries: u64,
    /// Buffer underruns since the device was opened. Counted apart from errors
    /// because they are routine and self-healing — a rising count means the
    /// buffer is too small for the machine, not that anything has failed.
    pub underruns: u64,
    /// The most recent backend error, if there has been one. Retained after
    /// recovery on purpose — it is the thing worth reading afterwards.
    pub last_error: Option<String>,
}

/// Snapshot of all hardware subsystem statuses.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HardwareStatusSnapshot {
    pub init_done: bool,
    pub hostname: Option<String>,
    pub profile: Option<String>,
    /// The file the active profile was loaded from (`stage.yml`), which the
    /// web UI links it by; none for an inline profile.
    pub profile_file: Option<String>,
    pub audio: SubsystemStatus,
    /// Runtime output health, when the audio device can report it.
    pub audio_output: Option<AudioOutputHealth>,
    pub midi: SubsystemStatus,
    pub dmx: SubsystemStatus,
    pub trigger: SubsystemStatus,
    /// Whether the current lighting venue's fixtures are registered. DMX can
    /// be `connected` while the venue failed — then nothing lights — so it
    /// is its own entry.
    pub lighting_venue: LightingVenueStatus,
    /// A fixture test's output, while one is live: which universe and
    /// addresses it holds, and for how much longer without an update.
    pub test_output: Option<TestOutputStatus>,
}

/// The current lighting venue's registration, for status surfaces.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LightingVenueStatus {
    /// The venue; `None` when there is none.
    pub name: Option<String>,
    /// `ok`, `failed`, or `none` (no DMX engine, no lighting system, or no
    /// current venue).
    pub status: String,
    /// Why it failed: the fixture and the reason. Present only for `failed`
    /// (serialized as null otherwise).
    pub error: Option<String>,
}

/// What `/api/status` says about a running test.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TestOutputStatus {
    pub universe: u16,
    pub address: u16,
    pub footprint: u16,
    pub fixture_type: String,
    pub mode: Option<String>,
    pub expires_in_secs: f64,
}

/// Status of a single controller.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ControllerStatus {
    /// The kind of controller (grpc, osc, midi).
    pub kind: String,
    /// Whether the controller started successfully.
    pub status: String,
    /// Additional info (e.g. port number, device name).
    pub detail: Option<String>,
    /// Error message if the controller failed to start.
    pub error: Option<String>,
}
