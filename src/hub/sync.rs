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

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mtrack_core::project::{self, hash_file, Manifest, Report};

use super::apply::{self, changed, file_at, Journal, Source};
use super::client::{Client, HubError, Poll};
use super::protocol::{PairingStarted, Revision, ACCESS_DENIED, EXPIRED_TOKEN};
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
    /// The project here has files a copy made from the hub could not have.
    #[error("this project cannot be uploaded as it is: {0}")]
    NotUploadable(String),
    /// The hub's revision is not a project: nothing of it was put in place.
    #[error("the hub's revision is not one this rig can take: {0}")]
    BadRevision(String),
    /// The hub's revision does not load with this player: nothing of it was
    /// put in place.
    #[error("the hub's revision does not load, so the project was left as it was: {0}")]
    RevisionDoesNotLoad(String),
    /// Files here were changed since they were last the hub's, and what was
    /// asked would replace them.
    #[error(
        "files in this project were changed since it was last pulled or uploaded. Upload \
         them, or run this again with --discard-local-changes (what that replaces is kept \
         until the next pull, for `undo`)"
    )]
    LocalChanges,
    /// Files here were written while the pull was fetching.
    #[error(
        "files in this project were written while the pull was fetching, so nothing was put \
         in place. Stop what is writing them and pull again"
    )]
    ChangedMeanwhile,
    /// The directory holds files and no player config: not a project to
    /// replace.
    #[error(
        "{0} holds files and no player config, so it does not look like a project. Nothing \
         was pulled into it"
    )]
    NotAProject(String),
    /// The hub has nothing to pull.
    #[error("nothing has been uploaded to this project yet")]
    NothingToPull,
    /// The pairing code was not entered in time, or was refused.
    #[error("the pairing was not completed: {0}")]
    NotPairedInTime(&'static str),
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
    /// A new revision; `sent` files had to be sent for it. `report` is what
    /// loading the project here found.
    Saved {
        revision: Revision,
        sent: usize,
        report: Report,
    },
}

/// What a pull did.
#[derive(Debug, PartialEq, Eq)]
pub enum Pulled {
    /// The project's files were already the hub's current revision.
    UpToDate(Revision),
    /// The project is now at `revision`; `fetched` files had to be fetched.
    /// `report` is what loading it found before it was put in place.
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

/// The least a rig waits between asks for a pairing's token, whatever the
/// hub says.
const LEAST_POLL: Duration = if cfg!(test) {
    Duration::from_millis(5)
} else {
    Duration::from_secs(1)
};

/// The longest a rig waits for a pairing code to be entered, whatever the
/// hub says.
const LONGEST_PAIRING: Duration = Duration::from_secs(60 * 60);

impl Rig {
    /// The paired project whose player config is at `config_path`.
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

    fn config_name(&self) -> &str {
        self.config_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("mtrack.yaml")
    }

    /// Finishes a pull that was cut short, if one was.
    fn recovered(&self) -> Result<(), SyncError> {
        if apply::recover(&self.state)? {
            tracing::info!("Finished a pull that was cut short");
        }
        Ok(())
    }

    /// The project's files as they are, a cut-short pull finished first.
    fn local(&self) -> Result<Manifest, SyncError> {
        self.recovered()?;
        Ok(Manifest::of_dir(self.project())?)
    }

    fn changed_since(recorded: Option<&SyncState>, local: &Manifest) -> bool {
        match recorded {
            Some(recorded) => recorded.digest != local.digest(),
            // Never the hub's: any file here is this rig's own.
            None => !local.files.is_empty(),
        }
    }

    /// Tells the hub which revision this rig is on. The rig is on it whether
    /// or not the hub hears, so a hub that cannot be told is only logged.
    async fn report(&self, revision: Option<&Revision>) {
        if let Err(e) = self.client.report(revision).await {
            tracing::warn!("The hub was not told which revision this rig is on: {e}");
        }
    }

