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

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use yaml_rust2::{Yaml, YamlEmitter};

/// Extracts a displayable file name from a path, returning a fallback if the name is unreadable.
pub fn filename_display(path: &Path) -> &str {
    path.file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("unreadable file name")
}

/// Outputs the given duration in a minutes:seconds format.
pub fn duration_minutes_seconds(duration: Duration) -> String {
    let minutes = duration.as_secs() / 60;
    let secs = duration.as_secs() - minutes * 60;
    format!("{}:{:02}", minutes, secs)
}

/// Converts a string to kebab-case. Handles spaces, underscores, camelCase,
/// and PascalCase by inserting hyphens at word boundaries and lowercasing.
pub fn to_kebab_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::with_capacity(s.len() + 4);
    let mut prev_was_separator = false;

    for (i, &ch) in chars.iter().enumerate() {
        if ch == ' ' || ch == '_' || ch == '-' {
            if !result.is_empty() && !prev_was_separator {
                result.push('-');
            }
            prev_was_separator = true;
        } else if ch.is_uppercase() {
            // Insert hyphen before uppercase if preceded by a lowercase letter/digit,
            // or if preceded by uppercase followed by lowercase (e.g., "XMLParser" -> "xml-parser").
            if !result.is_empty() && !prev_was_separator {
                let prev = chars[i - 1];
                if prev.is_lowercase()
                    || prev.is_ascii_digit()
                    || (prev.is_uppercase()
                        && chars.get(i + 1).is_some_and(|next| next.is_lowercase()))
                {
                    result.push('-');
                }
            }
            for lc in ch.to_lowercase() {
                result.push(lc);
            }
            prev_was_separator = false;
        } else {
            result.push(ch);
            prev_was_separator = false;
        }
    }

    // Strip trailing hyphen from trailing separators in input.
    while result.ends_with('-') {
        result.pop();
    }

    result
}

/// Serializes a value to a YAML string using serde_json as an intermediary and yaml-rust2 for
/// emission.
pub fn to_yaml_string<T: Serialize>(value: &T) -> Result<String, Box<dyn std::error::Error>> {
    let json_value = serde_json::to_value(value)?;
    let yaml = json_to_yaml(&json_value);
    let mut out = String::new();
    let mut emitter = YamlEmitter::new(&mut out);
    emitter.dump(&yaml)?;
    Ok(out)
}

fn json_to_yaml(value: &serde_json::Value) -> Yaml {
    match value {
        serde_json::Value::Null => Yaml::Null,
        serde_json::Value::Bool(b) => Yaml::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Yaml::Integer(i)
            } else {
                Yaml::Real(n.to_string())
            }
        }
        serde_json::Value::String(s) => Yaml::String(s.clone()),
        serde_json::Value::Array(arr) => Yaml::Array(arr.iter().map(json_to_yaml).collect()),
        serde_json::Value::Object(obj) => {
            let mut hash = yaml_rust2::yaml::Hash::new();
            for (k, v) in obj {
                hash.insert(Yaml::String(k.clone()), json_to_yaml(v));
            }
            Yaml::Hash(hash)
        }
    }
}

/// The suffix mtrack stages a pending write under, alongside its destination.
///
/// Deliberately appended rather than substituted for the extension, so a staged
/// `01-main.yaml.mtrack-new` is not picked up by the directory scans that look
/// for `.yaml` and `.light` files.
const STAGED_SUFFIX: &str = ".mtrack-new";

/// The path a pending write to `path` is staged under.
pub fn staged_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(STAGED_SUFFIX);
    path.with_file_name(name)
}

/// Writes `contents` to `path`, preserving who owns the file and leaving a
/// recoverable copy if the write is interrupted.
///
/// The destination is rewritten **through its existing inode** rather than by
/// renaming a replacement over it. That is what preserves ownership: a rename
/// installs a new inode, which carries the ownership and mode of the file the
/// writing process created (itself, mode 0600) rather than the destination's.
/// Run mtrack once under `sudo` to debug something and a rename-based save
/// leaves every config file `root:root` and mode 0600, locking the normal user
/// out of their own configuration. Writing in place cannot do that, and it
/// keeps the file's ACLs and extended attributes into the bargain.
///
/// The cost is that an in-place rewrite is not atomic — it truncates and then
/// writes, so losing power midway leaves a short file. To bound that, the
/// complete new content is first staged in a sidecar (see [`staged_path`]) and
/// flushed to disk, and only removed once the destination is durable. A sidecar
/// left behind therefore means the last write to that file may not have
/// completed, and holds the content it was trying to write. Recovery is
/// currently manual: copy the sidecar over the destination.
///
/// The destination is locked for the duration, so two mtrack processes writing
/// the same file serialise rather than interleave. The lock is advisory and
/// only binds cooperating processes, which is all we need — the racing writer
/// this guards against is another mtrack.
pub fn write_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    // Read the ownership we are aiming for before creating anything, since
    // creating the destination would otherwise answer the question with the
    // freshly created file's own ownership.
    let intended = intended_ownership(path)?;

    // Stage the complete new content beside the destination and get it on disk,
    // so the rewrite below always has something to recover from.
    let staged = staged_path(path);
    {
        let mut file = std::fs::File::create(&staged)?;
        if let Some(ownership) = intended {
            ownership.apply(&file, &staged)?;
        }
        file.write_all(contents)?;
        file.sync_all()?;
    }
    // Flushing the file is not enough on its own: without syncing the directory
    // too, the sidecar's *name* may not survive a crash, and an unnamed file is
    // no use to recover from.
    if let Some(parent) = parent_of(path) {
        std::fs::File::open(parent)?.sync_all()?;
    }

    {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        // Lock before truncating, not as part of the open: opening with
        // truncation would discard a competing writer's content before we had
        // any claim to the file.
        file.lock()?;
        if let Some(ownership) = intended {
            ownership.apply(&file, path)?;
        }
        file.set_len(0)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }

    // The destination is durable, so the staged copy has served its purpose.
    // Leaving it behind would falsely advertise an interrupted write.
    std::fs::remove_file(&staged)
}

/// Explains a write failure whose cause is not the file's own permissions.
///
/// The generated systemd unit runs mtrack under `ProtectSystem=strict` as a
/// dedicated system user, and both of those produce failures that name the
/// symptom and not the cause. A path the operator owns and can write from a
/// shell reports a read-only filesystem, because it falls outside the unit's
/// `ReadWritePaths=`; a library they just created reports permission denied,
/// because the service user owns nothing it did not create. Neither points at
/// the unit, which is not where anyone looks.
///
/// `None` when the error is anything else, or when nothing suggests we are
/// running under systemd — outside a unit these two errors mean what they say.
///
/// `target` decides which directory the advice names, and callers must say
/// which they have: see [`WriteTarget`].
pub fn write_failure_hint(target: WriteTarget<'_>, error: &std::io::Error) -> Option<String> {
    let env = HintEnv {
        under_systemd: under_systemd(),
        service_user: service_user(),
        directory_exists: target.directory().is_dir(),
    };
    write_failure_hint_in(target, error, &env)
}

