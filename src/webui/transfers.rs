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

//! File bytes between the web UI and the player: "calls are gRPC; bytes are
//! HTTP" (web UI on gRPC design, section 5).
//!
//! An upload is announced by `BeginUpload` (what it is for, its name and its
//! size, all checked there), which answers an id and a path on the web
//! server. The bytes are `PUT` to that path, into a staging file; the RPC
//! that uses them ([`Transfers::claim`]) takes the file out of the registry,
//! once. A download is staged by the RPC that builds it (`ExportMvr`) and
//! fetched by `GET` until it expires.
//!
//! The registry belongs to the [`Player`](crate::player::Player), which both
//! servers hold, so the gRPC services and the web server reach the same one.
//! A caller on the gRPC port is told a path only the web port serves.
//!
//! Staging files live in a private directory made when the player is, mode
//! `0700`, and are named by the id alone. Nothing a caller sends is ever part
//! of a path here. The directory is removed when the registry is dropped.
//!
//! Time is tokio's, so tests can pause it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::time::Instant;

/// The most uploads and staged downloads at once.
pub const MAX_PENDING: usize = 32;

/// How long an upload not yet sent, a received upload nobody claimed, and a
/// staged download stay.
pub const LIFETIME: Duration = Duration::from_secs(10 * 60);

/// How often the sweeper looks for what has expired.
const SWEEP_EVERY: Duration = Duration::from_secs(30);

/// The most a GDTF or MVR upload may declare: the archive layer refuses
/// anything past 256 MiB, and the transport allowed a little over that.
pub const ARCHIVE_MAX_SIZE: u64 = 272 * 1024 * 1024;

/// What an upload is for. The id is good for this alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// A track of the song.
    Track { song: String },
    /// A sample.
    Sample,
    /// A GDTF to import.
    GdtfImport,
    /// An MVR to inspect.
    MvrInspect,
    /// An MVR to import.
    MvrImport,
}

/// Why a transfer call was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum TransferError {
    /// There are [`MAX_PENDING`] already.
    Full,
    /// No such id: never issued, claimed, expired or discarded.
    NotFound,
    /// The bytes of this upload were sent (or are being) already.
    AlreadySent,
    /// The upload is not what the call needs: not sent yet, sent for another
    /// purpose or song, or not as long as it said it would be.
    Unusable(String),
    /// The staging directory is not there.
    Unavailable(String),
    /// A staging file could not be written or read.
    Io(String),
}

impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransferError::Full => write!(
                f,
                "{MAX_PENDING} uploads and downloads are pending; use or wait for them to expire"
            ),
            TransferError::NotFound => write!(f, "No such upload (it may have expired)"),
            TransferError::AlreadySent => write!(f, "This upload has its bytes already"),
            TransferError::Unusable(why) => write!(f, "{why}"),
            TransferError::Unavailable(why) | TransferError::Io(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for TransferError {}

#[derive(Debug)]
enum UploadState {
    /// Announced; no bytes yet.
    Waiting,
    /// Bytes are arriving.
    Receiving,
    /// The bytes are all in, `length` of them, since `at`.
    Received { length: u64, at: Instant },
}

#[derive(Debug)]
struct Upload {
    purpose: Purpose,
    file_name: String,
    size: u64,
    created: Instant,
    state: UploadState,
}

#[derive(Debug)]
struct Download {
    file_name: String,
    created: Instant,
}

#[derive(Debug)]
enum Entry {
    Upload(Upload),
    Download(Download),
}

impl Entry {
    /// Whether the entry's time is up at `now`. An upload being received
    /// never is: it is not dropped mid-transfer.
    fn expired(&self, now: Instant) -> bool {
        let since = match self {
            Entry::Upload(Upload {
                state: UploadState::Waiting,
                created,
                ..
            })
            | Entry::Download(Download { created, .. }) => *created,
            Entry::Upload(Upload {
                state: UploadState::Received { at, .. },
                ..
            }) => *at,
            Entry::Upload(Upload {
                state: UploadState::Receiving,
                ..
            }) => return false,
        };
        now.saturating_duration_since(since) >= LIFETIME
    }
}

struct Inner {
    /// The private staging directory. `None` when it could not be made.
    dir: Option<tempfile::TempDir>,
    entries: Mutex<HashMap<String, Entry>>,
    sweeper: AtomicBool,
}

impl Inner {
    fn staging_path(&self, id: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|dir| dir.path().join(id))
    }

    /// Drops what has expired, and the files with it.
    fn sweep(&self) {
        let now = Instant::now();
        let mut entries = self.entries.lock();
        let expired: Vec<String> = entries
            .iter()
            .filter(|(_, entry)| entry.expired(now))
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            entries.remove(&id);
            self.remove_file(&id);
        }
    }

    fn remove_file(&self, id: &str) {
        if let Some(path) = self.staging_path(id) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The registry of uploads and staged downloads. Cloning shares it.
#[derive(Clone)]
pub struct Transfers {
    inner: Arc<Inner>,
}

impl Default for Transfers {
    fn default() -> Self {
        Self::new()
    }
}

impl Transfers {
    /// A registry with a staging directory of its own (`0700`).
    ///
    /// It is made under `/var/tmp` where that can be written, and under the
    /// system's temporary directory otherwise. `/tmp` is memory on many
    /// hosts (Debian 13 and its Raspberry Pi images among them), and a set
    /// of tracks waiting to be claimed should not be held in RAM; `/var/tmp`
    /// is disk by convention, and systemd's `PrivateTmp` covers both.
    pub fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let staging = |parent: &Path| {
            tempfile::Builder::new()
                .prefix("mtrack-transfers-")
                .permissions(std::fs::Permissions::from_mode(0o700))
                .tempdir_in(parent)
        };
        let dir = staging(Path::new("/var/tmp"))
            .or_else(|_| staging(&std::env::temp_dir()))
            .inspect_err(|e| tracing::warn!("No staging directory for file transfers: {e}"))
            .ok();
        Transfers {
            inner: Arc::new(Inner {
                dir,
                entries: Mutex::new(HashMap::new()),
                sweeper: AtomicBool::new(false),
            }),
        }
    }

    /// The staging directory, for tests.
    #[cfg(test)]
    pub fn staging_dir(&self) -> Option<&Path> {
        self.inner.dir.as_ref().map(|dir| dir.path())
    }

    /// How many uploads and downloads are pending, for tests.
    #[cfg(test)]
    pub fn pending(&self) -> usize {
        self.inner.entries.lock().len()
    }

    /// Starts the timer that sweeps, once, when there is a runtime to run it
    /// on. It holds the registry weakly and ends with it.
    fn ensure_sweeper(&self) {
        if self.inner.sweeper.load(Ordering::Relaxed) {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self.inner.sweeper.swap(true, Ordering::Relaxed) {
            return;
        }
        let weak: Weak<Inner> = Arc::downgrade(&self.inner);
        runtime.spawn(async move {
            loop {
                tokio::time::sleep(SWEEP_EVERY).await;
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                inner.sweep();
            }
        });
    }

    /// A fresh id: 128 random bits from the thread's CSPRNG (seeded from the
    /// operating system), in hex.
    fn new_id() -> String {
        let mut bytes = [0u8; 16];
        rand::fill(&mut bytes[..]);
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Makes room for one more: sweeps, and refuses when [`MAX_PENDING`] are
    /// held. Returns the lock held, with the id to insert under.
    fn room(
        &self,
    ) -> Result<(parking_lot::MutexGuard<'_, HashMap<String, Entry>>, String), TransferError> {
        self.ensure_sweeper();
        self.inner.sweep();
        let entries = self.inner.entries.lock();
        if entries.len() >= MAX_PENDING {
            return Err(TransferError::Full);
        }
        if self.inner.dir.is_none() {
            return Err(TransferError::Unavailable(
                "No staging directory for file transfers".to_string(),
            ));
        }
        let mut id = Self::new_id();
        while entries.contains_key(&id) {
            id = Self::new_id();
        }
        Ok((entries, id))
    }

    /// Announces an upload of `size` bytes for `purpose`, named
    /// `file_name`; answers its id. Nothing is checked about the name here:
    /// the caller has done it.
    pub fn begin_upload(
        &self,
        purpose: Purpose,
        file_name: String,
        size: u64,
    ) -> Result<String, TransferError> {
        let (mut entries, id) = self.room()?;
        entries.insert(
            id.clone(),
            Entry::Upload(Upload {
                purpose,
                file_name,
                size,
                created: Instant::now(),
                state: UploadState::Waiting,
            }),
        );
        Ok(id)
    }

    /// Opens the upload `id` for its bytes: once. Unknown, expired and
    /// staged downloads are [`TransferError::NotFound`]; an upload that has
    /// its bytes or is getting them is [`TransferError::AlreadySent`].
    pub fn receive(&self, id: &str) -> Result<Receive, TransferError> {
        let mut entries = self.inner.entries.lock();
        let now = Instant::now();
        if entries.get(id).is_some_and(|entry| entry.expired(now)) {
            entries.remove(id);
            self.inner.remove_file(id);
        }
        let Some(Entry::Upload(upload)) = entries.get_mut(id) else {
            return Err(TransferError::NotFound);
        };
        if !matches!(upload.state, UploadState::Waiting) {
            return Err(TransferError::AlreadySent);
        }
        let path = self
            .inner
            .staging_path(id)
            .ok_or_else(|| TransferError::Unavailable("No staging directory".to_string()))?;
        upload.state = UploadState::Receiving;
        Ok(Receive {
            inner: self.inner.clone(),
            id: id.to_string(),
            path,
            size: upload.size,
            done: false,
        })
    }

    /// Takes the uploads `ids` out of the registry, for the call that uses
    /// them. Each must be known, sent whole, and for a purpose `accepts`;
    /// otherwise nothing is taken and the first fault is answered. Once
    /// taken, an upload is spent whatever follows: the files are deleted
    /// when the [`Claimed`] are dropped. One whose bytes are not as many as
    /// it said it would send is taken and refused.
    pub fn claim(
        &self,
        ids: &[String],
        accepts: impl Fn(&Purpose) -> bool,
    ) -> Result<Vec<Claimed>, TransferError> {
        if ids
            .iter()
            .enumerate()
            .any(|(at, id)| ids[..at].contains(id))
        {
            return Err(TransferError::Unusable(
                "An upload is named twice".to_string(),
            ));
        }
        let mut entries = self.inner.entries.lock();
        let now = Instant::now();
        for id in ids {
            if entries.get(id).is_some_and(|entry| entry.expired(now)) {
                entries.remove(id);
                self.inner.remove_file(id);
            }
        }
        for id in ids {
            let Some(Entry::Upload(upload)) = entries.get(id) else {
                return Err(TransferError::NotFound);
            };
            if !accepts(&upload.purpose) {
                return Err(TransferError::Unusable(format!(
                    "This upload was not made for this call ({})",
                    upload.file_name
                )));
            }
            if !matches!(upload.state, UploadState::Received { .. }) {
                return Err(TransferError::Unusable(format!(
                    "The bytes of {} have not all been sent",
                    upload.file_name
                )));
            }
        }
        let mut claimed = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(Entry::Upload(upload)) = entries.remove(id) else {
                continue;
            };
            let UploadState::Received { length, .. } = upload.state else {
                continue;
            };
            claimed.push(Claimed {
                path: self.inner.staging_path(id),
                file_name: upload.file_name,
                purpose: upload.purpose,
                size: upload.size,
                length,
            });
        }
        for c in &claimed {
            if c.length != c.size {
                let why = format!(
                    "{} was announced as {} bytes and {} arrived",
                    c.file_name, c.size, c.length
                );
                // `claimed` drops here, deleting every staged file.
                return Err(TransferError::Unusable(why));
            }
        }
        Ok(claimed)
    }

    /// Stages `bytes` as a download named `file_name`; answers its id. The
    /// download may be fetched until it expires.
    pub fn stage_download(&self, file_name: String, bytes: &[u8]) -> Result<String, TransferError> {
        let (mut entries, id) = self.room()?;
        let path = self
            .inner
            .staging_path(&id)
            .ok_or_else(|| TransferError::Unavailable("No staging directory".to_string()))?;
        std::fs::write(&path, bytes)
            .map_err(|e| TransferError::Io(format!("Failed to stage the download: {e}")))?;
        entries.insert(
            id.clone(),
            Entry::Download(Download {
                file_name,
                created: Instant::now(),
            }),
        );
        Ok(id)
    }

    /// A staged download: its file and the name it carries. Unknown and
    /// expired ids are [`TransferError::NotFound`].
    pub fn download(&self, id: &str) -> Result<(PathBuf, String), TransferError> {
        let mut entries = self.inner.entries.lock();
        let now = Instant::now();
        if entries.get(id).is_some_and(|entry| entry.expired(now)) {
            entries.remove(id);
            self.inner.remove_file(id);
        }
        let Some(Entry::Download(download)) = entries.get(id) else {
            return Err(TransferError::NotFound);
        };
        let path = self
            .inner
            .staging_path(id)
            .ok_or_else(|| TransferError::Unavailable("No staging directory".to_string()))?;
        Ok((path, download.file_name.clone()))
    }

    /// An upload of `bytes` announced, sent and claimed in one step, for the
    /// tests of what uses one.
    #[cfg(test)]
    pub fn stage_for_test(&self, purpose: Purpose, file_name: &str, bytes: &[u8]) -> Claimed {
        let id = self
            .begin_upload(purpose.clone(), file_name.to_string(), bytes.len() as u64)
            .unwrap();
        let receive = self.receive(&id).unwrap();
        std::fs::write(receive.path(), bytes).unwrap();
        receive.finish(bytes.len() as u64);
        self.claim(&[id], |p| *p == purpose).unwrap().pop().unwrap()
    }

    /// Drops what has expired. The timer does it; tests call it.
    #[cfg(test)]
    pub fn sweep(&self) {
        self.inner.sweep();
    }
}

/// `attachment; filename="..."` with an ASCII fallback and, when the name
/// has anything else in it, the RFC 5987 `filename*` form.
pub fn content_disposition(file_name: &str) -> String {
    let ascii: String = file_name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if ascii == file_name {
        return format!("attachment; filename=\"{ascii}\"");
    }
    let mut encoded = String::new();
    for byte in file_name.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

/// An upload being received. The bytes go to [`Receive::path`];
/// [`Receive::finish`] says they are all in. Dropped before that (the
/// request failed, was too long, or the client went away), the upload and
/// its file are discarded.
pub struct Receive {
    inner: Arc<Inner>,
    id: String,
    path: PathBuf,
    size: u64,
    done: bool,
}

impl Receive {
    /// The staging file, named by the id alone.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The size the upload announced.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The bytes are all in: `length` of them.
    pub fn finish(mut self, length: u64) {
        self.done = true;
        if let Some(Entry::Upload(upload)) = self.inner.entries.lock().get_mut(&self.id) {
            upload.state = UploadState::Received {
                length,
                at: Instant::now(),
            };
        }
    }
}

impl Drop for Receive {
    fn drop(&mut self) {
        if !self.done {
            self.inner.entries.lock().remove(&self.id);
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// An upload taken for use. Its staging file is deleted when this is dropped,
/// whether the use succeeded or not.
#[derive(Debug)]
pub struct Claimed {
    path: Option<PathBuf>,
    /// The name the upload announced.
    pub file_name: String,
    /// What it was for.
    pub purpose: Purpose,
    size: u64,
    length: u64,
}

impl Claimed {
    /// How long the file is, as announced and as received (they agree).
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The staging file.
    pub fn path(&self) -> Result<&Path, TransferError> {
        self.path
            .as_deref()
            .ok_or_else(|| TransferError::Unavailable("No staging directory".to_string()))
    }

    /// The whole file.
    pub fn read(&self) -> Result<Vec<u8>, TransferError> {
        std::fs::read(self.path()?)
            .map_err(|e| TransferError::Io(format!("Failed to read the upload: {e}")))
    }

    /// Writes the file to `destination` (made or replaced, with the
    /// permissions a new file gets, not the staging file's).
    pub fn write_to(&self, destination: &Path) -> std::io::Result<u64> {
        let mut source = std::fs::File::open(self.path().map_err(std::io::Error::other)?)?;
        let mut target = std::fs::File::create(destination)?;
        std::io::copy(&mut source, &mut target)
    }
}

impl Drop for Claimed {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests;