    /// Where the project stands, without asking the hub. A pull that was
    /// cut short is finished first.
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
    /// the hub lacks, then the manifest. After it there is no pull to undo. Not sent: a project whose config
    /// does not load here, and one a copy made from the hub could not be
    /// the same as (a file a song uses from outside the project, two names
    /// that differ only by case).
    pub async fn upload(&self) -> Result<Uploaded, SyncError> {
        let _lock = self.state.lock()?;
        let local = self.local()?;
        let report =
            project::check(&self.config_path).map_err(|e| SyncError::DoesNotLoad(e.to_string()))?;
        if let Some(outside) = report.outside.first() {
            return Err(SyncError::NotUploadable(format!(
                "a song uses {}, which is not one of the project's files (it is outside the \
                 project's folder, or under a name starting with a dot)",
                outside.display()
            )));
        }
        // What another rig will check before it takes the revision.
        local
            .validate()
            .map_err(|e| SyncError::NotUploadable(e.to_string()))?;

        let remote = self.client.project().await?;
        if let Some(current) = &remote.revision {
            if self.client.manifest(current).await? == local {
                self.state.record(Some(current.clone()), &local)?;
                self.report(Some(current)).await;
                return Ok(Uploaded::UpToDate(current.clone()));
            }
        }

        let mut hashes: Vec<String> = local.files.values().map(|e| e.sha256.clone()).collect();
        hashes.sort();
        hashes.dedup();
        let missing = self.client.missing(hashes).await?;
        let mut sent = 0;
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
                return Err(SyncError::ChangedMeanwhile);
            }
            tracing::info!(path, size = entry.size, "Uploading");
            self.client.upload(upload, &file, entry.size).await?;
            sent += 1;
        }