/// What a failed write was aiming at, which decides the directory the advice
/// names.
///
/// Getting this from the path alone is not possible: a directory that failed to
/// be *created* does not exist to be inspected, so `is_dir` says no and the
/// advice ends up one level too high — telling an operator to `chown -R` or
/// unseal `/var/lib` rather than their library.
#[derive(Debug, Clone, Copy)]
pub enum WriteTarget<'a> {
    /// A file. Advice names the directory holding it, since `ReadWritePaths=`
    /// is normally given one and chowning a lone file leaves its siblings
    /// unwritable.
    File(&'a Path),
    /// A directory, named as-is.
    Directory(&'a Path),
}

impl<'a> WriteTarget<'a> {
    /// The path itself, for reporting what failed.
    pub fn path(&self) -> &'a Path {
        match self {
            WriteTarget::File(path) | WriteTarget::Directory(path) => path,
        }
    }

    /// The directory the advice should name.
    fn directory(&self) -> &'a Path {
        match self {
            WriteTarget::File(path) => parent_of(path).unwrap_or(path),
            WriteTarget::Directory(path) => path,
        }
    }
}

/// What the advice needs to know about the machine it is running on, gathered
/// in one place so the wording can be tested without a real filesystem or
/// process-global environment variables.
struct HintEnv {
    under_systemd: bool,
    service_user: Option<String>,
    /// Whether the directory the advice will name is actually there. A
    /// directory that failed to be created is not, and most of the remediation
    /// changes when that is the case.
    directory_exists: bool,
}

/// Whether systemd started this process as a unit.
///
/// `INVOCATION_ID` is set by systemd for every service it starts, and is not
/// inherited by anything an operator runs from a shell on the same machine.
fn under_systemd() -> bool {
    std::env::var_os("INVOCATION_ID").is_some()
}

/// The account the service is running as, when it can be known.
///
/// systemd sets `USER` from the unit's `User=`. Naming a specific account
/// matters because `INVOCATION_ID` is set for *any* unit — a `systemd --user`
/// service, or a hand-written one with a different `User=` — and telling
/// somebody to `chown -R mtrack:mtrack` their own library is either an error
/// about an unknown user or a way to hand it to a system account.
fn service_user() -> Option<String> {
    std::env::var("USER").ok().filter(|user| !user.is_empty())
}

/// [`write_failure_hint`] with its environment supplied.
fn write_failure_hint_in(
    target: WriteTarget<'_>,
    error: &std::io::Error,
    env: &HintEnv,
) -> Option<String> {
    if !env.under_systemd {
        return None;
    }

    let dir = target.directory();
    // Commands are only offered for an absolute path. `mtrack systemd` refuses
    // a relative one, and a relative `chown` means something different in the
    // operator's shell than it did in the service's working directory.
    let actionable = dir.is_absolute();
    let user = env.service_user.as_deref();

    match error.kind() {
        std::io::ErrorKind::ReadOnlyFilesystem => {
            // Deliberately does not name `strict`: a unit generated without any
            // path gets `ProtectSystem=full`, and sending its operator to grep
            // for a directive their file does not contain wastes the one place
            // this message had their attention.
            let mut hint = format!(
                "The systemd sandbox is blocking this write: the unit's \
                 ProtectSystem= setting makes the filesystem read-only except \
                 for the paths listed in ReadWritePaths=, and {} is not one of \
                 them. The file's own permissions are not the cause — a \
                 read-only mount refuses the write before they are consulted.",
                dir.display(),
            );
            if actionable {
                // Order matters. systemd bind-mounts each ReadWritePaths= entry,
                // and cannot mount what is not there: an operator who pastes a
                // missing path in gets a unit that fails namespace setup and
                // never runs, which produces no diagnostic at all.
                if !env.directory_exists {
                    hint.push_str(&format!(
                        " Create {} first — systemd cannot bind-mount a path \
                         that does not exist, and an unprefixed ReadWritePaths= \
                         entry naming a missing one fails the unit at startup.",
                        dir.display(),
                    ));
                }
                hint.push_str(&format!(
                    " Add {} to ReadWritePaths= in the unit and run `systemctl \
                     daemon-reload`, or regenerate the unit naming {} alongside \
                     every path it already lists — `mtrack systemd` takes the \
                     full set each time and drops any path left out.",
                    dir.display(),
                    dir.display(),
                ));
            }
            Some(hint)
        }
        std::io::ErrorKind::PermissionDenied => {
            let mut hint = match user {
                Some(user) => format!(
                    "The service runs as {user}, which owns nothing it did not \
                     create, so this is likely ownership rather than a missing \
                     file."
                ),
                None => "The service runs as the account in the unit's User=, \
                         which owns nothing it did not create, so this is \
                         likely ownership rather than a missing file."
                    .to_string(),
            };
            if actionable {
                // A directory that could not be created cannot be chowned
                // either -- `chown` on a missing path just reports that it is
                // missing. What needs to be writable is the directory above it.
                let grant = if env.directory_exists {
                    dir
                } else {
                    hint.push_str(&format!(
                        " {} does not exist and could not be created, so the \
                         directory above it is the one that has to be writable.",
                        dir.display(),
                    ));
                    parent_of(dir).unwrap_or(dir)
                };
                match user {
                    Some(user) => hint.push_str(&format!(
                        " `chown -R {user}:{user} {}` grants it.",
                        grant.display()
                    )),
                    None => hint.push_str(&format!(
                        " Granting that account ownership of {} fixes it.",
                        grant.display()
                    )),
                }
            }
            Some(hint)
        }
        _ => None,
    }
}

/// [`write_file`] for callers on a Tokio runtime, which keeps the filesystem
/// work off the async worker.
pub async fn write_file_async(path: &Path, contents: Vec<u8>) -> std::io::Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || write_file(&path, &contents))
        .await
        .map_err(std::io::Error::other)?
}

