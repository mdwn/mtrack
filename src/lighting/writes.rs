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

//! What a writer of one venue or fixture type may do to its file, by the
//! one rule in [`super::files`]: a new name is claimed at its canonical
//! path; an existing one is written only where its owner's file is that
//! name's alone; anything else is refused with the file and why. The web
//! UI's saves, the MVR import and the fixture settings all plan here, so a
//! venue or type is written alike whoever writes it.

use std::path::{Path, PathBuf};

use super::files::{self, Editable, Kind, Owner, Refusal};
use super::library::LibraryType;

/// Why a save is refused, before anything is written.
#[derive(Debug)]
pub enum Refused {
    /// A rename of a name no file declares.
    NotFound { kind: Kind, name: String },
    /// The name's file is not its alone.
    ReadOnly {
        kind: Kind,
        dir: PathBuf,
        owner: Owner,
    },
    /// A new name, or a rename's, that [`files::claim_new`] or
    /// [`files::name_free`] refused.
    Claim(Refusal),
    /// A `.light` whose text needs the newer format, where the `.venue` or
    /// `.fixture` it moves to (`file`, shown relative to the directory) is
    /// already there.
    UpgradeTaken {
        kind: Kind,
        name: String,
        file: String,
    },
    /// The owner's file could not be read.
    Unreadable {
        path: PathBuf,
        error: std::io::Error,
    },
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::NotFound { kind, name } => write!(f, "{} \"{name}\" not found", kind.noun()),
            Refused::ReadOnly { kind, dir, owner } => {
                f.write_str(&owner.reason(*kind, dir).unwrap_or_default())
            }
            Refused::Claim(refusal) => write!(f, "{refusal}"),
            Refused::UpgradeTaken { kind, name, file } => write!(
                f,
                "{} \"{name}\" now needs a .{} file, and {file} is there; move or remove it by \
                 hand, then save again",
                kind.noun(),
                kind.extension()
            ),
            Refused::Unreadable { path, error } => {
                write!(f, "cannot read {}: {error}", path.display())
            }
        }
    }
}

impl std::error::Error for Refused {}

/// How a save is written.
#[derive(Debug, PartialEq)]
pub enum Write {
    /// A file made with `create_new`: a new name's claimed canonical file,
    /// a renamed canonical file's new one, or a `.light` upgraded.
    Create(PathBuf),
    /// The owner's file, rewritten in place.
    Rewrite(PathBuf),
}

impl Write {
    /// The file written.
    pub fn path(&self) -> &Path {
        match self {
            Write::Create(path) | Write::Rewrite(path) => path,
        }
    }
}

/// What a save of one venue or fixture type writes, decided before
/// anything is.
#[derive(Debug)]
pub struct Plan {
    /// The directory the save is in, which its files are shown relative to.
    pub dir: PathBuf,
    pub write: Write,
    /// The owner's file, removed once the new one is written.
    pub retire: Option<PathBuf>,
    /// The owner's file as it is now; `None` for a new name.
    pub current: Option<Vec<u8>>,
}

impl Plan {
    /// Whether the save creates a name: new, or renamed.
    pub fn names_anew(&self, renaming: bool) -> bool {
        self.current.is_none() || renaming
    }
}

/// What is being saved: `name`, renamed from `from` when that is another
/// name, in `dir`, whose files were read into `record`.
pub struct Save<'a> {
    pub kind: Kind,
    pub dir: &'a Path,
    pub record: &'a files::Record,
    /// The library's unrecorded types (fixture types), whose names a new
    /// name may not take; none for venues.
    pub library: &'a [LibraryType],
    pub name: &'a str,
    pub from: Option<&'a str>,
    /// Whether the new text needs the newer format (`.venue` / `.fixture`).
    pub needs_new_format: bool,
}

