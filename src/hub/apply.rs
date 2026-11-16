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
//! Putting fetched files in place, so that it is done whole or can be
//! finished: the one step of a pull that changes the project.
//!
//! Before this runs, every file the new manifest needs and the project lacks
//! is in `staging/files`, by its path, checked against its hash. A [`Journal`]
//! of what will move is written first. Then each file the project has and the
//! manifest replaces or lacks moves to `displaced/files`, and each staged
//! file moves to its place. Each move is a rename inside the project's
//! directory, the only one a packaged player may write to. When all are made,
//! `displaced` becomes `previous` (what [`undo`] puts back), the state is
//! recorded and the journal goes.
//!
//! Cut short anywhere, the journal is still there, and [`recover`] makes the
//! moves that were not made: every one can be told from what is on disk (a
//! staged file still staged is one not yet moved), so running them again is
//! safe.

use std::path::{Path, PathBuf};

use mtrack_core::project::Manifest;
use serde::{Deserialize, Serialize};

use super::protocol::Revision;
use super::state::{read_json, write_json, StateDir};

/// A pull's moves, kept on disk while they are made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    /// The revision the project's files become.
    pub revision: Option<Revision>,
    /// The project's files once every move is made.
    pub manifest: Manifest,
    /// The project's files before any was.
    pub before: Manifest,
    /// The revision they were at.
    pub revision_before: Option<Revision>,
}

/// What a pull replaced, kept to undo it.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Previous {
    revision: Option<Revision>,
    manifest: Manifest,
}

/// The file a project path names under `root`.
pub fn file_at(root: &Path, path: &str) -> PathBuf {
    path.split('/')
        .fold(root.to_path_buf(), |at, part| at.join(part))
}

/// The paths of `to` whose file `from` lacks or has otherwise.
pub fn changed<'a>(from: &Manifest, to: &'a Manifest) -> impl Iterator<Item = &'a String> {
    let from = from.files.clone();
    to.files
        .iter()
        .filter(move |(path, entry)| from.get(*path) != Some(entry))
        .map(|(path, _)| path)
}

/// The paths of `from` that `to` does not have.
pub fn removed<'a>(from: &'a Manifest, to: &Manifest) -> impl Iterator<Item = &'a String> {
    let to = to.files.clone();
    from.files
        .keys()
        .filter(move |path| !to.contains_key(*path))
}

fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(from, to)
}