/// The ownership a write to `path` should land with: that of the file already
/// there, or — when it is about to be created — that implied by its directory.
/// `None` when there is neither, leaving the process defaults to stand.
fn intended_ownership(path: &Path) -> std::io::Result<Option<Ownership>> {
    match Ownership::of(path) {
        Ok(existing) => Ok(Some(existing)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match parent_of(path) {
            Some(parent) => Ok(Some(Ownership::of(parent)?.for_file_in_dir())),
            None => Ok(None),
        },
        Err(e) => Err(e),
    }
}

/// Like [`std::fs::create_dir_all`], but every directory it creates adopts the
/// ownership and mode of the nearest ancestor that already existed, rather than
/// of the invoking user — for the same reason as [`write_file`]. A path that
/// already exists is left untouched.
pub fn create_dir_all(path: &Path) -> std::io::Result<()> {
    // Walk up to the nearest existing ancestor, remembering what we pass.
    let mut created = Vec::new();
    let mut cursor = path;
    let existing = loop {
        if cursor.try_exists()? {
            break Some(cursor);
        }
        created.push(cursor);
        match parent_of(cursor) {
            Some(parent) => cursor = parent,
            None => break None,
        }
    };

    std::fs::create_dir_all(path)?;

    let Some(existing) = existing else {
        return Ok(());
    };
    let ownership = Ownership::of(existing)?;

    // Outermost first, so a failure part way down still leaves the shallower
    // directories — the ones most likely to be shared — correctly owned.
    for dir in created.iter().rev() {
        // A directory opens read-only on Unix, which is enough for fchmod/fchown.
        ownership.apply(&std::fs::File::open(dir)?, dir)?;
    }
    Ok(())
}

/// Why [`create_dir_within`] declined to create a directory.
#[derive(Debug)]
pub enum CreateWithinError {
    /// The directory is outside the project directory, so nothing was created.
    Outside {
        /// The directory that was asked for.
        path: PathBuf,
        /// The project directory it had to be inside of.
        project: PathBuf,
    },
    /// Something is already there, and it is not a directory.
    NotADirectory {
        /// The path that is occupied.
        path: PathBuf,
        /// Whether the occupant is a symlink that leads nowhere useful. Worth
        /// distinguishing: a link to unmounted media is invisible to `exists`,
        /// and is the removable-media case this rule exists to protect.
        symlink: bool,
    },
    /// It was inside the project, and creating it failed anyway.
    Io(std::io::Error),
}

impl std::fmt::Display for CreateWithinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreateWithinError::Outside { path, project } => write!(
                f,
                "{} does not exist, and mtrack only creates directories inside \
                 its project directory ({}). Create it, or point the setting at \
                 a path inside the project.",
                path.display(),
                project.display(),
            ),
            CreateWithinError::NotADirectory {
                path,
                symlink: true,
            } => write!(
                f,
                "{} is a symlink that does not lead to a directory. Its target \
                 may be missing, or on media that is not mounted — mount it, \
                 repoint the link, or remove it.",
                path.display(),
            ),
            CreateWithinError::NotADirectory { path, .. } => write!(
                f,
                "{} is not a directory. Point the setting at a directory, or \
                 move what is there out of the way.",
                path.display(),
            ),
            CreateWithinError::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CreateWithinError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CreateWithinError::Io(error) => Some(error),
            CreateWithinError::Outside { .. } | CreateWithinError::NotADirectory { .. } => None,
        }
    }
}

/// Creates `dir` if it is missing, but only when it lies inside `project`.
///
/// mtrack creates directories on behalf of a config that names them, and a
/// config can name anywhere on the filesystem. Creating those wherever they
/// point turns a typo into an empty directory and a puzzling "no songs found",
/// and asks the service to write outside the very paths the generated systemd
/// unit's `ProtectSystem=strict` exists to fence off.
///
/// A directory that already exists is accepted wherever it is: the restriction
/// is on *creating* one, not on using a path the operator set up themselves.
///
/// Returns the directory that now exists, resolved. Callers must use it rather
/// than the path they passed in: creation follows the resolved path, and a
/// spelling like `songs/../real` is not equivalent to it — POSIX cannot resolve
/// that spelling unless `songs` exists, which is exactly the stray intermediate
/// this avoids making.
pub fn create_dir_within(dir: &Path, project: &Path) -> Result<PathBuf, CreateWithinError> {
    if dir.is_dir() {
        return Ok(dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()));
    }
    // `symlink_metadata` rather than `exists`, which follows the link and so
    // reports a dangling one as absent. Creation then fails deep inside
    // `create_dir_all` with `File exists` for a path the operator can see is
    // not there, saying nothing about the link.
    if let Ok(occupant) = std::fs::symlink_metadata(dir) {
        return Err(CreateWithinError::NotADirectory {
            path: dir.to_path_buf(),
            symlink: occupant.file_type().is_symlink(),
        });
    }

    let resolved = match resolve_as_far_as_possible(dir) {
        Resolved::Path(resolved) => resolved,
        // A link to nowhere anywhere along the path, not merely at its end.
        Resolved::Dangling(link) => {
            return Err(CreateWithinError::NotADirectory {
                path: link,
                symlink: true,
            })
        }
    };

    if !is_within(&resolved, project) {
        return Err(CreateWithinError::Outside {
            path: dir.to_path_buf(),
            project: project.to_path_buf(),
        });
    }

    // The resolved path, not the original. Creating the original walks the
    // literal spelling: it would materialize a stray intermediate for a benign
    // `songs/../real`, and it re-walks components that were checked as
    // something else — a window in which one could have become a symlink out of
    // the project.
    create_dir_all(&resolved).map_err(CreateWithinError::Io)?;
    Ok(resolved)
}

/// [`create_dir_within`] for callers on a Tokio runtime, which keeps the
/// filesystem work off the async worker.
pub async fn create_dir_within_async(
    dir: PathBuf,
    project: PathBuf,
) -> Result<PathBuf, CreateWithinError> {
    tokio::task::spawn_blocking(move || create_dir_within(&dir, &project))
        .await
        .map_err(|e| CreateWithinError::Io(std::io::Error::other(e)))?
}

/// The outcome of resolving a path as far as the filesystem allows.
#[derive(Debug)]
enum Resolved {
    /// Resolved to this absolute path.
    Path(PathBuf),
    /// A symlink along the way leads nowhere. Creating through it fails with
    /// `File exists` deep inside `create_dir_all` and says nothing about the
    /// link, for a path the operator can see is not there.
    Dangling(PathBuf),
}

/// `path` resolved as far as the filesystem allows.
///
/// Walks the components, canonicalizing whenever what has been built so far
/// exists. Resolving only the longest existing *prefix* is not enough: the
/// remainder gets pushed literally, so a `..` can walk back into existing,
/// symlinked territory — `project/missing/../link/songs` — and land outside
/// the project while still reading as inside it.
fn resolve_as_far_as_possible(path: &Path) -> Resolved {
    // A relative path is relative to the process's directory, and comparing one
    // against an absolute root answers "outside" for everything. `mtrack start
    // mtrack.yaml` produces exactly that: a project of "." and a songs path of
    // "songs", both of which mean the current directory.
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };

    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => {
                resolved.push(part);
                // Canonicalize the moment it exists, so a symlink is followed
                // here rather than compared as though it were a directory.
                match resolved.canonicalize() {
                    Ok(canonical) => resolved = canonical,
                    Err(_) => {
                        // Unresolvable but present means a link to nowhere.
                        // Checked at every component, not just the last: an
                        // unmounted drive is as likely to sit part way along
                        // the path as at the end of it.
                        if std::fs::symlink_metadata(&resolved)
                            .is_ok_and(|meta| meta.file_type().is_symlink())
                        {
                            return Resolved::Dangling(resolved);
                        }
                    }
                }
            }
            // Popping a canonical path is exact; popping one that still holds
            // an unresolved symlink is what the canonicalization above avoids.
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            std::path::Component::CurDir => {}
            other => resolved.push(other.as_os_str()),
        }
    }
    Resolved::Path(resolved)
}

