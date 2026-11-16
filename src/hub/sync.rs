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
//! Upload and pull: a rig's project against the hub's.

use std::path::{Path, PathBuf};

use mtrack_core::project::{self, hash_file, Manifest, Report};

use super::apply::{self, changed, file_at, Journal};
use super::client::{Client, HubError};
use super::protocol::Revision;
use super::state::{Credentials, StateDir, SyncState};

/// What went wrong with an upload or a pull.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("this project is not paired with a hub")]
    NotPaired,
    #[error(transparent)]
    Hub(#[from] HubError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Scan(#[from] project::ScanError),
    /// The project here does not load, so it is not sent.
    #[error("this project does not load, so it was not uploaded: {0}")]
    DoesNotLoad(String),
    /// The hub's revision is not a project: nothing of it was put in place.
    #[error("the hub's revision is not one this rig can take: {0}")]
    BadRevision(String),
    /// The hub's revision does not load with this player: nothing of it was
    /// put in place.
    #[error("the hub's revision does not load, so the project was left as it was: {0}")]
    RevisionDoesNotLoad(String),
    /// Files here were changed since they were last the hub's. A pull would
    /// replace them.
    #[error(
        "files in this project were changed since it was last pulled or uploaded; \
         upload them, or pull with --discard-local-changes (they are kept for one undo)"
    )]
    LocalChanges,
    /// The hub has nothing to pull.
    #[error("nothing has been uploaded to this project yet")]
    NothingToPull,
}

/// A paired project on this rig.
pub struct Rig {
    config_path: PathBuf,
    state: StateDir,
    credentials: Credentials,
    client: Client,
}

/// What an upload did.
#[derive(Debug, PartialEq, Eq)]
pub enum Uploaded {
    /// The hub already had exactly these files as its current revision.
    UpToDate(Revision),
    /// A new revision, and how many files had to be sent for it.
    Saved { revision: Revision, sent: usize },
}

/// What a pull did.
#[derive(Debug, PartialEq, Eq)]
pub enum Pulled {
    /// The project's files were already the hub's current revision.
    UpToDate(Revision),
    /// The project is now at `revision`; `fetched` files had to be fetched.
    Updated {
        revision: Revision,
        fetched: usize,
        report: Report,
    },
}

/// Where a project stands, for `mtrack hub status`.
#[derive(Debug)]
pub struct Status {
    pub credentials: Credentials,
    /// The revision the files were last at, if any.
    pub revision: Option<Revision>,
    /// Whether the files have changed since.
    pub local_changes: bool,
    /// Whether a pull can be undone.
    pub undoable: bool,
}

impl Rig {
    /// The paired project whose player config is at `config_path`. Finishes
    /// a pull that was cut short, if one was, before anything else is done.
    pub fn open(config_path: &Path) -> Result<Rig, SyncError> {
        let state = StateDir::of(&mtrack_core::util::project_dir_of(config_path));
        let credentials = state.credentials()?.ok_or(SyncError::NotPaired)?;
        let client = Client::new(&credentials.hub, Some(credentials.token.clone()))?;
        Ok(Rig {
            config_path: config_path.to_path_buf(),
            state,
            credentials,
            client,
        })
    }

    fn project(&self) -> &Path {
        self.state.project()
    }

    /// The project's files as they are, a cut-short pull finished first.
    fn local(&self) -> Result<Manifest, SyncError> {
        if apply::recover(&self.state)? {
            tracing::info!("Finished a pull that was cut short");
        }
        Ok(Manifest::of_dir(self.project())?)
    }

    fn changed_since(recorded: Option<&SyncState>, local: &Manifest) -> bool {
        match recorded {
            Some(recorded) => recorded.digest != local.digest(),
            // Never the hub's: any file here is this rig's own.
            None => !local.files.is_empty(),
        }
    }

