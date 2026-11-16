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
//! The hub's requests ([`super::protocol`]), made.

use std::path::Path;
use std::time::Duration;

use mtrack_core::project::{hex, FileEntry, Manifest};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::protocol::{
    ApiError, MissingRequest, MissingResponse, NewRevision, Paired, PairingRequest, PairingStarted,
    ProjectState, Revision, RigReport, TokenRequest, Upload, API, AUTHORIZATION_PENDING,
    INVALID_PROJECT, NOT_CURRENT, SLOW_DOWN,
};

/// What went wrong talking to the hub.
#[derive(Debug, thiserror::Error)]
pub enum HubError {
    /// The hub could not be reached, or stopped answering, or what it sent
    /// could not be read.
    #[error("talking to the hub failed: {0}")]
    Http(#[from] reqwest::Error),
    /// An address the rig's token cannot be sent to.
    #[error("{0} is not an https address, and a rig's token is sent only over https")]
    NotHttps(String),
    /// The hub refused.
    #[error("the hub refused ({status}): {}", .message.as_deref().unwrap_or(.error))]
    Refused {
        status: u16,
        error: String,
        message: Option<String>,
    },
    /// The project has a newer revision than the one this rig was on.
    #[error("the hub has a newer revision of this project than this rig started from")]
    NotCurrent,
    /// The hub would not take the project: the player could not load it.
    #[error("the hub would not take this project: {0}")]
    InvalidProject(String),
    /// A file's bytes were not the ones its hash names. Named by its hash
    /// here, and by its path once the caller knows it.
    #[error("{0} did not arrive whole: its hash or length is not the revision's")]
    Corrupt(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// What asking for a pairing's token found.
#[derive(Debug)]
pub enum Poll {
    /// Nobody has entered the code yet.
    Pending,
    /// The same, and ask less often.
    SlowDown,
    /// Approved.
    Paired(Paired),
}

/// How long the hub may take to begin an answer, and a transfer may go
/// without a byte arriving, before it is given up.
const PATIENCE: Duration = Duration::from_secs(60);

/// The slowest link a file is still sent over: an upload is given
/// [`PATIENCE`] and its length at this many bytes a second.
const SLOWEST_UPLOAD: u64 = 16 * 1024;

/// A connection to one hub, as one rig once it has a token.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
}

impl Client {
    /// A client of the hub at `hub` (its address, no path), with the rig's
    /// token when it has one. The address is https, or this machine's own
    /// (a hub under test): the token is not sent in the clear.
    pub fn new(hub: &str, token: Option<String>) -> Result<Client, HubError> {
        // Read as the HTTP client will read it, so that what is checked is
        // where the token goes: `http://127.0.0.1:80@elsewhere` is elsewhere.
        let safe = reqwest::Url::parse(hub).is_ok_and(|url| {
            let own = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
            url.username().is_empty()
                && url.password().is_none()
                && (url.scheme() == "https" || (url.scheme() == "http" && own))
        });
        if !safe {
            return Err(HubError::NotHttps(hub.to_string()));
        }
        let http = reqwest::Client::builder()
            .user_agent(concat!("mtrack/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(20))
            .build()?;
        Ok(Client {
            http,
            base: format!("{}{API}", hub.trim_end_matches('/')),
            token,
        })
    }

    /// A request of the hub's API. It ends if the hub has not begun to
    /// answer in [`PATIENCE`]: every one is small but a file's, which has
    /// its own bounds ([`Self::upload`], [`Self::download`]).
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let request = self
            .http
            .request(method, format!("{}/{path}", self.base))
            .timeout(PATIENCE);
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }

    /// The response when the hub took the request, else what it refused
    /// with.
    async fn accepted(response: reqwest::Response) -> Result<reqwest::Response, HubError> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.bytes().await.unwrap_or_default();
        let refusal: ApiError = serde_json::from_slice(&body).unwrap_or_else(|_| ApiError {
            error: status
                .canonical_reason()
                .unwrap_or("refused")
                .to_lowercase(),
            message: None,
        });
        Err(HubError::Refused {
            status: status.as_u16(),
            error: refusal.error,
            message: refusal.message,
        })
    }

    async fn json<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, HubError> {
        Ok(Self::accepted(request.send().await?).await?.json().await?)
    }

    async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, HubError> {
        self.json(self.request(reqwest::Method::POST, path).json(body))
            .await
    }

    /// Starts a pairing for a rig that calls itself `rig_name`.
    pub async fn start_pairing(&self, rig_name: &str) -> Result<PairingStarted, HubError> {
        self.post(
            "pairings",
            &PairingRequest {
                rig_name: rig_name.to_string(),
            },
        )
        .await
    }

    /// Asks for the token of the pairing `device_code` started.
    pub async fn poll_pairing(&self, device_code: &str) -> Result<Poll, HubError> {
        let asked: Result<Paired, HubError> = self
            .post(
                "pairings/token",
                &TokenRequest {
                    device_code: device_code.to_string(),
                },
            )
            .await;
        match asked {
            Ok(paired) => Ok(Poll::Paired(paired)),
            Err(HubError::Refused { error, .. }) if error == AUTHORIZATION_PENDING => {
                Ok(Poll::Pending)
            }
            Err(HubError::Refused { error, .. }) if error == SLOW_DOWN => Ok(Poll::SlowDown),
            Err(e) => Err(e),
        }
    }

    /// The project this rig is paired to, and its current revision.
    pub async fn project(&self) -> Result<ProjectState, HubError> {
        self.json(self.request(reqwest::Method::GET, "project"))
            .await
    }

    /// The files of `revision`.
    pub async fn manifest(&self, revision: &Revision) -> Result<Manifest, HubError> {
        self.json(self.request(
            reqwest::Method::GET,
            &format!("revisions/{}/manifest", revision.id),
        ))
        .await
    }

    /// Which of the files with these hashes the hub lacks, and where each
    /// goes.
    pub async fn missing(&self, sha256: Vec<String>) -> Result<Vec<Upload>, HubError> {
        let answer: MissingResponse = self
            .post("blobs/missing", &MissingRequest { sha256 })
            .await?;
        Ok(answer.missing)
    }

    /// Sends the file at `path` where the hub said to.
    pub async fn upload(&self, upload: &Upload, path: &Path, size: u64) -> Result<(), HubError> {
        let file = tokio::fs::File::open(path).await?;
        // Bounded by the file's length, not by a fixed time: a stem takes
        // minutes on a venue's connection.
        let mut request = self
            .http
            .put(&upload.url)
            .timeout(PATIENCE + Duration::from_secs(size / SLOWEST_UPLOAD))
            .header(reqwest::header::CONTENT_LENGTH, size);
        // The token goes to the hub alone: the URL may be storage's own.
        if upload.url.starts_with(&format!("{}/", self.base)) {
            if let Some(token) = &self.token {
                request = request.bearer_auth(token);
            }
        }
        for (name, value) in &upload.headers {
            request = request.header(name, value);
        }
        Self::accepted(request.body(file).send().await?).await?;
        Ok(())
    }

    /// Saves a revision made from `base`: `manifest`, whose files the hub
    /// has.
    pub async fn create_revision(
        &self,
        base: Option<&Revision>,
        manifest: &Manifest,
    ) -> Result<Revision, HubError> {
        let saved = self
            .post(
                "revisions",
                &NewRevision {
                    base: base.map(|revision| revision.id.clone()),
                    manifest: manifest.clone(),
                },
            )
            .await;
        match saved {
            Err(HubError::Refused { error, .. }) if error == NOT_CURRENT => {
                Err(HubError::NotCurrent)
            }
            Err(HubError::Refused { error, message, .. }) if error == INVALID_PROJECT => {
                Err(HubError::InvalidProject(message.unwrap_or(error)))
            }
            other => other,
        }
    }

    /// Fetches the file `entry` names into `to`. The bytes are hashed as
    /// they come, and a file of another hash or length is removed: after an
    /// `Ok`, `to` is the file whole. After an error it is not to be used,
    /// and is gone unless removing it failed too. It may take as long as
    /// bytes keep arriving, and ends when none has for [`PATIENCE`].
    pub async fn download(&self, entry: &FileEntry, to: &Path) -> Result<(), HubError> {
        match tokio::fs::remove_file(to).await {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        let mut response = Self::accepted(
            // No bound on the whole of it: see the loop below.
            self.request(reqwest::Method::GET, &format!("blobs/{}", entry.sha256))
                .timeout(Duration::from_secs(u64::from(u32::MAX)))
                .send()
                .await?,
        )
        .await?;
        if let Some(parent) = to.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut file = tokio::fs::File::create(to).await?;
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        let fetched: Result<(), HubError> = async {
            loop {
                let Ok(next) = tokio::time::timeout(PATIENCE, response.chunk()).await else {
                    return Err(HubError::Io(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "the hub stopped sending",
                    )));
                };
                let Some(chunk) = next? else { break };
                size += chunk.len() as u64;
                if size > entry.size {
                    break;
                }
                hasher.update(&chunk);
                file.write_all(&chunk).await?;
            }
            file.sync_all().await?;
            Ok(())
        }
        .await;
        drop(file);
        let whole = fetched.and_then(|()| {
            if size == entry.size && hex(&hasher.finalize()) == entry.sha256 {
                Ok(())
            } else {
                Err(HubError::Corrupt(entry.sha256.clone()))
            }
        });
        if whole.is_err() {
            let _ = tokio::fs::remove_file(to).await;
        }
        whole
    }

    /// Tells the hub which revision this rig is on.
    pub async fn report(&self, revision: Option<&Revision>) -> Result<(), HubError> {
        Self::accepted(
            self.request(reqwest::Method::PUT, "rig")
                .json(&RigReport {
                    revision: revision.map(|revision| revision.id.clone()),
                })
                .send()
                .await?,
        )
        .await?;
        Ok(())
    }
}