/// Whether an already-resolved `path` lands inside `root`.
///
/// Takes the *resolved* path rather than resolving here, so the caller creates
/// exactly what was checked. Purely lexical comparison would not do:
/// `project/songs` is inside the project by spelling, and outside it if
/// `project/songs` is a symlink somewhere else.
fn is_within(path: &Path, root: &Path) -> bool {
    root.canonicalize().is_ok_and(|root| path.starts_with(root))
}

/// The project directory implied by a config file's path: the directory holding
/// it, and the only place mtrack creates directories on a config's behalf.
///
/// [`Path::parent`] answers `Some("")` rather than `None` for a bare filename,
/// so a fallback written against `None` never fires and leaves an empty path
/// behind — which canonicalizes to nothing and refuses every creation. That is
/// reachable: `mtrack start mtrack.yaml` passes exactly such a path.
pub fn project_dir_of(config_path: &Path) -> PathBuf {
    parent_of(config_path)
        .map(|parent| parent.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// [`create_dir_all`] for callers on a Tokio runtime, which keeps the
/// filesystem work off the async worker.
pub async fn create_dir_all_async(path: &Path) -> std::io::Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || create_dir_all(&path))
        .await
        .map_err(std::io::Error::other)?
}

/// The parent directory of `path`, treating the empty parent of a bare relative
/// name as absent rather than as the root.
fn parent_of(path: &Path) -> Option<&Path> {
    path.parent().filter(|p| !p.as_os_str().is_empty())
}

/// The ownership and mode of an existing file or directory. Changing ownership
/// is best effort — an unprivileged process cannot hand a file to another user,
/// and there is nothing useful to do about that beyond logging it — while the
/// mode is always applied.
#[derive(Clone, Copy)]
struct Ownership {
    #[cfg(unix)]
    uid: u32,
    #[cfg(unix)]
    gid: u32,
    #[cfg(unix)]
    mode: u32,
}

#[cfg(unix)]
impl Ownership {
    /// Reads the ownership of `path`, following symlinks: renaming over a
    /// symlink replaces the link, and the file it pointed at describes what the
    /// user actually wants far better than the link's own `lrwxrwxrwx`.
    fn of(path: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::MetadataExt;

        let meta = std::fs::metadata(path)?;
        Ok(Self {
            uid: meta.uid(),
            gid: meta.gid(),
            mode: meta.mode() & 0o7777,
        })
    }

    /// The ownership a file created inside this directory should have: the same
    /// owner, and the directory's mode without its execute bits — a 0755
    /// directory yields a 0644 file, a 0700 one yields 0600. That tracks the
    /// umask the directory itself was created under, without having to read the
    /// current umask, which cannot be queried without also setting it.
    fn for_file_in_dir(self) -> Self {
        Self {
            mode: self.mode & 0o666,
            ..self
        }
    }

    /// Applies this ownership and mode to an open file or directory. `path`
    /// names it for diagnostics only.
    fn apply(&self, file: &std::fs::File, path: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        // Skip the syscall in the overwhelmingly common case of writing a file
        // we already own — chown only matters when the ids actually differ.
        let current = file.metadata()?;
        if current.uid() != self.uid || current.gid() != self.gid {
            // Only a privileged process can give a file away, so failure here
            // is expected whenever a user edits a file owned by someone else,
            // and is not something they can act on. Log it and keep the write.
            if let Err(e) = std::os::unix::fs::fchown(file, Some(self.uid), Some(self.gid)) {
                tracing::warn!(
                    "Unable to set ownership of {} to {}:{} ({}). It will be owned by {}:{} instead.",
                    path.display(),
                    self.uid,
                    self.gid,
                    e,
                    current.uid(),
                    current.gid(),
                );
            }
        }

        // Mode last: chown clears the setuid and setgid bits on a file, so
        // setting the mode first would let the chown strip bits we just set.
        file.set_permissions(std::fs::Permissions::from_mode(self.mode))
    }
}

#[cfg(not(unix))]
impl Ownership {
    fn of(path: &Path) -> std::io::Result<Self> {
        // Confirm the path exists so callers can still distinguish replacing a
        // file from creating one, then carry no ownership to apply.
        std::fs::metadata(path)?;
        Ok(Self {})
    }

    fn for_file_in_dir(self) -> Self {
        self
    }

    fn apply(&self, _file: &std::fs::File, _path: &Path) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod ownership_test {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    fn set_mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn write_file_keeps_the_inode_of_the_destination() {
        // The whole point of writing in place: the file that ends up on disk is
        // the same file, so everything hanging off the inode survives.
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("config.yaml");
        std::fs::write(&dest, "old").unwrap();
        let before = std::fs::metadata(&dest).unwrap();

        write_file(&dest, b"new").unwrap();

        let after = std::fs::metadata(&dest).unwrap();
        assert_eq!(after.ino(), before.ino());
        assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "new");
    }

    #[test]
    fn write_file_keeps_the_mode_of_the_destination() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("config.yaml");
        std::fs::write(&dest, "old").unwrap();
        set_mode(&dest, 0o640);

        write_file(&dest, b"new").unwrap();