fn remove_dir(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Removes the directories above `path`, up to `root`, that are now empty.
fn prune(root: &Path, path: &str) {
    let mut at = file_at(root, path);
    while at.pop() && at != root && at.starts_with(root) {
        // Fails on the first that still holds something, which ends it.
        if std::fs::remove_dir(&at).is_err() {
            break;
        }
    }
}

/// Makes the journal's moves, the staged files being in `staging/files`.
/// `journal.before` must be the project's files as they are.
pub fn apply(state: &StateDir, journal: &Journal) -> std::io::Result<()> {
    begin(state, journal)?;
    finish(state, journal)
}

/// Sets a pull's moves down before any is made: an empty `displaced`,
/// holding what an undo of this pull goes back to, and then the journal.
/// `displaced` being there is how [`finish`] knows it has not yet become
/// `previous`.
fn begin(state: &StateDir, journal: &Journal) -> std::io::Result<()> {
    remove_dir(&state.displaced())?;
    std::fs::create_dir_all(state.displaced())?;
    write_json(
        &state.displaced().join("previous.json"),
        &Previous {
            revision: journal.revision_before.clone(),
            manifest: journal.before.clone(),
        },
        false,
    )?;
    write_json(&state.journal(), journal, false)
}

/// Finishes a pull that was cut short, if one was. True when there was one.
pub fn recover(state: &StateDir) -> std::io::Result<bool> {
    match read_json::<Journal>(&state.journal())? {
        Some(journal) => finish(state, &journal).map(|()| true),
        None => Ok(false),
    }
}

/// Every move of `journal` not yet made, then what follows them. Each step
/// leaves what the next needs, and is skipped when already done.
fn finish(state: &StateDir, journal: &Journal) -> std::io::Result<()> {
    let project = state.project();
    let staged = state.staging().join("files");
    let displaced = state.displaced().join("files");

    for path in removed(&journal.before, &journal.manifest) {
        let live = file_at(project, path);
        if live.is_file() {
            rename(&live, &file_at(&displaced, path))?;
        }
        prune(project, path);
    }
    for path in changed(&journal.before, &journal.manifest) {
        let incoming = file_at(&staged, path);
        if !incoming.is_file() {
            // Already in place.
            continue;
        }
        let live = file_at(project, path);
        if live.is_file() {
            rename(&live, &file_at(&displaced, path))?;
        }
        rename(&incoming, &live)?;
    }

    // What was displaced is the pull's undo. Once it is `previous` there is
    // no `displaced`, and a second run must not take `previous` for the one
    // before it.
    if state.displaced().is_dir() {
        remove_dir(&state.previous())?;
        std::fs::rename(state.displaced(), state.previous())?;
    }

    state.record(journal.revision.clone(), &journal.manifest)?;
    remove_dir(&state.staging())?;
    std::fs::remove_file(state.journal())
}

/// What undoing the last pull would go back to, when there is one to undo.
pub fn undoable(state: &StateDir) -> std::io::Result<Option<(Option<Revision>, Manifest)>> {
    Ok(
        read_json::<Previous>(&state.previous().join("previous.json"))?
            .map(|previous| (previous.revision, previous.manifest)),
    )
}

/// Puts back what the last pull replaced. `current` is the project's files
/// as they are and `revision` the revision they are at; what this replaces
/// is kept in turn, so an undo can itself be undone.
pub fn undo(
    state: &StateDir,
    revision: Option<Revision>,
    current: &Manifest,
) -> std::io::Result<Option<Revision>> {
    let Some((revision_before, manifest)) = undoable(state)? else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "there is no pull to undo",
        ));
    };
    // The kept files become the staged ones of a pull back to them.
    remove_dir(&state.staging())?;
    std::fs::rename(state.previous(), state.staging())?;
    apply(
        state,
        &Journal {
            revision: revision_before.clone(),
            manifest,
            before: current.clone(),
            revision_before: revision,
        },
    )?;
    Ok(revision_before)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, path: &str, contents: &str) {
        let file = file_at(root, path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, contents).unwrap();
    }

    fn read(root: &Path, path: &str) -> Option<String> {
        std::fs::read_to_string(file_at(root, path)).ok()
    }

    fn revision(number: u64) -> Option<Revision> {
        Some(Revision {
            id: format!("rev-{number}"),
            number,
        })
    }

    /// A project of three files at revision 1, and the files of revision 2
    /// staged: `keep` as it was, `change` changed, `gone` gone, `new` new.
    fn staged_pull() -> (tempfile::TempDir, StateDir, Journal) {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path();
        write(project, "mtrack.yaml", "keep");
        write(project, "songs/a/change.yaml", "old");
        write(project, "songs/b/gone.wav", "gone");
        let before = Manifest::of_dir(project).unwrap();

        let target = tempfile::tempdir().unwrap();
        write(target.path(), "mtrack.yaml", "keep");
        write(target.path(), "songs/a/change.yaml", "new");
        write(target.path(), "songs/c/new.wav", "added");
        let manifest = Manifest::of_dir(target.path()).unwrap();

        let state = StateDir::of(project);
        let staged = state.staging().join("files");
        write(&staged, "songs/a/change.yaml", "new");
        write(&staged, "songs/c/new.wav", "added");
        let journal = Journal {
            revision: revision(2),
            manifest,
            before,
            revision_before: revision(1),
        };
        (dir, state, journal)
    }

    fn assert_at_revision_2(state: &StateDir, journal: &Journal) {
        let project = state.project();
        assert_eq!(Manifest::of_dir(project).unwrap(), journal.manifest);
        assert_eq!(read(project, "songs/a/change.yaml").as_deref(), Some("new"));
        assert!(!project.join("songs/b").exists(), "an emptied folder goes");
        assert!(!state.journal().exists());
        assert!(!state.staging().exists());
        let recorded = state.sync_state().unwrap().unwrap();
        assert_eq!(recorded.revision, revision(2));
        assert_eq!(recorded.digest, journal.manifest.digest());
    }

    #[test]
    fn a_pull_puts_every_file_in_place_and_keeps_what_it_replaced() {
        let (_dir, state, journal) = staged_pull();
        apply(&state, &journal).unwrap();
        assert_at_revision_2(&state, &journal);
        let kept = state.previous().join("files");
        assert_eq!(read(&kept, "songs/a/change.yaml").as_deref(), Some("old"));
        assert_eq!(read(&kept, "songs/b/gone.wav").as_deref(), Some("gone"));
        assert_eq!(read(&kept, "mtrack.yaml"), None, "an unchanged file stays");
    }

    #[test]
    fn a_pull_cut_short_at_any_move_is_finished_on_recovery() {
        // Each prefix of the moves a pull makes, made by hand; then recovery.
        type Step = fn(&StateDir);
        let steps: [Step; 4] = [
            |s| {
                rename(
                    &file_at(s.project(), "songs/b/gone.wav"),
                    &file_at(&s.displaced().join("files"), "songs/b/gone.wav"),
                )
                .unwrap()
            },
            |s| {
                rename(
                    &file_at(s.project(), "songs/a/change.yaml"),
                    &file_at(&s.displaced().join("files"), "songs/a/change.yaml"),
                )
                .unwrap()
            },
            |s| {
                rename(
                    &file_at(&s.staging().join("files"), "songs/a/change.yaml"),
                    &file_at(s.project(), "songs/a/change.yaml"),
                )
                .unwrap()
            },
            |s| {
                rename(
                    &file_at(&s.staging().join("files"), "songs/c/new.wav"),
                    &file_at(s.project(), "songs/c/new.wav"),
                )
                .unwrap()
            },
        ];
        for made in 0..=steps.len() {
            let (_dir, state, journal) = staged_pull();
            begin(&state, &journal).unwrap();
            for step in &steps[..made] {
                step(&state);
            }
            assert!(recover(&state).unwrap(), "after {made} moves");
            assert_at_revision_2(&state, &journal);
            let kept = state.previous().join("files");
            assert_eq!(
                read(&kept, "songs/a/change.yaml").as_deref(),
                Some("old"),
                "after {made} moves"
            );
            assert_eq!(read(&kept, "songs/b/gone.wav").as_deref(), Some("gone"));
            assert!(!recover(&state).unwrap(), "nothing is left to finish");
        }
    }

    #[test]
    fn recovery_after_the_moves_but_before_the_journal_went_changes_nothing() {
        let (_dir, state, journal) = staged_pull();
        apply(&state, &journal).unwrap();
        // The journal back, as if it had not been removed.
        write_json(&state.journal(), &journal, false).unwrap();
        assert!(recover(&state).unwrap());
        assert_at_revision_2(&state, &journal);
        // What the pull replaced is still there to undo it with.
        let kept = state.previous().join("files");
        assert_eq!(read(&kept, "songs/a/change.yaml").as_deref(), Some("old"));
        assert_eq!(undoable(&state).unwrap().unwrap().0, revision(1));
    }

    #[test]
    fn a_pull_that_only_adds_files_can_still_be_undone() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "mtrack.yaml", "keep");
        let before = Manifest::of_dir(dir.path()).unwrap();
        let state = StateDir::of(dir.path());
        write(&state.staging().join("files"), "songs/c/new.wav", "added");
        let mut manifest = before.clone();
        manifest.files.insert(
            "songs/c/new.wav".to_string(),
            mtrack_core::project::hash_file(&file_at(
                &state.staging().join("files"),
                "songs/c/new.wav",
            ))
            .unwrap(),
        );
        let journal = Journal {
            revision: revision(2),
            manifest: manifest.clone(),
            before: before.clone(),
            revision_before: revision(1),
        };
        apply(&state, &journal).unwrap();
        assert_eq!(Manifest::of_dir(dir.path()).unwrap(), manifest);
        undo(&state, revision(2), &manifest).unwrap();
        assert_eq!(Manifest::of_dir(dir.path()).unwrap(), before);
    }

    #[test]
    fn an_undo_puts_the_project_back_and_can_be_undone() {
        let (_dir, state, journal) = staged_pull();
        let project = state.project().to_path_buf();
        apply(&state, &journal).unwrap();

        let back = undo(&state, revision(2), &journal.manifest).unwrap();
        assert_eq!(back, revision(1));
        assert_eq!(Manifest::of_dir(&project).unwrap(), journal.before);
        assert_eq!(
            read(&project, "songs/a/change.yaml").as_deref(),
            Some("old")
        );
        assert_eq!(read(&project, "songs/b/gone.wav").as_deref(), Some("gone"));
        assert!(!project.join("songs/c").exists());
        assert_eq!(state.sync_state().unwrap().unwrap().revision, revision(1));

        let forward = undo(&state, revision(1), &journal.before).unwrap();
        assert_eq!(forward, revision(2));
        assert_eq!(Manifest::of_dir(&project).unwrap(), journal.manifest);
    }

    #[test]
    fn there_is_nothing_to_undo_before_a_pull() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateDir::of(dir.path());
        assert!(undoable(&state).unwrap().is_none());
        assert_eq!(
            undo(&state, None, &Manifest::default()).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }
}