impl Save<'_> {
    /// The name whose file is written: the old one of a rename.
    pub fn subject(&self) -> &str {
        self.from.unwrap_or(self.name)
    }

    /// Whether this save gives a name a new name.
    pub fn renaming(&self) -> bool {
        self.from
            .is_some_and(|from| !files::same_name(from, self.name))
    }

    /// The owner of the name whose file is written, if any.
    pub fn owner(&self) -> Option<Owner> {
        self.record.owner(self.subject())
    }

    /// Where the save goes, or the refusal:
    /// - a new name: [`files::claim_new`] at its canonical path;
    /// - a name whose file is not its alone: [`Refused::ReadOnly`];
    /// - a rename of a file at the old name's canonical place: a new file
    ///   at the new name's, claimed, and the old one removed;
    /// - otherwise the owner's file in place, unless the text needs the
    ///   newer format and the file is a `.light`: then the same stem as a
    ///   `.venue` / `.fixture` beside it ([`files::upgrade_path`]), made
    ///   new, and the `.light` removed.
    pub fn plan(&self) -> Result<Plan, Refused> {
        let Some(owner) = self.owner() else {
            if self.renaming() {
                return Err(Refused::NotFound {
                    kind: self.kind,
                    name: self.subject().to_string(),
                });
            }
            let claim = files::claim_new(self.kind, self.dir, self.name, self.library)
                .map_err(Refused::Claim)?;
            return Ok(Plan {
                dir: self.dir.to_path_buf(),
                write: Write::Create(claim.path().to_path_buf()),
                retire: None,
                current: None,
            });
        };
        if owner.editable != Editable::Editable {
            return Err(Refused::ReadOnly {
                kind: self.kind,
                dir: self.dir.to_path_buf(),
                owner,
            });
        }
        let current = crate::util::read_file(&owner.path).map_err(|error| Refused::Unreadable {
            path: owner.path.clone(),
            error,
        })?;
        if self.renaming() {
            files::name_free(self.kind, self.dir, self.name, self.library)
                .map_err(Refused::Claim)?;
            let moves = owner.at_canonical_place(self.kind, self.dir, self.subject())
                && files::stem(self.name) != files::stem(self.subject());
            if moves {
                let claim = files::claim_new(self.kind, self.dir, self.name, self.library)
                    .map_err(Refused::Claim)?;
                return Ok(Plan {
                    dir: self.dir.to_path_buf(),
                    write: Write::Create(claim.path().to_path_buf()),
                    retire: Some(owner.path),
                    current: Some(current),
                });
            }
        }
        let is_light = owner
            .path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("light"));
        if self.needs_new_format && is_light {
            let upgraded =
                files::upgrade_path(self.kind, self.dir, &owner.path).map_err(|refusal| {
                    match refusal {
                        Refusal::Taken { file, .. } => Refused::UpgradeTaken {
                            kind: self.kind,
                            name: self.subject().to_string(),
                            file,
                        },
                        other => Refused::Claim(other),
                    }
                })?;
            return Ok(Plan {
                dir: self.dir.to_path_buf(),
                write: Write::Create(upgraded),
                retire: Some(owner.path),
                current: Some(current),
            });
        }
        Ok(Plan {
            dir: self.dir.to_path_buf(),
            write: Write::Rewrite(owner.path),
            retire: None,
            current: Some(current),
        })
    }
}

/// Why a write failed.
#[derive(Debug)]
pub enum WriteError {
    /// The file to be made appeared after it was claimed; it was not
    /// written over. `shown` is the file as its writer's user knows it.
    Exists { path: PathBuf, shown: String },
    /// The file could not be written.
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    /// The old file of a move (`shown`) could not be removed.
    NotRetired {
        shown: String,
        error: std::io::Error,
    },
    /// `cause` failed, and everything written before it (and what it
    /// changed itself) was put back, but for `left`: each file that could
    /// not be, and why.
    RolledBack {
        cause: Box<WriteError>,
        left: Vec<String>,
    },
}

impl WriteError {
    /// The failure itself, under any putting back.
    pub fn cause(&self) -> &WriteError {
        match self {
            WriteError::RolledBack { cause, .. } => cause.cause(),
            other => other,
        }
    }
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Exists { shown, .. } => write!(
                f,
                "{shown} appeared after it was claimed and was not written over; move or remove \
                 it by hand, then try again"
            ),
            WriteError::Io { path, error } => f.write_str(&crate::util::write_failure_message(
                crate::util::WriteTarget::File(path),
                error,
            )),
            WriteError::NotRetired { shown, error } => {
                write!(f, "cannot remove {shown}: {error}")
            }
            WriteError::RolledBack { cause, left } if left.is_empty() => {
                write!(f, "{cause}; nothing was saved")
            }
            WriteError::RolledBack { cause, left } => write!(
                f,
                "{cause}; nothing more was saved, but {} could not be put back",
                left.join(", ")
            ),
        }
    }
}

impl std::error::Error for WriteError {}