        assert_eq!(mode_of(&dest), 0o640);
    }

    #[test]
    fn write_file_derives_a_new_files_mode_from_its_directory() {
        for (dir_mode, want) in [(0o755, 0o644), (0o700, 0o600), (0o775, 0o664)] {
            let dir = tempfile::tempdir().unwrap();
            set_mode(dir.path(), dir_mode);
            let dest = dir.path().join("new.yaml");

            write_file(&dest, b"contents").unwrap();

            assert_eq!(
                mode_of(&dest),
                want,
                "directory mode {:o} should yield file mode {:o}",
                dir_mode,
                want
            );
            // Restore a writable mode so the temp dir can clean itself up.
            set_mode(dir.path(), 0o755);
        }
    }

    #[test]
    fn write_file_removes_the_staged_copy_once_the_write_lands() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("config.yaml");

        write_file(&dest, b"contents").unwrap();

        // A leftover sidecar means "the last write may not have completed", so
        // a successful write must not leave one lying about.
        assert!(!staged_path(&dest).exists());
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "contents");
    }

    #[test]
    fn staged_path_does_not_masquerade_as_a_config_file() {
        // Directory scans pick files by extension, so the sidecar must not keep
        // the one it was staged from.
        let staged = staged_path(Path::new("/cfg/profiles/01-main.yaml"));
        assert_eq!(staged.file_name().unwrap(), "01-main.yaml.mtrack-new");
        assert_ne!(staged.extension().unwrap(), "yaml");
        assert_ne!(
            staged_path(Path::new("/cfg/show.light"))
                .extension()
                .unwrap(),
            "light"
        );
    }

    #[test]
    fn create_dir_all_gives_new_directories_the_ancestors_mode() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("base");
        std::fs::create_dir(&base).unwrap();
        set_mode(&base, 0o750);

        create_dir_all(&base.join("a/b")).unwrap();

        assert_eq!(mode_of(&base.join("a")), 0o750);
        assert_eq!(mode_of(&base.join("a/b")), 0o750);
    }

    #[test]
    fn create_dir_all_leaves_existing_directories_alone() {
        let root = tempfile::tempdir().unwrap();
        let existing = root.path().join("existing");
        std::fs::create_dir(&existing).unwrap();
        set_mode(&existing, 0o700);

        create_dir_all(&existing).unwrap();

        assert_eq!(mode_of(&existing), 0o700);
    }

    #[test]
    fn write_file_leaves_an_existing_files_mode_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.yaml");
        std::fs::write(&path, "old").unwrap();
        set_mode(&path, 0o604);

        write_file(&path, b"new").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(mode_of(&path), 0o604);
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use crate::util::duration_minutes_seconds;

    #[test]
    fn test_duration_minutes_strings() {
        assert_eq!("0:00", duration_minutes_seconds(Duration::new(0, 0)));
        assert_eq!("0:05", duration_minutes_seconds(Duration::new(5, 0)));
        assert_eq!("0:55", duration_minutes_seconds(Duration::new(55, 0)));
        assert_eq!("1:00", duration_minutes_seconds(Duration::new(60, 0)));
        assert_eq!("2:05", duration_minutes_seconds(Duration::new(125, 0)));
        assert_eq!("60:06", duration_minutes_seconds(Duration::new(3606, 0)));
    }

    #[test]
    fn filename_display_normal() {
        use std::path::Path;
        assert_eq!(
            super::filename_display(Path::new("/home/user/song.wav")),
            "song.wav"
        );
    }

    #[test]
    fn filename_display_no_extension() {
        use std::path::Path;
        assert_eq!(
            super::filename_display(Path::new("/home/user/readme")),
            "readme"
        );
    }

    #[test]
    fn filename_display_just_filename() {
        use std::path::Path;
        assert_eq!(super::filename_display(Path::new("track.wav")), "track.wav");
    }

    #[test]
    fn filename_display_root_path() {
        use std::path::Path;
        // "/" has no file_name component
        assert_eq!(
            super::filename_display(Path::new("/")),
            "unreadable file name"
        );
    }

    #[test]
    fn filename_display_empty_path() {
        use std::path::Path;
        assert_eq!(
            super::filename_display(Path::new("")),
            "unreadable file name"
        );
    }

    #[test]
    fn kebab_case_spaces() {
        assert_eq!(super::to_kebab_case("Backing Track"), "backing-track");
    }

    #[test]
    fn kebab_case_underscores() {
        assert_eq!(super::to_kebab_case("backing_track"), "backing-track");
    }

    #[test]
    fn kebab_case_camel() {
        assert_eq!(super::to_kebab_case("backingTrack"), "backing-track");
    }

    #[test]
    fn kebab_case_pascal() {
        assert_eq!(super::to_kebab_case("BackingTrack"), "backing-track");
    }

    #[test]
    fn kebab_case_already_kebab() {
        assert_eq!(super::to_kebab_case("backing-track"), "backing-track");
    }

    #[test]
    fn kebab_case_mixed() {
        assert_eq!(
            super::to_kebab_case("My Cool_Song Name"),
            "my-cool-song-name"
        );
    }

    #[test]
    fn kebab_case_consecutive_separators() {
        assert_eq!(super::to_kebab_case("a  b__c--d"), "a-b-c-d");
    }

    #[test]
    fn kebab_case_all_caps() {
        assert_eq!(super::to_kebab_case("LOUD"), "loud");
    }

    #[test]
    fn kebab_case_acronym_then_word() {
        assert_eq!(super::to_kebab_case("XMLParser"), "xml-parser");
    }

    #[test]
    fn kebab_case_digit_before_upper() {
        assert_eq!(super::to_kebab_case("v2Track"), "v2-track");
        assert_eq!(super::to_kebab_case("Part2Guitars"), "part2-guitars");
    }

    #[test]
    fn kebab_case_non_ascii() {
        assert_eq!(super::to_kebab_case("café_backing"), "café-backing");
        assert_eq!(super::to_kebab_case("Élite Track"), "élite-track");
    }

    #[test]
    fn kebab_case_numbers_only() {
        assert_eq!(super::to_kebab_case("123"), "123");
    }

    #[test]
    fn kebab_case_trailing_leading_separators() {
        assert_eq!(super::to_kebab_case("_hello_"), "hello");
        assert_eq!(super::to_kebab_case("  spaced  "), "spaced");
    }
}

#[cfg(test)]
mod write_failure_hint_tests {
    use super::{write_failure_hint_in, HintEnv, WriteTarget};
    use std::io::{Error, ErrorKind};
    use std::path::Path;

    /// A machine where the named directory is there and the service is mtrack.
    fn env() -> HintEnv {
        HintEnv {
            under_systemd: true,
            service_user: Some("mtrack".to_string()),
            directory_exists: true,
        }
    }

    fn hint_for(target: WriteTarget<'_>, kind: ErrorKind, env: &HintEnv) -> Option<String> {
        write_failure_hint_in(target, &Error::from(kind), env)
    }

    fn hint(kind: ErrorKind) -> Option<String> {
        hint_for(
            WriteTarget::File(Path::new("/srv/songs/mtrack.yaml")),
            kind,
            &env(),
        )
    }

    /// The sandbox is the cause nobody looks for, so the advice has to name it
    /// and say what to do about it.
    #[test]
    fn a_read_only_filesystem_under_systemd_blames_the_sandbox() {
        let hint = hint(ErrorKind::ReadOnlyFilesystem).expect("a hint");
        assert!(hint.contains("ReadWritePaths"), "{hint}");
        // The directory, not the file: ReadWritePaths= is normally given one.
        assert!(hint.contains("/srv/songs"), "{hint}");
        assert!(!hint.contains("mtrack.yaml"), "{hint}");
        assert!(hint.contains("permissions are not the cause"), "{hint}");
    }

    /// A directory that failed to be created is the thing to name. Taking its
    /// parent instead told operators to `chown -R` or unseal `/var/lib`, which
    /// hands a system account far more than it needs and defeats the sandbox
    /// this advice exists to explain.
    #[test]
    fn a_directory_target_names_itself_not_its_parent() {
        for kind in [ErrorKind::ReadOnlyFilesystem, ErrorKind::PermissionDenied] {
            let hint = hint_for(
                WriteTarget::Directory(Path::new("/var/lib/mtrack-songs")),
                kind,
                &env(),
            )
            .expect("a hint");
            assert!(hint.contains("/var/lib/mtrack-songs"), "{kind:?}: {hint}");
            assert!(!hint.contains("/var/lib "), "{kind:?}: {hint}");
            assert!(!hint.contains("/var/lib`"), "{kind:?}: {hint}");
        }
    }