        let base = self.state.sync_state()?.and_then(|state| state.revision);
        let revision = self.client.create_revision(base.as_ref(), &local).await?;
        self.state.record(Some(revision.clone()), &local)?;
        // The project has moved on from the pull an undo would take back:
        // undoing it now would put old files under a revision made since.
        remove_dir_all(&self.state.previous())?;
        self.report(Some(&revision)).await;
        Ok(Uploaded::Saved {
            revision,
            sent,
            report,
        })
    }

    /// Brings the project to the hub's current revision. Files changed here
    /// since the last pull or upload stop it unless `discard_local_changes`;
    /// a revision that is not a project, or whose config does not load with
    /// this player, stops it with nothing in the project touched.
    pub async fn pull(&self, discard_local_changes: bool) -> Result<Pulled, SyncError> {
        let _lock = self.state.lock()?;
        self.recovered()?;
        // Taken before the files are read and again before any is moved. The
        // same both times, no file was added, removed, or written in a way
        // that shows in its length or time: what a player saving a song
        // does, which is what this is for.
        let stamps = project::stamps(self.project())?;
        let local = Manifest::of_dir(self.project())?;
        let recorded = self.state.sync_state()?;

        let remote = self.client.project().await?;
        let revision = remote.revision.ok_or(SyncError::NothingToPull)?;
        let manifest = self.client.manifest(&revision).await?;
        manifest
            .validate()
            .map_err(|e| SyncError::BadRevision(e.to_string()))?;
        let config_name = self.config_name();
        if !manifest.files.contains_key(config_name) {
            return Err(SyncError::BadRevision(format!(
                "it has no player config named {config_name}"
            )));
        }
        if manifest == local {
            self.state.record(Some(revision.clone()), &local)?;
            self.report(Some(&revision)).await;
            return Ok(Pulled::UpToDate(revision));
        }
        if Self::changed_since(recorded.as_ref(), &local) {
            if !discard_local_changes {
                return Err(SyncError::LocalChanges);
            }
            // Files that were never the hub's and are not a project's: not
            // something to replace on anyone's say-so.
            if recorded.is_none() && !local.files.contains_key(config_name) {
                return Err(SyncError::NotAProject(self.project().display().to_string()));
            }
        }
        apply::preflight(&self.state, &local, &manifest)?;

        // Fetch what differs. A file staged by a pull that did not finish is
        // kept when it is one this pull wants, so a pull picks up where one
        // left off; anything else staged is not this pull's and goes.
        let staged = self.state.staging().join("files");
        let wanted: BTreeSet<&String> = changed(&local, &manifest).collect();
        if staged.is_dir() {
            // Whatever else is there is not looked at, only cleared.
            let keep = self.state.staging().join("keep");
            remove_dir_all(&keep)?;
            for path in &wanted {
                let left = file_at(&staged, path);
                let whole = std::fs::symlink_metadata(&left).is_ok_and(|m| m.is_file())
                    && hash_file(&left).ok().as_ref() == manifest.files.get(*path);
                if whole {
                    let to = file_at(&keep, path);
                    if let Some(parent) = to.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::rename(&left, &to)?;
                }
            }
            remove_dir_all(&staged)?;
            if keep.is_dir() {
                std::fs::rename(&keep, &staged)?;
            }
        }
        let mut fetched = 0;
        for path in &wanted {
            let entry = &manifest.files[*path];
            let to = file_at(&staged, path);
            if to.is_file() {
                continue;
            }
            tracing::info!(path = path.as_str(), size = entry.size, "Fetching");
            self.client
                .download(entry, &to)
                .await
                .map_err(|e| match e {
                    HubError::Corrupt(_) => HubError::Corrupt((*path).clone()),
                    other => other,
                })?;
            fetched += 1;
        }

        let report = self.check_as_it_would_be(&manifest, &wanted, &staged)?;

        if project::stamps(self.project())? != stamps {
            return Err(SyncError::ChangedMeanwhile);
        }
        apply::apply(
            &self.state,
            &Journal {
                source: Source::Staging,
                state: Some(SyncState {
                    revision: Some(revision.clone()),
                    digest: manifest.digest(),
                }),
                manifest,
                before: local,
                state_before: recorded,
                moved: false,
            },
        )?;
        self.report(Some(&revision)).await;
        Ok(Pulled::Updated {
            revision,
            fetched,
            report,
        })
    }

    /// Loads the project as it would be once `manifest` is in place, with
    /// the player's own loader, before anything is: a tree of links, to the
    /// staged file for each path the pull fetched and else to the project's
    /// own. What it reports is by the path the file will have.
    fn check_as_it_would_be(
        &self,
        manifest: &Manifest,
        wanted: &BTreeSet<&String>,
        staged: &PathBuf,
    ) -> Result<Report, SyncError> {
        let view = self.state.staging().join("view");
        remove_dir_all(&view)?;
        for path in manifest.files.keys() {
            let source = if wanted.contains(path) {
                file_at(staged, path)
            } else {
                file_at(self.project(), path)
            };
            let link = file_at(&view, path);
            if let Some(parent) = link.parent() {
                std::fs::create_dir_all(parent)?;
            }
            link_or_copy(&std::path::absolute(&source)?, &link)?;
        }
        let checked = project::check(&view.join(self.config_name()))
            .map_err(|e| SyncError::RevisionDoesNotLoad(e.to_string()));
        // Where the files will be, not where they were looked at: the
        // loader names a file by where its link leads, which for a fetched
        // one is the staging folder.
        let seen_at = [&view, staged].map(|dir| dir.canonicalize().unwrap_or_else(|_| dir.clone()));
        let _ = std::fs::remove_dir_all(&view);
        checked.map(|mut report| {
            for (file, _) in &mut report.problems {
                if let Some(relative) = seen_at.iter().find_map(|dir| file.strip_prefix(dir).ok()) {
                    *file = self.project().join(relative);
                }
            }
            // The links are to files of the project or of the pull: none is
            // outside it, whatever resolving them says.
            report.outside.clear();
            report
        })
    }

    /// Puts back what the last pull replaced, with no network: the files it
    /// changed or removed as they were, and the files it added gone. Files
    /// changed here since the pull stop it unless `discard_local_changes`;
    /// with it, those the pull had touched are replaced, and those it had
    /// not (a file made here since, say) stay as they are.
    pub async fn undo(&self, discard_local_changes: bool) -> Result<Option<Revision>, SyncError> {
        let _lock = self.state.lock()?;
        let local = self.local()?;
        let recorded = self.state.sync_state()?;
        if Self::changed_since(recorded.as_ref(), &local) && !discard_local_changes {
            return Err(SyncError::LocalChanges);
        }
        let back =
            apply::undo(&self.state, recorded, &local)?.and_then(|recorded| recorded.revision);
        self.report(back.as_ref()).await;
        Ok(back)
    }
}