    /// Where the project stands, without asking the hub.
    pub fn status(&self) -> Result<Status, SyncError> {
        let _lock = self.state.lock()?;
        let local = self.local()?;
        let recorded = self.state.sync_state()?;
        Ok(Status {
            credentials: self.credentials.clone(),
            local_changes: Self::changed_since(recorded.as_ref(), &local),
            revision: recorded.and_then(|recorded| recorded.revision),
            undoable: apply::undoable(&self.state)?.is_some(),
        })
    }

    /// Sends this rig's project to the hub as its next revision: the files
    /// the hub lacks, then the manifest. A project that does not load here
    /// is not sent.
    pub async fn upload(&self) -> Result<Uploaded, SyncError> {
        let _lock = self.state.lock()?;
        let local = self.local()?;
        project::check(&self.config_path).map_err(|e| SyncError::DoesNotLoad(e.to_string()))?;

        let remote = self.client.project().await?;
        if let Some(current) = &remote.revision {
            if self.client.manifest(current).await? == local {
                self.state.record(Some(current.clone()), &local)?;
                self.client.report(Some(current)).await?;
                return Ok(Uploaded::UpToDate(current.clone()));
            }
        }

        let mut hashes: Vec<String> = local.files.values().map(|e| e.sha256.clone()).collect();
        hashes.sort();
        hashes.dedup();
        let missing = self.client.missing(hashes).await?;
        for upload in &missing {
            // Any file of that hash: they are the same bytes.
            let Some((path, entry)) = local
                .files
                .iter()
                .find(|(_, entry)| entry.sha256 == upload.sha256)
            else {
                continue;
            };
            let file = file_at(self.project(), path);
            // The file as it is now, which a write since the scan may have
            // changed: the manifest must name what was sent.
            if &hash_file(&file)? != entry {
                return Err(SyncError::Io(std::io::Error::other(format!(
                    "{path} changed while it was being uploaded"
                ))));
            }
            tracing::info!(path, size = entry.size, "Uploading");
            self.client.upload(upload, &file, entry.size).await?;
        }

        let base = self.state.sync_state()?.and_then(|state| state.revision);
        let revision = self.client.create_revision(base.as_ref(), &local).await?;
        self.state.record(Some(revision.clone()), &local)?;
        self.client.report(Some(&revision)).await?;
        Ok(Uploaded::Saved {
            revision,
            sent: missing.len(),
        })
    }

    /// Brings the project to the hub's current revision. Files changed here
    /// since the last pull or upload stop it unless `discard_local_changes`;
    /// a revision that is not a project, or does not load with this player,
    /// stops it with nothing in the project touched.
    pub async fn pull(&self, discard_local_changes: bool) -> Result<Pulled, SyncError> {
        let _lock = self.state.lock()?;
        let local = self.local()?;
        let recorded = self.state.sync_state()?;

        let remote = self.client.project().await?;
        let revision = remote.revision.ok_or(SyncError::NothingToPull)?;
        let manifest = self.client.manifest(&revision).await?;
        manifest
            .validate()
            .map_err(|e| SyncError::BadRevision(e.to_string()))?;
        if manifest == local {
            self.state.record(Some(revision.clone()), &local)?;
            self.client.report(Some(&revision)).await?;
            return Ok(Pulled::UpToDate(revision));
        }
        if Self::changed_since(recorded.as_ref(), &local) && !discard_local_changes {
            return Err(SyncError::LocalChanges);
        }

        // Fetch what differs. A file staged by a pull that did not finish is
        // kept when it is the one wanted, so a pull picks up where one left
        // off.
        let staged = self.state.staging().join("files");
        let wanted: Vec<&String> = changed(&local, &manifest).collect();
        let mut fetched = 0;
        for path in &wanted {
            let entry = &manifest.files[*path];
            let to = file_at(&staged, path);
            if to.is_file() && hash_file(&to).ok().as_ref() == Some(entry) {
                continue;
            }
            tracing::info!(path = path.as_str(), size = entry.size, "Fetching");
            self.client.download(entry, &to).await?;
            fetched += 1;
        }

        let report = self.check_as_it_would_be(&manifest, &staged)?;

        apply::apply(
            &self.state,
            &Journal {
                revision: Some(revision.clone()),
                manifest: manifest.clone(),
                before: local,
                revision_before: recorded.and_then(|recorded| recorded.revision),
            },
        )?;
        self.client.report(Some(&revision)).await?;
        Ok(Pulled::Updated {
            revision,
            fetched,
            report,
        })
    }