    /// systemd bind-mounts every ReadWritePaths= entry and cannot mount what is
    /// not there. An operator who pastes a missing path in gets a unit that
    /// fails namespace setup and never starts — no diagnostic at all, which is
    /// worse than the failure they began with.
    #[test]
    fn a_missing_directory_must_be_created_before_it_is_declared_writable() {
        let missing = HintEnv {
            directory_exists: false,
            ..env()
        };
        let hint = hint_for(
            WriteTarget::Directory(Path::new("/var/lib/outside-songs")),
            ErrorKind::ReadOnlyFilesystem,
            &missing,
        )
        .expect("a hint");
        assert!(
            hint.contains("Create /var/lib/outside-songs first"),
            "{hint}"
        );
        // And still says what to do after creating it.
        assert!(
            hint.contains("Add /var/lib/outside-songs to ReadWritePaths="),
            "{hint}"
        );
    }

    /// `chown` on a path that does not exist reports only that it does not
    /// exist. What has to be writable is the directory above it.
    #[test]
    fn a_missing_directory_is_granted_through_its_parent() {
        let missing = HintEnv {
            directory_exists: false,
            ..env()
        };
        let hint = hint_for(
            WriteTarget::Directory(Path::new("/var/lib/outside-songs")),
            ErrorKind::PermissionDenied,
            &missing,
        )
        .expect("a hint");
        assert!(
            hint.contains("does not exist and could not be created"),
            "{hint}"
        );
        assert!(hint.contains("chown -R mtrack:mtrack /var/lib"), "{hint}");
        // Not the missing directory itself, which chown would simply reject.
        assert!(
            !hint.contains("chown -R mtrack:mtrack /var/lib/outside-songs"),
            "{hint}"
        );
    }

    /// `mtrack systemd` takes the complete set of paths and drops any left out,
    /// so advice to re-run it has to say so — following it verbatim otherwise
    /// trades this block for a new one on the next start.
    #[test]
    fn regenerating_is_described_as_taking_every_path() {
        let hint = hint(ErrorKind::ReadOnlyFilesystem).expect("a hint");
        assert!(hint.contains("alongside"), "{hint}");
        assert!(hint.contains("drops any path left out"), "{hint}");
    }

    /// The unit is only `ProtectSystem=strict` when it was generated with a
    /// path; naming the value sends everyone else grepping for a line their
    /// file does not have.
    #[test]
    fn the_sandbox_advice_does_not_assert_a_particular_protect_system() {
        let hint = hint(ErrorKind::ReadOnlyFilesystem).expect("a hint");
        assert!(hint.contains("ProtectSystem="), "{hint}");
        assert!(!hint.contains("ProtectSystem=strict"), "{hint}");
        assert!(!hint.contains("ProtectSystem=full"), "{hint}");
    }

    /// A denied write is an ownership problem, and pointing it at the sandbox
    /// would send the operator to edit a unit that is already correct.
    #[test]
    fn permission_denied_under_systemd_blames_ownership_not_the_sandbox() {
        let hint = hint(ErrorKind::PermissionDenied).expect("a hint");
        assert!(hint.contains("chown -R mtrack:mtrack /srv/songs"), "{hint}");
        assert!(!hint.contains("ReadWritePaths"), "{hint}");
        assert!(!hint.contains("ProtectSystem"), "{hint}");
    }

    /// `INVOCATION_ID` is set for any unit, including a `systemd --user` one
    /// running as somebody's own account. Prescribing `mtrack:mtrack` there is
    /// either an error about an unknown user or a way to hand a personal
    /// library to a system account.
    #[test]
    fn ownership_advice_names_the_account_the_service_runs_as() {
        let alice = HintEnv {
            service_user: Some("alice".to_string()),
            ..env()
        };
        let hint = hint_for(
            WriteTarget::Directory(Path::new("/home/alice/music")),
            ErrorKind::PermissionDenied,
            &alice,
        )
        .expect("a hint");
        assert!(
            hint.contains("chown -R alice:alice /home/alice/music"),
            "{hint}"
        );
        assert!(!hint.contains("mtrack:mtrack"), "{hint}");
    }

    /// With no account to name, the advice still has to be true.
    #[test]
    fn ownership_advice_without_a_known_account_prescribes_no_command() {
        let anonymous = HintEnv {
            service_user: None,
            ..env()
        };
        let hint = hint_for(
            WriteTarget::Directory(Path::new("/srv/songs")),
            ErrorKind::PermissionDenied,
            &anonymous,
        )
        .expect("a hint");
        assert!(hint.contains("User="), "{hint}");
        assert!(!hint.contains("chown"), "{hint}");
        assert!(hint.contains("/srv/songs"), "{hint}");
    }

    /// A relative path gets the explanation but no command: `mtrack systemd`
    /// refuses one, and a relative `chown` means something different in the
    /// operator's shell than it did in the service's working directory.
    #[test]
    fn a_relative_path_is_explained_but_gets_no_command_to_paste() {
        for (kind, expected) in [
            (ErrorKind::ReadOnlyFilesystem, "ReadWritePaths"),
            (
                ErrorKind::PermissionDenied,
                "owns nothing it did not create",
            ),
        ] {
            let hint = hint_for(WriteTarget::File(Path::new("mtrack.yaml")), kind, &env())
                .expect("a hint");
            // Still says what went wrong.
            assert!(hint.contains(expected), "{kind:?}: {hint}");
            // But offers nothing that would be rejected or misread on arrival.
            assert!(!hint.contains("mtrack systemd"), "{kind:?}: {hint}");
            assert!(!hint.contains("daemon-reload"), "{kind:?}: {hint}");
            assert!(!hint.contains("chown"), "{kind:?}: {hint}");
        }
    }

    /// Outside a unit these errors mean exactly what they say, and advice about
    /// a systemd sandbox that is not running would be a false lead.
    #[test]
    fn nothing_is_offered_outside_systemd() {
        let shell = HintEnv {
            under_systemd: false,
            ..env()
        };
        for kind in [ErrorKind::ReadOnlyFilesystem, ErrorKind::PermissionDenied] {
            assert_eq!(
                hint_for(
                    WriteTarget::File(Path::new("/srv/songs/x.yaml")),
                    kind,
                    &shell
                ),
                None
            );
        }
    }

    /// Only the two failures the unit actually explains get an explanation.
    #[test]
    fn unrelated_failures_are_left_alone() {
        for kind in [
            ErrorKind::NotFound,
            ErrorKind::AlreadyExists,
            ErrorKind::StorageFull,
            ErrorKind::InvalidInput,
        ] {
            assert_eq!(hint(kind), None, "{kind:?}");
        }
    }
}

#[cfg(test)]
mod project_dir_of_tests {
    use super::project_dir_of;
    use std::path::{Path, PathBuf};

    /// The case that broke `mtrack start mtrack.yaml`.
    ///
    /// `Path::parent` answers `Some("")` for a bare filename, so a fallback
    /// written against `None` never fired and left an empty path. That
    /// canonicalizes to nothing, which made every directory look outside the
    /// project and refused creation with an error naming no project at all.
    #[test]
    fn a_bare_config_filename_means_the_current_directory() {
        assert_eq!(project_dir_of(Path::new("mtrack.yaml")), PathBuf::from("."));
    }