/// Removes `dir` and all in it; one that is not there is no error.
fn remove_dir_all(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
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

/// Finishes a pull of the project whose player config is at `config_path`
/// that was cut short, if one was: for the player to call before it loads
/// the project, so it does not start on one part-way between two revisions
/// when that can be helped. True when there was one. An error is a pull
/// that is still unfinished; the caller decides whether to start anyway. Nothing is done, or made, for a project that is
/// not paired.
pub fn recover(config_path: &Path) -> Result<bool, SyncError> {
    let state = StateDir::of(&mtrack_core::util::project_dir_of(config_path));
    if !apply::pending(&state) {
        return Ok(false);
    }
    let _lock = state.lock()?;
    Ok(apply::recover(&state)?)
}

/// Pairs the project whose player config is at `config_path` with the hub at
/// `hub`: starts a pairing, hands its code to `show` for a person to enter,
/// and waits until they have or the code runs out. A project paired before
/// with another hub or project starts afresh: what it was recorded as, and
/// its undo, were about that one.
pub async fn pair(
    config_path: &Path,
    hub: &str,
    rig_name: &str,
    show: impl FnOnce(&PairingStarted),
) -> Result<Credentials, SyncError> {
    let state = StateDir::of(&mtrack_core::util::project_dir_of(config_path));
    let _lock = state.lock()?;
    apply::recover(&state)?;
    let client = Client::new(hub, None)?;
    let started = client.start_pairing(rig_name).await?;
    show(&started);

    let mut interval = Duration::from_secs(started.interval).max(LEAST_POLL);
    let patience = Duration::from_secs(started.expires_in).min(LONGEST_PAIRING);
    let deadline = tokio::time::Instant::now() + patience;
    let paired = loop {
        tokio::time::sleep(interval).await;
        match client.poll_pairing(&started.device_code).await {
            Ok(Poll::Paired(paired)) => break paired,
            Ok(Poll::Pending) => {}
            Ok(Poll::SlowDown) => interval += Duration::from_secs(5),
            Err(HubError::Refused { error, .. }) if error == EXPIRED_TOKEN => {
                return Err(SyncError::NotPairedInTime(
                    "the code ran out before it was entered",
                ));
            }
            Err(HubError::Refused { error, .. }) if error == ACCESS_DENIED => {
                return Err(SyncError::NotPairedInTime("it was refused in the hub"));
            }
            Err(e) => return Err(e.into()),
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(SyncError::NotPairedInTime(
                "the code ran out before it was entered",
            ));
        }
    };
    let credentials = Credentials {
        hub: hub.trim_end_matches('/').to_string(),
        token: paired.token,
        rig: paired.rig,
        project: paired.project,
    };
    let same = state.credentials()?.is_some_and(|before| {
        before.hub == credentials.hub && before.project.id == credentials.project.id
    });
    if !same {
        state.forget_sync()?;
    }
    state.set_credentials(&credentials)?;
    Ok(credentials)
}

#[cfg(test)]
mod tests;
