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
    /// The file to be rewritten was removed since it was read; it was not
    /// made again.
    Gone { path: PathBuf, shown: String },
    /// The old file of a move (`shown`) could not be removed.
    NotRetired {
        shown: String,
        error: std::io::Error,
    },
    /// `cause` failed, and everything written before it (and what it
    /// changed itself) was put back, but for `left`: each file that could
    /// not be, and why. `kept` are files written before these changes that
    /// stand anyway (an import's library copies), as the user knows them.
    RolledBack {
        cause: Box<WriteError>,
        left: Vec<String>,
        kept: Vec<String>,
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
            WriteError::Gone { shown, .. } => write!(
                f,
                "{shown} was removed while saving and was not made again; try again"
            ),
            WriteError::NotRetired { shown, error } => {
                write!(f, "cannot remove {shown}: {error}")
            }
            WriteError::RolledBack { cause, left, kept } => {
                write!(f, "{cause}; {}", saved_after_failure(kept, left))
            }
        }
    }
}

/// `items` as one phrase: "A", "A and B", "A, B and C".
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// What a failed write says was saved after it: the library copies that
/// stand whatever happens (`kept`), and the files that could not be put
/// back (`left`, each with why). The one wording for every importer's and
/// writer's failure.
pub fn saved_after_failure(kept: &[String], left: &[String]) -> String {
    let mut said = match kept {
        [] if left.is_empty() => return "nothing was saved".to_string(),
        [] => "nothing was saved".to_string(),
        [one] => format!("only the library copy {one} was saved"),
        many => format!("only the library copies {} were saved", and_list(many)),
    };
    if !left.is_empty() {
        said.push_str(&format!(", but {} could not be put back", and_list(left)));
    }
    said
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
        let mode = if new {
            crate::util::WriteMode::CreateNew
        } else {
            crate::util::WriteMode::ReplaceExisting
        };
        let result = crate::util::write_file_tracked(path, contents, mode);
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
            } else if !new
                && !failure.touched
                && failure.error.kind() == std::io::ErrorKind::NotFound
            {
                WriteError::Gone {
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
        self.roll_back_keeping(cause, Vec::new())
    }

    /// [`Self::roll_back`], saying which files written before these
    /// changes stand anyway (`kept`, as the user knows them).
    pub fn roll_back_keeping(self, cause: WriteError, kept: Vec<String>) -> WriteError {
        let mut left = Vec::new();
        for undo in self.done.into_iter().rev() {
            match undo {
                Undo::Remove { path, shown } => {
                    if let Err(e) = crate::util::remove_written(&path) {
                        left.push(format!("{shown} ({e})"));
                    }
                }
                Undo::Restore { path, shown, was } => {
                    if let Some(why) = restore(&path, &was) {
                        left.push(format!("{shown} ({why})"));
                    }
                }
            }
        }
        WriteError::RolledBack {
            cause: Box::new(cause),
            left,
            kept,
        }
    }

    /// Runs `steps` against these changes: what they wrote stands if they
    /// all succeed; otherwise it is all put back and the answer says so.
    pub fn all_or_nothing(
        steps: impl FnOnce(&mut Changes) -> Result<(), WriteError>,
    ) -> Result<(), WriteError> {
        Self::all_or_nothing_keeping(Vec::new(), steps)
    }

    /// [`Self::all_or_nothing`] after `kept` (as the user knows them) were
    /// written, which stand whatever happens: a failure's answer says so.
    pub fn all_or_nothing_keeping(
        kept: Vec<String>,
        steps: impl FnOnce(&mut Changes) -> Result<(), WriteError>,
    ) -> Result<(), WriteError> {
        let mut changes = Changes::default();
        match steps(&mut changes) {
            Ok(()) => Ok(()),
            Err(cause) => Err(changes.roll_back_keeping(cause, kept)),
        }
    }
}

