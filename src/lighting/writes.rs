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
                write: Write::Create(upgraded),
                retire: Some(owner.path),
                current: Some(current),
            });
        }
        Ok(Plan {
            write: Write::Rewrite(owner.path),
            retire: None,
            current: Some(current),
        })
    }
}

/// Why [`write`] failed.
#[derive(Debug)]
pub enum WriteError {
    /// The file to be made is there by now; nothing was written.
    Exists(PathBuf),
    /// The file could not be written.
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    /// The new file was written but the old one, `old`, could not be
    /// removed. The new one was removed again (`undone` is `Ok`), or that
    /// failed too.
    NotRetired {
        old: PathBuf,
        error: std::io::Error,
        new: PathBuf,
        undone: std::io::Result<()>,
    },
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Exists(path) => write!(
                f,
                "{} already exists; move or remove it by hand, then save again",
                path.display()
            ),
            WriteError::Io { path, error } => {
                f.write_str(&crate::util::write_file_message(path, error))
            }
            WriteError::NotRetired {
                old,
                error,
                new,
                undone,
            } => {
                write!(f, "cannot remove {}: {error}; ", old.display())?;
                match undone {
                    Ok(()) => f.write_str("nothing was saved"),
                    Err(u) => write!(
                        f,
                        "{} was written and cannot be removed: {u}",
                        new.display()
                    ),
                }
            }
        }
    }
}

impl std::error::Error for WriteError {}

/// Writes `content` as `plan` says. A file to be made that is there by now
/// is refused, nothing written. When the old file cannot be removed after
/// a new one is made, the new one is removed again and the save fails, so
/// a name is never left in two files.
pub fn write(plan: &Plan, content: &str) -> Result<(), WriteError> {
    match &plan.write {
        Write::Rewrite(path) => {
            crate::util::write_file(path, content.as_bytes()).map_err(|error| WriteError::Io {
                path: path.clone(),
                error,
            })?
        }
        Write::Create(path) => match crate::util::write_new_file(path, content.as_bytes()) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(WriteError::Exists(path.clone()))
            }
            Err(error) => {
                return Err(WriteError::Io {
                    path: path.clone(),
                    error,
                })
            }
        },
    }
    if let (Some(old), Write::Create(new)) = (&plan.retire, &plan.write) {
        if let Err(error) = std::fs::remove_file(old) {
            return Err(WriteError::NotRetired {
                old: old.clone(),
                error,
                new: new.clone(),
                undone: std::fs::remove_file(new),
            });
        }
    }
    Ok(())
}