/// How to put back one change.
#[derive(Debug)]
enum Undo {
    /// A file this writer made.
    Remove { path: PathBuf, shown: String },
    /// A file this writer rewrote, and what it held.
    Restore {
        path: PathBuf,
        shown: String,
        was: Vec<u8>,
    },
}

/// One writer's files, written one after another, each change recorded as
/// it is made, so that a failure anywhere puts back everything this writer
/// changed (the failing file's own change included) and nothing it did not:
/// a file someone else made in the meantime is never touched. The one rule
/// for every write of venue and fixture type files, one alone or with what
/// goes with it (a rename's venue files, a settings save's pins).
#[derive(Debug, Default)]
pub struct Changes {
    done: Vec<Undo>,
}

impl Changes {
    fn write(
        &mut self,
        path: &Path,
        contents: &[u8],
        shown: String,
        was: Option<&[u8]>,
    ) -> Result<(), WriteError> {
        let new = was.is_none();
        let result = crate::util::write_file_tracked(path, contents, new);
        let touched = match &result {
            Ok(()) => true,
            Err(failure) => failure.touched,
        };
        if touched {
            self.done.push(match was {
                None => Undo::Remove {
                    path: path.to_path_buf(),
                    shown: shown.clone(),
                },
                Some(was) => Undo::Restore {
                    path: path.to_path_buf(),
                    shown: shown.clone(),
                    was: was.to_vec(),
                },
            });
        }
        result.map_err(|failure| {
            if new && failure.error.kind() == std::io::ErrorKind::AlreadyExists {
                WriteError::Exists {
                    path: path.to_path_buf(),
                    shown,
                }
            } else {
                WriteError::Io {
                    path: path.to_path_buf(),
                    error: failure.error,
                }
            }
        })
    }

    /// Makes the new file `path` with `create_new`, as a claim is written:
    /// one that appeared since it was claimed is [`WriteError::Exists`],
    /// never written over, and never removed by [`Self::roll_back`].
    /// `shown` is the file as the user knows it.
    pub fn create(
        &mut self,
        path: &Path,
        contents: &[u8],
        shown: String,
    ) -> Result<(), WriteError> {
        self.write(path, contents, shown, None)
    }

    /// Rewrites `path`, which held `was`, in place.
    pub fn rewrite(
        &mut self,
        path: &Path,
        contents: &[u8],
        was: &[u8],
        shown: String,
    ) -> Result<(), WriteError> {
        self.write(path, contents, shown, Some(was))
    }

    /// Writes `content` as `plan` says, leaving the file it moves from (if
    /// any) for [`Self::retire`], which comes once everything that goes
    /// with it is written.
    pub fn plan(&mut self, plan: &Plan, content: &str) -> Result<(), WriteError> {
        let shown = super::project_files::display_in(&plan.dir, plan.write.path());
        match &plan.write {
            Write::Rewrite(path) => {
                let was = plan.current.as_deref().unwrap_or_default();
                self.rewrite(path, content.as_bytes(), was, shown)
            }
            Write::Create(path) => self.create(path, content.as_bytes(), shown),
        }
    }

    /// Removes the old file `plan` moves from, if any: the last step.
    pub fn retire(&mut self, plan: &Plan) -> Result<(), WriteError> {
        match &plan.retire {
            Some(old) => remove_file(old).map_err(|error| WriteError::NotRetired {
                shown: super::project_files::display_in(&plan.dir, old),
                error,
            }),
            None => Ok(()),
        }
    }

    /// Puts back every change, newest first, after `cause` failed: the
    /// error to answer with, naming each file that could not be put back.
    pub fn roll_back(self, cause: WriteError) -> WriteError {
        let mut left = Vec::new();
        for undo in self.done.into_iter().rev() {
            let (shown, result) = match undo {
                Undo::Remove { path, shown } => (shown, remove_file(&path)),
                Undo::Restore { path, shown, was } => (shown, crate::util::write_file(&path, &was)),
            };
            if let Err(e) = result {
                left.push(format!("{shown} ({e})"));
            }
        }
        WriteError::RolledBack {
            cause: Box::new(cause),
            left,
        }
    }

    /// Runs `steps` against these changes: what they wrote stands if they
    /// all succeed; otherwise it is all put back and the answer says so.
    pub fn all_or_nothing(
        steps: impl FnOnce(&mut Changes) -> Result<(), WriteError>,
    ) -> Result<(), WriteError> {
        let mut changes = Changes::default();
        match steps(&mut changes) {
            Ok(()) => Ok(()),
            Err(cause) => Err(changes.roll_back(cause)),
        }
    }
}