/// Puts `path` back as it was (`was`) ([`crate::util::restore_file`]);
/// `None` when it is, else what the answer says of it: the file is left as
/// the failed save wrote it, and where its original is.
fn restore(path: &Path, was: &[u8]) -> Option<String> {
    use crate::util::{Original, Restored};
    let Restored::No { error, original } = crate::util::restore_file(path, was) else {
        return None;
    };
    let original = match original {
        Original::Kept(name) => format!("its original is in {name}"),
        Original::Staged(name) => format!(
            "its original is only in {name}, which the next save of this file writes over; \
             copy it somewhere else first"
        ),
        Original::Lost(e) => format!("its original could not be kept either: {e}"),
    };
    Some(format!(
        "{error}; it is left as the failed save wrote it, perhaps empty, and {original}"
    ))
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
            crate::util::inject_faults(&[crate::util::Fault::AfterTruncating]);
            Truncation
        }
    }

    impl Drop for Truncation {
        fn drop(&mut self) {
            crate::util::inject_faults(&[]);
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
        assert!(!crate::util::staged_path(&file).exists(), "no sidecar left");
    }

    #[test]
    fn a_rewrite_of_a_file_removed_since_it_was_read_makes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("club.venue");
        let rewrite = plan(dir.path(), Write::Rewrite(file.clone()), Some("was\n"));

        let err = write(&rewrite, "venue \"club\" {\n}\n").unwrap_err();

        assert!(matches!(err.cause(), WriteError::Gone { .. }), "{err:?}");
        assert_eq!(
            err.to_string(),
            "club.venue was removed while saving and was not made again; try again; nothing \
             was saved"
        );
        assert!(!file.exists());
        assert!(!crate::util::staged_path(&file).exists());
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
        assert!(!crate::util::staged_path(&file).exists(), "no sidecar left");
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

    #[test]
    fn a_failure_says_what_was_saved_in_one_wording() {
        let v = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(saved_after_failure(&[], &[]), "nothing was saved");
        assert_eq!(
            saved_after_failure(&v(&["A"]), &[]),
            "only the library copy A was saved"
        );
        assert_eq!(
            saved_after_failure(&v(&["A", "B", "C"]), &[]),
            "only the library copies A, B and C were saved"
        );
        assert_eq!(
            saved_after_failure(&[], &v(&["X (e)"])),
            "nothing was saved, but X (e) could not be put back"
        );
        assert_eq!(
            saved_after_failure(&v(&["A"]), &v(&["X (e)", "Y (f)"])),
            "only the library copy A was saved, but X (e) and Y (f) could not be put back"
        );
    }

    use crate::util::Fault;

    /// A venue rewrite whose write fails after truncating, then the restore
    /// meets `restore`: the error, the file's text, and every file beside it.
    fn failed_rewrite(
        restore: &[Fault],
        before: &[(&str, &str)],
    ) -> (String, String, Vec<(String, String)>) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("club.venue");
        std::fs::write(&file, "was\n").unwrap();
        for (name, text) in before {
            std::fs::write(dir.path().join(name), text).unwrap();
        }
        let rewrite = plan(dir.path(), Write::Rewrite(file.clone()), Some("was\n"));
        let mut faults = vec![Fault::AfterTruncating];
        faults.extend_from_slice(restore);
        crate::util::inject_faults(&faults);
        let err = write(&rewrite, "new\n").unwrap_err().to_string();
        crate::util::inject_faults(&[]);
        let text = std::fs::read_to_string(&file).unwrap();
        let mut files: Vec<(String, String)> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .map(|p| {
                (
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    std::fs::read_to_string(&p).unwrap(),
                )
            })
            .collect();
        files.sort();
        (err, text, files)
    }

    #[test]
    fn a_restore_failing_before_truncating_keeps_the_original_in_a_file_of_its_own() {
        let (err, text, files) = failed_rewrite(&[Fault::Staging], &[]);

        assert_eq!(text, "", "left as the failed save wrote it");
        assert_eq!(
            files,
            [
                ("club.venue".to_string(), String::new()),
                (
                    "club.venue.mtrack-original".to_string(),
                    "was\n".to_string()
                ),
            ],
            "no staged copy is left"
        );
        assert!(
            err.ends_with(
                "; nothing was saved, but club.venue (injected staging failure; it is left as the \
                 failed save wrote it, perhaps empty, and its original is in \
                 club.venue.mtrack-original) could not be put back"
            ),
            "{err}"
        );
    }

    #[test]
    fn a_restore_failing_after_truncating_keeps_the_original_in_a_file_of_its_own() {
        let (err, text, files) = failed_rewrite(&[Fault::AfterTruncating], &[]);

        assert_eq!(text, "");
        assert_eq!(
            files,
            [
                ("club.venue".to_string(), String::new()),
                (
                    "club.venue.mtrack-original".to_string(),
                    "was\n".to_string()
                ),
            ]
        );
        assert!(
            err.contains(
                "club.venue (injected failure after truncating; it is left as the failed save \
                 wrote it, perhaps empty, and its original is in club.venue.mtrack-original)"
            ),
            "{err}"
        );
    }

    #[test]
    fn a_restore_whose_only_failure_is_its_staged_copy_s_removal_is_put_back() {
        let (err, text, files) = failed_rewrite(&[Fault::SidecarRemoval], &[]);

        assert_eq!(text, "was\n");
        assert_eq!(files, [("club.venue".to_string(), "was\n".to_string())]);
        assert!(err.ends_with("; nothing was saved"), "{err}");
    }

    #[test]
    fn a_taken_recovery_name_is_never_written_over() {
        let (err, _, files) = failed_rewrite(
            &[Fault::Staging],
            &[("club.venue.mtrack-original", "an earlier original\n")],
        );

        assert_eq!(
            files,
            [
                ("club.venue".to_string(), String::new()),
                (
                    "club.venue.mtrack-original".to_string(),
                    "an earlier original\n".to_string()
                ),
                (
                    "club.venue.mtrack-original-2".to_string(),
                    "was\n".to_string()
                ),
            ]
        );
        assert!(
            err.contains("its original is in club.venue.mtrack-original-2"),
            "{err}"
        );
    }

    #[test]
    fn an_original_that_cannot_be_kept_leaves_the_staged_copy_and_names_it() {
        let (err, text, files) = failed_rewrite(
            &[
                Fault::AfterTruncating,
                Fault::Link,
                Fault::Staging,
                Fault::Rename,
            ],
            &[],
        );

        let staged = crate::util::staged_path(Path::new("club.venue"))
            .to_string_lossy()
            .into_owned();
        assert_eq!(text, "");
        assert_eq!(
            files,
            [
                ("club.venue".to_string(), String::new()),
                (staged.clone(), "was\n".to_string()),
            ],
            "the staged copy, holding the original, is all that holds it"
        );
        assert!(
            err.contains(&format!(
                "club.venue (injected failure after truncating; it is left as the failed save \
                 wrote it, perhaps empty, and its original is only in {staged}, which the next \
                 save of this file writes over; copy it somewhere else first)"
            )),
            "{err}"
        );
    }

    /// #685: with no link and no room to write the original out, the
    /// staged copy is moved to a kept original's name, which no later
    /// write uses: the next save of the file does not write over it.
    #[test]
    fn an_original_neither_linked_nor_written_out_is_moved_to_a_name_of_its_own() {
        let (err, text, files) = failed_rewrite(
            &[Fault::AfterTruncating, Fault::Link, Fault::Staging],
            &[("club.venue.mtrack-original", "an earlier original\n")],
        );

        assert_eq!(text, "");
        assert_eq!(
            files,
            [
                ("club.venue".to_string(), String::new()),
                (
                    "club.venue.mtrack-original".to_string(),
                    "an earlier original\n".to_string()
                ),
                (
                    "club.venue.mtrack-original-2".to_string(),
                    "was\n".to_string()
                ),
            ],
            "moved, not copied: no staged copy is left, and a taken name is skipped"
        );
        assert!(
            err.contains("its original is in club.venue.mtrack-original-2"),
            "{err}"
        );
    }

    #[test]
    fn an_original_whose_link_fails_is_written_out_instead() {
        let (err, text, files) = failed_rewrite(&[Fault::AfterTruncating, Fault::Link], &[]);

        assert_eq!(text, "");
        assert_eq!(
            files,
            [
                ("club.venue".to_string(), String::new()),
                (
                    "club.venue.mtrack-original".to_string(),
                    "was\n".to_string()
                ),
            ],
            "written out, and the staged copy gone"
        );
        assert!(
            err.contains("its original is in club.venue.mtrack-original"),
            "{err}"
        );
    }

    #[test]
    fn a_linked_original_is_the_staged_copy_itself() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("club.venue");
        std::fs::write(&file, "was\n").unwrap();
        let rewrite = plan(dir.path(), Write::Rewrite(file.clone()), Some("was\n"));
        // A written-out original would meet the staging failure; the link
        // needs no write.
        crate::util::inject_faults(&[
            Fault::AfterTruncating,
            Fault::AfterTruncating,
            Fault::Pass,
            Fault::Staging,
        ]);
        let err = write(&rewrite, "new\n").unwrap_err().to_string();
        crate::util::inject_faults(&[]);
        assert!(
            err.contains("its original is in club.venue.mtrack-original"),
            "{err}"
        );

        let original = dir.path().join("club.venue.mtrack-original");
        assert_eq!(std::fs::read_to_string(&original).unwrap(), "was\n");
        assert_eq!(
            std::fs::metadata(&original).unwrap().nlink(),
            1,
            "the staged copy let go once linked"
        );
        assert!(!crate::util::staged_path(&file).exists());
    }
}