    #[test]
    fn a_config_in_a_directory_means_that_directory() {
        assert_eq!(
            project_dir_of(Path::new("/srv/mtrack/mtrack.yaml")),
            PathBuf::from("/srv/mtrack")
        );
    }

    #[test]
    fn a_relative_config_keeps_its_directory() {
        assert_eq!(
            project_dir_of(Path::new("gig/mtrack.yaml")),
            PathBuf::from("gig")
        );
    }

    /// The project directory it yields has to be one `create_dir_within`
    /// accepts, or the fix moves the failure rather than removing it.
    #[test]
    fn the_directory_it_yields_accepts_a_child() {
        let project = tempfile::tempdir().expect("tempdir");
        let config = project.path().join("mtrack.yaml");

        let resolved = project_dir_of(&config);
        super::create_dir_within(&resolved.join("songs"), &resolved).expect("created");
        assert!(project.path().join("songs").is_dir());
    }
}

#[cfg(test)]
mod create_dir_within_tests {
    use super::{create_dir_within, CreateWithinError};
    use std::path::{Path, PathBuf};

    fn is_outside(result: Result<PathBuf, CreateWithinError>) -> bool {
        matches!(result, Err(CreateWithinError::Outside { .. }))
    }

    /// The first-run case this keeps: a relative `songs:` resolves inside the
    /// project, and mtrack still sets it up without being asked.
    #[test]
    fn a_directory_inside_the_project_is_created() {
        let project = tempfile::tempdir().expect("tempdir");
        let songs = project.path().join("songs");

        create_dir_within(&songs, project.path()).expect("created");
        assert!(songs.is_dir());
    }

    #[test]
    fn nested_directories_inside_the_project_are_created() {
        let project = tempfile::tempdir().expect("tempdir");
        let nested = project.path().join("media/audio/songs");

        create_dir_within(&nested, project.path()).expect("created");
        assert!(nested.is_dir());
    }

    /// The case that motivated this: a typo in an absolute path used to become
    /// an empty directory and a puzzling "no songs found".
    #[test]
    fn a_directory_outside_the_project_is_refused_not_created() {
        let project = tempfile::tempdir().expect("tempdir");
        let elsewhere = tempfile::tempdir().expect("tempdir");
        let outside = elsewhere.path().join("sonsg");

        assert!(is_outside(create_dir_within(&outside, project.path())));
        assert!(!outside.exists(), "it must not have been created anyway");
    }

    /// Naming the parent is what makes the message actionable.
    #[test]
    fn the_refusal_names_both_the_path_and_the_project() {
        let project = tempfile::tempdir().expect("tempdir");
        let elsewhere = tempfile::tempdir().expect("tempdir");
        let outside = elsewhere.path().join("songs");

        let error = create_dir_within(&outside, project.path()).expect_err("refused");
        let message = error.to_string();
        assert!(
            message.contains(&outside.display().to_string()),
            "{message}"
        );
        assert!(
            message.contains(&project.path().display().to_string()),
            "{message}"
        );
    }

    /// A path that merely spells its way back out is still out.
    ///
    /// The project is nested inside an outer temp directory so the escape has
    /// somewhere contained to land: a test that escapes into /tmp litters a
    /// shared path, and the leftover then fails the next run for the wrong
    /// reason.
    #[test]
    fn a_traversal_out_of_the_project_is_refused() {
        let outer = tempfile::tempdir().expect("tempdir");
        let project = outer.path().join("project");
        std::fs::create_dir(&project).expect("project dir");
        let escaped = project.join("../escaped-songs");

        assert!(is_outside(create_dir_within(&escaped, &project)));
        assert!(!outer.path().join("escaped-songs").exists());
    }

    /// "Does not exist" sent the operator looking for a path sitting right
    /// there — `songs: /mnt/nas/songs.tar` names a real file, just not a
    /// directory.
    #[test]
    fn an_occupied_path_says_so_rather_than_claiming_it_is_missing() {
        let project = tempfile::tempdir().expect("tempdir");
        let occupied = project.path().join("songs.tar");
        std::fs::write(&occupied, b"not a directory").expect("write");

        let error = create_dir_within(&occupied, project.path()).expect_err("refused");
        let message = error.to_string();
        assert!(message.contains("is not a directory"), "{message}");
        assert!(!message.contains("does not exist"), "{message}");
        // The file it named is left alone.
        assert!(occupied.is_file());
    }

    /// The removable-media case this rule exists to protect, and the one that
    /// was reported worst: `songs -> /media/usb/songs` with the stick
    /// unmounted. `exists()` follows the link and says nothing is there, so
    /// creation used to run on and fail inside `create_dir_all` with `File
    /// exists` — for a path the operator can see is absent, with no mention of
    /// the link.
    #[cfg(unix)]
    #[test]
    fn a_symlink_leading_nowhere_says_so_rather_than_file_exists() {
        let project = tempfile::tempdir().expect("tempdir");
        let dangling = project.path().join("songs");
        std::os::unix::fs::symlink("/media/usb/songs-that-are-not-mounted", &dangling)
            .expect("symlink");

        let message = create_dir_within(&dangling, project.path())
            .expect_err("refused")
            .to_string();
        assert!(message.contains("is a symlink"), "{message}");
        assert!(message.contains("not mounted"), "{message}");
        assert!(!message.contains("File exists"), "{message}");
        assert!(!message.contains("does not exist"), "{message}");
    }

    /// A link to nowhere part way along the path is the same failure as one at
    /// the end, and just as likely: an unmounted drive sits wherever the
    /// operator mounted it, not necessarily at the leaf.
    #[cfg(unix)]
    #[test]
    fn a_symlink_leading_nowhere_mid_path_says_so_too() {
        let project = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink(
            "/media/usb/lib-that-is-not-mounted",
            project.path().join("lib"),
        )
        .expect("symlink");

        // The dangling link is `lib`, not the leaf being asked for.
        let through = project.path().join("lib/songs");
        let message = create_dir_within(&through, project.path())
            .expect_err("refused")
            .to_string();
        assert!(message.contains("is a symlink"), "{message}");
        assert!(!message.contains("File exists"), "{message}");
    }

    /// Creation uses the path that was checked, not the spelling it came in as
    /// — which would walk components already resolved to something else and
    /// leave stray intermediates behind.
    ///
    /// The returned path is the point: POSIX cannot resolve `songs/../real`
    /// unless `songs` exists, so a caller that kept its own spelling would be
    /// told the directory was created and then fail to read it.
    #[test]
    fn a_benign_traversal_returns_a_path_the_caller_can_use() {
        let project = tempfile::tempdir().expect("tempdir");
        let configured = project.path().join("songs/../real");

        let created = create_dir_within(&configured, project.path()).expect("created");

        assert!(project.path().join("real").is_dir(), "target not created");
        assert!(
            !project.path().join("songs").exists(),
            "left a stray intermediate behind"
        );
        // The returned path resolves; the configured spelling does not.
        assert!(
            created.is_dir(),
            "returned {} is unusable",
            created.display()
        );
        assert!(
            !configured.is_dir(),
            "the spelling resolves after all — this test proves nothing"
        );
        assert!(
            std::fs::read_dir(&created).is_ok(),
            "cannot read what was made"
        );
    }