    /// Loads the project as it would be once `manifest` is in place, with
    /// the player's own loader, before anything is: a tree of links, to the
    /// staged file where there is one and else to the project's own.
    fn check_as_it_would_be(
        &self,
        manifest: &Manifest,
        staged: &Path,
    ) -> Result<Report, SyncError> {
        let config_name = self
            .config_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("mtrack.yaml");
        if !manifest.files.contains_key(config_name) {
            return Err(SyncError::BadRevision(format!(
                "it has no player config named {config_name}"
            )));
        }
        let view = self.state.staging().join("view");
        match std::fs::remove_dir_all(&view) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        for path in manifest.files.keys() {
            let incoming = file_at(staged, path);
            let source = if incoming.is_file() {
                incoming
            } else {
                file_at(self.project(), path)
            };
            let link = file_at(&view, path);
            if let Some(parent) = link.parent() {
                std::fs::create_dir_all(parent)?;
            }
            link_or_copy(&std::path::absolute(&source)?, &link)?;
        }
        let checked = project::check(&view.join(config_name))
            .map_err(|e| SyncError::RevisionDoesNotLoad(e.to_string()));
        let _ = std::fs::remove_dir_all(&view);
        checked
    }

    /// Puts back what the last pull replaced, with no network. The hub is
    /// told which revision the rig is on when it can be reached.
    pub async fn undo(&self, discard_local_changes: bool) -> Result<Option<Revision>, SyncError> {
        let _lock = self.state.lock()?;
        let local = self.local()?;
        let recorded = self.state.sync_state()?;
        if Self::changed_since(recorded.as_ref(), &local) && !discard_local_changes {
            return Err(SyncError::LocalChanges);
        }
        let back = apply::undo(
            &self.state,
            recorded.and_then(|recorded| recorded.revision),
            &local,
        )?;
        if let Err(e) = self.client.report(back.as_ref()).await {
            tracing::warn!("The hub was not told of the undo: {e}");
        }
        Ok(back)
    }
}

#[cfg(unix)]
fn link_or_copy(source: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, link)
}

#[cfg(not(unix))]
fn link_or_copy(source: &Path, link: &Path) -> std::io::Result<()> {
    std::fs::copy(source, link).map(|_| ())
}

/// Pairs the project whose player config is at `config_path` with the hub at
/// `hub`: starts a pairing, hands its code to `show` for a person to enter,
/// and waits until they have or the code runs out.
pub async fn pair(
    config_path: &Path,
    hub: &str,
    rig_name: &str,
    show: impl FnOnce(&super::protocol::PairingStarted),
) -> Result<Credentials, SyncError> {
    let state = StateDir::of(&mtrack_core::util::project_dir_of(config_path));
    let _lock = state.lock()?;
    let client = Client::new(hub, None)?;
    let started = client.start_pairing(rig_name).await?;
    show(&started);

    let mut interval = std::time::Duration::from_secs(started.interval);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(started.expires_in);
    let paired = loop {
        tokio::time::sleep(interval).await;
        match client.poll_pairing(&started.device_code).await? {
            super::client::Poll::Paired(paired) => break paired,
            super::client::Poll::Pending => {}
            super::client::Poll::SlowDown => interval += std::time::Duration::from_secs(5),
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(HubError::Refused {
                status: 400,
                error: super::protocol::EXPIRED_TOKEN.to_string(),
                message: Some("the pairing code ran out before it was entered".to_string()),
            }
            .into());
        }
    };
    let credentials = Credentials {
        hub: hub.trim_end_matches('/').to_string(),
        token: paired.token,
        rig: paired.rig,
        project: paired.project,
    };
    state.set_credentials(&credentials)?;
    Ok(credentials)
}

#[cfg(test)]
mod tests;
