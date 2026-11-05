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

//! What the audio output is doing, as one word: light, so the web UI's
//! WebAssembly module compiles it as it is.

use serde::Serialize;

/// What the output is doing, as one word.
///
/// The individual facts are still there for anyone who wants them; this exists
/// because "is the audio all right?" is the question actually being asked, and
/// answering it by reading four fields is how an operator ends up looking at
/// `connected` and believing it.
///
/// Deliberately says nothing about whether audio reached the room. `Healthy`
/// means the callback is running and being handed buffers — a device can still
/// accept every one and produce nothing, which is not observable from here.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStatus {
    /// The callback is running.
    Healthy,
    /// The callback has stopped, and the output thread is rebuilding the stream.
    /// Someone is coming for it.
    Recovering,
    /// The callback has stopped and no rebuild is in progress. The wedge case:
    /// the stream still looks open and nothing is going to fix it.
    Stalled,
    /// The device is open but the callback has never run once.
    NeverStarted,
}

impl OutputStatus {
    /// The canonical name for this state.
    ///
    /// One vocabulary for every surface — the JSON status endpoint, the gRPC
    /// status response, the OSC broadcast and the web UI all use these exact
    /// strings, and the UI switches on them; serde's name for each is this one
    /// (`as_str_is_the_serialized_name`).
    pub fn as_str(&self) -> &'static str {
        match self {
            OutputStatus::Healthy => "healthy",
            OutputStatus::Recovering => "recovering",
            OutputStatus::Stalled => "stalled",
            OutputStatus::NeverStarted => "never_started",
        }
    }

    /// Whether this state should be shown to an operator as something wrong.
    ///
    /// `NeverStarted` is not a fault: it only survives for one liveness window
    /// after the device opens, and flagging it would make every start look
    /// broken for its first moments. A device that never calls back is reported
    /// as `Stalled` once that window passes, which is a fault.
    ///
    /// Kept in step with `StatusPage.audioFault()` deliberately — the two used to
    /// disagree about `NeverStarted`, so the first caller to reach for this
    /// helper would have contradicted the UI sitting next to it.
    pub fn is_fault(&self) -> bool {
        matches!(self, OutputStatus::Recovering | OutputStatus::Stalled)
    }
}

#[cfg(test)]
mod test {
    use super::OutputStatus;

    #[test]
    fn as_str_is_the_serialized_name() {
        for status in [
            OutputStatus::Healthy,
            OutputStatus::Recovering,
            OutputStatus::Stalled,
            OutputStatus::NeverStarted,
        ] {
            assert_eq!(serde_json::json!(status), status.as_str());
        }
    }
}