    /// An existing directory also comes back resolved, so callers get the same
    /// kind of path either way rather than one that depends on whether they
    /// happened to be first.
    #[test]
    fn an_existing_directory_is_returned_resolved() {
        let project = tempfile::tempdir().expect("tempdir");
        let songs = project.path().join("songs");
        std::fs::create_dir(&songs).expect("songs");

        let returned = create_dir_within(&songs, project.path()).expect("accepted");
        assert_eq!(returned, songs.canonicalize().expect("canonical"));
    }

    /// A symlink that does lead to a directory is simply used, wherever it
    /// points — the same as any other existing directory.
    #[cfg(unix)]
    #[test]
    fn a_symlink_leading_to_a_real_directory_is_accepted() {
        let project = tempfile::tempdir().expect("tempdir");
        let elsewhere = tempfile::tempdir().expect("tempdir");
        let link = project.path().join("songs");
        std::os::unix::fs::symlink(elsewhere.path(), &link).expect("symlink");

        create_dir_within(&link, project.path()).expect("accepted");
    }

    /// A file in the way outside the project is still reported as the file it
    /// is: the operator has to move it either way, and "outside the project"
    /// would send them to fix the wrong thing.
    #[test]
    fn an_occupied_path_outside_the_project_reports_the_occupant() {
        let project = tempfile::tempdir().expect("tempdir");
        let elsewhere = tempfile::tempdir().expect("tempdir");
        let occupied = elsewhere.path().join("songs.tar");
        std::fs::write(&occupied, b"not a directory").expect("write");

        let message = create_dir_within(&occupied, project.path())
            .expect_err("refused")
            .to_string();
        assert!(message.contains("is not a directory"), "{message}");
    }

    /// The restriction is on *creating*, not on using. An operator who set up
    /// /mnt/nas/songs themselves must keep working.
    #[test]
    fn an_existing_directory_outside_the_project_is_accepted() {
        let project = tempfile::tempdir().expect("tempdir");
        let elsewhere = tempfile::tempdir().expect("tempdir");

        create_dir_within(elsewhere.path(), project.path()).expect("accepted");
    }

    /// Spelling is not containment. `project/songs` looks inside the project
    /// whatever it points at, so a lexical check would create the target of a
    /// symlink that leaves it.
    #[cfg(unix)]
    #[test]
    fn a_symlink_leaving_the_project_is_refused() {
        let project = tempfile::tempdir().expect("tempdir");
        let elsewhere = tempfile::tempdir().expect("tempdir");

        let link = project.path().join("songs");
        std::os::unix::fs::symlink(elsewhere.path(), &link).expect("symlink");

        // Spelled inside the project, actually somewhere else entirely.
        let through_link = link.join("inner");
        assert!(is_outside(create_dir_within(&through_link, project.path())));
        assert!(!elsewhere.path().join("inner").exists());
    }

    /// A `..` can walk back into existing, symlinked territory.
    ///
    /// Canonicalizing only the longest *existing* ancestor is not enough: the
    /// remainder is pushed literally, so `project/missing/../link/songs` hops
    /// over `missing`, lands on a symlink that leaves the project, and reads as
    /// inside it. The previous fix closed the lexical spelling and left this.
    #[cfg(unix)]
    #[test]
    fn a_symlink_reached_after_a_missing_component_is_refused() {
        let outer = tempfile::tempdir().expect("tempdir");
        let project = outer.path().join("project");
        std::fs::create_dir(&project).expect("project dir");
        let outside = outer.path().join("outside");
        std::fs::create_dir(&outside).expect("outside dir");

        std::os::unix::fs::symlink(&outside, project.join("link")).expect("symlink");

        // `missing` does not exist, so resolution stops before `link`.
        let escaped = project.join("missing/../link/songs");
        assert!(
            is_outside(create_dir_within(&escaped, &project)),
            "escaped through a symlink after a missing component"
        );
        assert!(
            !outside.join("songs").exists(),
            "created {} outside",
            outside.join("songs").display()
        );
    }

    /// A symlink that stays inside the project is not the problem.
    #[cfg(unix)]
    #[test]
    fn a_symlink_staying_inside_the_project_is_allowed() {
        let project = tempfile::tempdir().expect("tempdir");
        let real = project.path().join("real");
        std::fs::create_dir(&real).expect("real dir");

        let link = project.path().join("link");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");

        create_dir_within(&link.join("songs"), project.path()).expect("created");
        assert!(real.join("songs").is_dir());
    }

    /// `..` after a component that does not exist used to escape entirely.
    ///
    /// Resolution gave up at a path ending in `..` (whose `file_name()` is
    /// `None`), returned the path unresolved, and `starts_with` then compared
    /// it lexically -- where `/proj/new/../../escaped` is "inside" `/proj`.
    /// The directory really was created out there.
    #[test]
    fn a_traversal_through_a_missing_component_is_refused() {
        let outer = tempfile::tempdir().expect("tempdir");
        let project = outer.path().join("project");
        std::fs::create_dir(&project).expect("project dir");
        let outside = project.join("missing/../../escaped-songs");

        assert!(
            is_outside(create_dir_within(&outside, &project)),
            "escaped the project"
        );
        // Where the old code actually made it: the project's sibling.
        let escaped = outer.path().join("escaped-songs");
        assert!(!escaped.exists(), "created {} outside", escaped.display());
    }

    /// A relative path means "relative to the process's directory", and
    /// comparing one against an absolute root answers "outside" for everything.
    ///
    /// `mtrack start mtrack.yaml` produces exactly that pair — a project of "."
    /// and a songs path of "songs" — and refused to create it, which unit tests
    /// of the pieces missed because only running the binary puts the two
    /// together. Reads the current directory rather than changing it, which
    /// would race the rest of the suite.
    #[test]
    fn a_relative_path_is_measured_against_the_current_directory() {
        let super::Resolved::Path(resolved) =
            super::resolve_as_far_as_possible(Path::new("a-relative-name"))
        else {
            panic!("expected a resolved path");
        };

        let cwd = std::env::current_dir().expect("cwd");
        assert!(
            super::is_within(&resolved, &cwd),
            "a relative name must be inside the current directory: {}",
            resolved.display()
        );

        let elsewhere = tempfile::tempdir().expect("tempdir");
        assert!(
            !super::is_within(&resolved, elsewhere.path()),
            "and outside an unrelated project"
        );
    }

    /// Without a project directory there is nothing to be inside of, and
    /// creating anywhere would be the old behaviour by another name.
    #[test]
    fn a_project_directory_that_does_not_exist_refuses_everything() {
        let project = Path::new("/nonexistent-project-dir-for-tests");
        let elsewhere = tempfile::tempdir().expect("tempdir");

        assert!(is_outside(create_dir_within(
            &elsewhere.path().join("songs"),
            project
        )));
    }
}
