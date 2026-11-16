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
//! What a rig and the hub say to each other: JSON over HTTPS, under
//! [`API`]. Every request to the hub but pairing carries the rig's token as
//! a bearer token. A file's `PUT` goes where the hub says, which may be
//! storage of its own; the token goes with it only when that is the hub.
//!
//! | Request | Does |
//! | --- | --- |
//! | `POST pairings` | Starts a pairing: a code for a person to enter in the hub. |
//! | `POST pairings/token` | Asks whether the code was entered; the rig's token once it was. |
//! | `GET project` | The project this rig is paired to and its current revision. |
//! | `GET revisions/{id}/manifest` | A revision's files. |
//! | `POST blobs/missing` | Which of these files the hub lacks, and where to send each. |
//! | `PUT` (the URL given) | One file's bytes. |
//! | `POST revisions` | A new revision: a manifest whose files the hub now has. |
//! | `GET blobs/{sha256}` | One file's bytes. |
//! | `PUT rig` | Which revision this rig is on. |
//!
//! Pairing is the OAuth device authorization grant (RFC 8628) in shape: the
//! rig polls while a person approves.

use std::collections::BTreeMap;

use mtrack_core::project::Manifest;
use serde::{Deserialize, Serialize};

/// The path every request is under, after the hub's address.
pub const API: &str = "/api/rig/v1";

/// `POST pairings`: what the rig calls itself, for the person approving.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairingRequest {
    pub rig_name: String,
}

/// A pairing under way.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairingStarted {
    /// What the rig asks for its token with. Never shown.
    pub device_code: String,
    /// What a person enters in the hub.
    pub user_code: String,
    /// Where they enter it.
    pub verification_uri: String,
    /// Seconds until the code is no good.
    pub expires_in: u64,
    /// Seconds to leave between asks.
    pub interval: u64,
}

/// `POST pairings/token`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenRequest {
    pub device_code: String,
}

/// Something with an id and a name a person gave it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Named {
    pub id: String,
    pub name: String,
}

/// A pairing a person approved.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Paired {
    /// The rig's token: for this rig and this project only.
    pub token: String,
    pub rig: Named,
    pub project: Named,
}

/// What the hub says when it refuses. `error` is one of the words below or
/// any other; `message` is for a person.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// `pairings/token`: nobody has entered the code yet.
pub const AUTHORIZATION_PENDING: &str = "authorization_pending";
/// `pairings/token`: asked too often; leave longer between asks.
pub const SLOW_DOWN: &str = "slow_down";
/// `pairings/token`: the code ran out.
pub const EXPIRED_TOKEN: &str = "expired_token";
/// `pairings/token`: the person refused.
pub const ACCESS_DENIED: &str = "access_denied";
/// `revisions`: the project moved on from the revision this one was made
/// from.
pub const NOT_CURRENT: &str = "not_current";
/// `revisions`: the project would not load.
pub const INVALID_PROJECT: &str = "invalid_project";

/// One saved state of a project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    pub id: String,
    /// Counted from 1 in the order they were saved.
    pub number: u64,
}

/// `GET project`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectState {
    pub project: Named,
    /// None for a project nothing has been uploaded to.
    pub revision: Option<Revision>,
}

/// `POST blobs/missing`: the hashes of files a rig is about to name in a
/// revision.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MissingRequest {
    pub sha256: Vec<String>,
}

/// Where to send a file the hub lacks: a `PUT` of its bytes to `url` with
/// `headers`. The URL is the hub's to choose, and need not be on the hub.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Upload {
    pub sha256: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

/// The answer to [`MissingRequest`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MissingResponse {
    pub missing: Vec<Upload>,
}

/// `POST revisions`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewRevision {
    /// The revision this one was made from: the one the rig was on. The hub
    /// refuses ([`NOT_CURRENT`]) when that is no longer the project's
    /// current one, so one rig's upload cannot silently undo another's.
    pub base: Option<String>,
    pub manifest: Manifest,
}

/// `PUT rig`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RigReport {
    pub revision: Option<String>,
}
