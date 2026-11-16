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
//! What a rig keeps about the hub, in `.mtrack-hub` in the project's
//! directory: a name the project's own manifest leaves out, inside the one
//! directory a packaged player may write to.
//!
//! | File | Holds |
//! | --- | --- |
//! | `rig.json` | The hub's address, this rig's token, and what it is paired to. |
//! | `state.json` | The revision the project's files are at. |
//! | `lock` | Held while a hub command runs. |
//! | `staging/` | Files fetched for a pull, until they are put in place. |
//! | `displaced/` | Files a pull under way has taken out of the project. |
//! | `previous/` | What the last pull replaced, to undo it. |
//! | `journal.json` | A pull's moves, while they are being made. |

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use mtrack_core::project::Manifest;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::protocol::{Named, Revision};

/// The directory's name.
pub const DIR: &str = ".mtrack-hub";

/// The hub this rig is paired with.
#[derive(Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub hub: String,
    pub token: String,
    pub rig: Named,
    pub project: Named,
}

// By hand, to leave the token out of anything printed.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("hub", &self.hub)
            .field("rig", &self.rig)
            .field("project", &self.project)
            .finish_non_exhaustive()
    }
}

/// Where the project's files stand against the hub.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    /// The hub's revision the files were last made the same as, or were
    /// uploaded as. None before either.
    pub revision: Option<Revision>,
    /// The digest of the project's manifest then
    /// ([`Manifest::digest`]): files that no longer have it were changed on
    /// this rig since.
    pub digest: String,
}

/// A rig's `.mtrack-hub` directory.
#[derive(Clone, Debug)]
pub struct StateDir {
    project: PathBuf,
    dir: PathBuf,
}

/// Held for as long as a hub command runs; dropping it lets the next run.
pub struct Lock(#[allow(dead_code)] File);

impl StateDir {
    /// The state directory of the project in `project`. Nothing is made
    /// until something is written.
    pub fn of(project: &Path) -> StateDir {
        StateDir {
            project: project.to_path_buf(),
            dir: project.join(DIR),
        }
    }

    /// The project's directory.
    pub fn project(&self) -> &Path {
        &self.project
    }

    /// The state directory itself.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn staging(&self) -> PathBuf {
        self.dir.join("staging")
    }

    pub fn displaced(&self) -> PathBuf {
        self.dir.join("displaced")
    }

    pub fn previous(&self) -> PathBuf {
        self.dir.join("previous")
    }

    pub fn journal(&self) -> PathBuf {
        self.dir.join("journal.json")
    }

    /// Takes the lock, or says another hub command holds it.
    pub fn lock(&self) -> std::io::Result<Lock> {
        std::fs::create_dir_all(&self.dir)?;
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join("lock"))?;
        match file.try_lock() {
            Ok(()) => Ok(Lock(file)),
            Err(std::fs::TryLockError::WouldBlock) => Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "another mtrack hub command is running on this project",
            )),
            Err(std::fs::TryLockError::Error(e)) => Err(e),
        }
    }

    pub fn credentials(&self) -> std::io::Result<Option<Credentials>> {
        read_json(&self.dir.join("rig.json"))
    }

    /// Saves the rig's token, readable by its owner alone.
    pub fn set_credentials(&self, credentials: &Credentials) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        write_json(&self.dir.join("rig.json"), credentials, true)
    }

    pub fn sync_state(&self) -> std::io::Result<Option<SyncState>> {
        read_json(&self.dir.join("state.json"))
    }

    pub fn set_sync_state(&self, state: &SyncState) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        write_json(&self.dir.join("state.json"), state, false)
    }

    /// Forgets what the project was recorded as: it is no revision's.
    pub fn clear_sync_state(&self) -> std::io::Result<()> {
        match std::fs::remove_file(self.dir.join("state.json")) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Forgets everything about the hub but the pairing: the recorded
    /// revision, the undo, and anything a pull left half-fetched. For a
    /// project paired anew, which is not the one those were about.
    pub fn forget_sync(&self) -> std::io::Result<()> {
        self.clear_sync_state()?;
        for dir in [self.staging(), self.displaced(), self.previous()] {
            match std::fs::remove_dir_all(dir) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }

    /// Records the project's files, as they are now, as `revision`.
    pub fn record(&self, revision: Option<Revision>, manifest: &Manifest) -> std::io::Result<()> {
        self.set_sync_state(&SyncState {
            revision,
            digest: manifest.digest(),
        })
    }
}

/// The JSON at `path`, or None when there is no file.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> std::io::Result<Option<T>> {
    let text = match std::fs::read(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    serde_json::from_slice(&text).map(Some).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{}: {e}", path.display()),
        )
    })
}

/// Writes `value` to `path` whole or not at all: to a file beside it, synced
/// and renamed over it. `private` makes it readable by its owner alone.
pub fn write_json<T: Serialize>(path: &Path, value: &T, private: bool) -> std::io::Result<()> {
    let text = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    let staged = path.with_extension("json.new");
    let mut options = File::options();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options.open(&staged)?;
    file.write_all(&text)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&staged, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credentials() -> Credentials {
        Credentials {
            hub: "https://hub.example".into(),
            token: "secret".into(),
            rig: Named {
                id: "r1".into(),
                name: "Stage left".into(),
            },
            project: Named {
                id: "p1".into(),
                name: "The show".into(),
            },
        }
    }

    #[test]
    fn nothing_is_there_until_it_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateDir::of(dir.path());
        assert!(state.credentials().unwrap().is_none());
        assert!(state.sync_state().unwrap().is_none());
        assert!(!dir.path().join(DIR).exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_token_is_its_owners_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let state = StateDir::of(dir.path());
        state.set_credentials(&credentials()).unwrap();
        let mode = std::fs::metadata(dir.path().join(DIR).join("rig.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(state.credentials().unwrap().unwrap().token, "secret");
    }

    #[test]
    fn the_state_directory_is_not_part_of_the_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mtrack.yaml"), "songs: songs\n").unwrap();
        StateDir::of(dir.path())
            .set_credentials(&credentials())
            .unwrap();
        let manifest = Manifest::of_dir(dir.path()).unwrap();
        assert_eq!(manifest.files.keys().collect::<Vec<_>>(), ["mtrack.yaml"]);
    }

    #[test]
    fn one_hub_command_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateDir::of(dir.path());
        let held = state.lock().unwrap();
        assert_eq!(
            state.lock().err().map(|e| e.kind()),
            Some(std::io::ErrorKind::WouldBlock)
        );
        drop(held);
        assert!(state.lock().is_ok());
    }
}