/// Writes `content` as `plan` says: the new or rewritten file, then the old
/// one of a move removed. A failure at either step puts back what was
/// written ([`Changes`]), so a name is never left in two files.
pub fn write(plan: &Plan, content: &str) -> Result<(), WriteError> {
    Changes::all_or_nothing(|changes| {
        changes.plan(plan, content)?;
        changes.retire(plan)
    })
}

/// `std::fs::remove_file`, which a test can make fail for a path.
fn remove_file(path: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if test_hook::refuses_removal(path) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "refused by the test",
        ));
    }
    std::fs::remove_file(path)
}

/// Lets a test make the removal of a file fail, which a directory's
/// permissions cannot do without failing its creation too.
#[cfg(test)]
pub mod test_hook {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    static REFUSED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

    /// From now on, removing `path` fails (paths are a test's own).
    pub fn refuse_removal_of(path: &Path) {
        REFUSED.lock().unwrap().push(path.to_path_buf());
    }

    pub(super) fn refuses_removal(path: &Path) -> bool {
        REFUSED.lock().unwrap().iter().any(|p| p == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(dir: &Path, write: Write, current: Option<&str>) -> Plan {
        Plan {
            dir: dir.to_path_buf(),
            write,
            retire: None,
            current: current.map(|c| c.as_bytes().to_vec()),
        }
    }

    /// Fails the next write on this thread just after it truncates its
    /// file; the putting back that follows succeeds.
    struct Truncation;

    impl Truncation {
        fn fails() -> Truncation {
            crate::util::fail_after_truncating(1);
            Truncation
        }
    }

    impl Drop for Truncation {
        fn drop(&mut self) {
            crate::util::fail_after_truncating(0);
        }
    }

    #[test]
    fn a_rewrite_failing_after_truncating_is_put_back() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("club.venue");
        std::fs::write(&file, "venue \"club\" {\n}\n").unwrap();
        let rewrite = plan(
            dir.path(),
            Write::Rewrite(file.clone()),
            Some("venue \"club\" {\n}\n"),
        );

        let err = {
            let _failing = Truncation::fails();
            write(&rewrite, "venue \"club\" {\n  # new\n}\n").unwrap_err()
        };

        assert!(err.to_string().ends_with("; nothing was saved"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "venue \"club\" {\n}\n"
        );
    }

    #[test]
    fn a_create_failing_after_making_its_file_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("club.venue");
        let create = plan(dir.path(), Write::Create(file.clone()), None);

        let err = {
            let _failing = Truncation::fails();
            write(&create, "venue \"club\" {\n}\n").unwrap_err()
        };

        assert!(err.to_string().ends_with("; nothing was saved"), "{err}");
        assert!(!file.exists(), "the empty file it made is gone");
    }

    #[test]
    fn a_file_that_appeared_since_its_claim_is_left_as_its_maker_wrote_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("club.venue");
        let create = plan(dir.path(), Write::Create(file.clone()), None);
        std::fs::write(&file, "someone else's\n").unwrap();

        let err = write(&create, "venue \"club\" {\n}\n").unwrap_err();

        assert!(matches!(err.cause(), WriteError::Exists { .. }), "{err:?}");
        assert_eq!(
            err.to_string(),
            "club.venue appeared after it was claimed and was not written over; move or remove \
             it by hand, then try again; nothing was saved"
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "someone else's\n");
    }

    #[test]
    fn a_failure_after_earlier_files_puts_each_back() {
        let dir = tempfile::tempdir().unwrap();
        let (made, rewritten, other) = (
            dir.path().join("new.fixture"),
            dir.path().join("club.venue"),
            dir.path().join("taken.fixture"),
        );
        std::fs::write(&rewritten, "before\n").unwrap();
        std::fs::write(&other, "theirs\n").unwrap();

        let err = Changes::all_or_nothing(|changes| {
            changes.create(&made, b"made\n", "new.fixture".into())?;
            changes.rewrite(&rewritten, b"after\n", b"before\n", "club.venue".into())?;
            changes.create(&other, b"mine\n", "taken.fixture".into())
        })
        .unwrap_err();

        assert!(matches!(err.cause(), WriteError::Exists { .. }), "{err:?}");
        assert!(!made.exists());
        assert_eq!(std::fs::read_to_string(&rewritten).unwrap(), "before\n");
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "theirs\n");
    }
}
