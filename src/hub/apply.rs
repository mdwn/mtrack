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
//! is in a source directory (`staging/files` for a pull, `previous/files`
//! for an undo), by its path, checked against its hash. [`preflight`] looks
//! for what would stop a move part-way. A [`Journal`] of what will move is
//! written first. Then each file the project has and the manifest replaces
//! or lacks moves to `displaced/files`, and each source file moves to its
//! place. Each move is a rename inside the project's directory, the only one
//! a packaged player may write to. When all are made the journal says so;
//! then `displaced` becomes `previous` (what [`undo`] puts back), the state
//! is recorded and the journal goes.
//!
//! Cut short anywhere, the journal is still there, and [`recover`] carries
//! on: a move is made only while its source file is still in the source
//! directory, and none is made once the journal says all were.
//!
//! What this cannot do is go back. A move that fails for a reason that does
//! not pass (a disk that has gone read-only) leaves the project part-moved,
//! with the journal saying so, until the reason is dealt with and a hub
//! command or the player is run again. [`preflight`] is there to make that
//! rare.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use mtrack_core::project::Manifest;
use serde::{Deserialize, Serialize};

use super::state::{read_json, write_json, StateDir, SyncState};

/// Where a journal's incoming files are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    /// `staging/files`: a pull.
    Staging,
    /// `previous/files`: an undo.
    Previous,
}

/// A pull's moves, kept on disk while they are made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    pub source: Source,
    /// The project's files once every move is made.
    pub manifest: Manifest,
    /// What to record the project as then: the revision and digest it is at.
    /// None for a project that was never the hub's (an undo back to that).
    pub state: Option<SyncState>,
    /// The project's files before any move was.
    pub before: Manifest,
    /// What the project was recorded as before, for the undo.
    pub state_before: Option<SyncState>,
    /// Whether every move has been made.
    #[serde(default)]
    pub moved: bool,
}

/// What a pull replaced, kept to undo it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Previous {
    /// What the project was recorded as before the pull.
    pub state: Option<SyncState>,
    /// The project's files before it.
    pub manifest: Manifest,
    /// The project's files after it. What differs between the two is what
    /// the pull touched, and all an undo puts back.
    pub after: Manifest,
}

/// The file a project path names under `root`.
pub fn file_at(root: &Path, path: &str) -> PathBuf {
    path.split('/')
        .fold(root.to_path_buf(), |at, part| at.join(part))
}

/// The paths of `to` whose file `from` lacks or has otherwise.
pub fn changed<'a>(from: &'a Manifest, to: &'a Manifest) -> impl Iterator<Item = &'a String> {
    to.files
        .iter()
        .filter(move |(path, entry)| from.files.get(*path) != Some(entry))
        .map(|(path, _)| path)
}

/// The paths of `from` that `to` does not have.
pub fn removed<'a>(from: &'a Manifest, to: &'a Manifest) -> impl Iterator<Item = &'a String> {
    from.files
        .keys()
        .filter(move |path| !to.files.contains_key(*path))
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

/// Files a folder may hold and still count as empty: mtrack's own song
/// cache, the probe [`preflight`] writes, and the one a Mac leaves in every
/// folder it has shown. None is a project's file, and each is made again
/// when wanted.
const NOTHING: [&str; 3] = [".mtrack-cache.json", PROBE, ".DS_Store"];

/// The file [`preflight`] makes and removes in a folder to see that it can.
const PROBE: &str = ".mtrack-hub-probe";

/// Whether `dir` holds nothing but [`NOTHING`] and folders that do too.
fn holds_nothing(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.into_iter().all(|entry| {
        let Ok(entry) = entry else { return false };
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => holds_nothing(&entry.path()),
            Ok(kind) if kind.is_file() => entry
                .file_name()
                .to_str()
                .is_some_and(|name| NOTHING.contains(&name)),
            _ => false,
        }
    })
}

/// Removes the directories above `path`, up to `root`, that now hold
/// nothing ([`holds_nothing`]).
fn prune(root: &Path, path: &str) {
    let mut at = file_at(root, path);
    while at.pop() && at != root && at.starts_with(root) {
        if !at.is_dir() {
            continue;
        }
        if !holds_nothing(&at) || std::fs::remove_dir_all(&at).is_err() {
            break;
        }
    }
}

fn source_dir(state: &StateDir, source: Source) -> PathBuf {
    match source {
        Source::Staging => state.staging().join("files"),
        Source::Previous => state.previous().join("files"),
    }
}

fn refusal(message: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
}

/// Whether everything under `dir` is a file this pull takes out (`leaving`,
/// by project path under `prefix`) or counts as nothing ([`NOTHING`]): a
/// folder that will be gone, or removable, once the pull's removals are
/// made.
fn will_be_empty(dir: &Path, prefix: &str, leaving: &BTreeSet<&String>) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.into_iter().all(|entry| {
        let Ok(entry) = entry else { return false };
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            return false;
        };
        let path = format!("{prefix}/{name}");
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => will_be_empty(&entry.path(), &path, leaving),
            Ok(kind) if kind.is_file() => {
                NOTHING.contains(&name.as_str()) || leaving.contains(&path)
            }
            _ => false,
        }
    })
}

/// Looks, before anything moves, for what would stop a move part-way: a
/// folder on another file system than the project (a rename cannot cross
/// one), `.mtrack-hub` being on another, a folder a file has to be made in
/// or taken from that cannot be written, and something not the project's
/// standing where a file or folder is to go. Nothing is changed.
pub fn preflight(state: &StateDir, before: &Manifest, manifest: &Manifest) -> std::io::Result<()> {
    let project = state.project();
    #[cfg(unix)]
    let home = {
        use std::os::unix::fs::MetadataExt;
        let home = std::fs::metadata(project)?.dev();
        std::fs::create_dir_all(state.dir())?;
        if std::fs::metadata(state.dir())?.dev() != home {
            return Err(refusal(format!(
                "{} is on another disk than the project's folder; a pull cannot move files \
                 between them",
                state.dir().display()
            )));
        }
        home
    };
    let leaving: BTreeSet<&String> = removed(before, manifest).collect();
    let mut folders = BTreeSet::new();
    for path in removed(before, manifest).chain(changed(before, manifest)) {
        let live = file_at(project, path);
        if live.is_dir() && !will_be_empty(&live, path, &leaving) {
            return Err(refusal(format!(
                "{} is a folder here and a file in the revision, and holds files that are \
                 not the project's. Move or remove it and try again",
                live.display()
            )));
        }
        // The nearest folder above it that is there: where a rename lands.
        let mut folder = live.parent().unwrap_or(project).to_path_buf();
        while !folder.is_dir() {
            if folder.exists() && !before.files.keys().any(|p| file_at(project, p) == folder) {
                return Err(refusal(format!(
                    "{} is a file here and a folder in the revision, and is not one of the \
                     project's files. Move or remove it and try again",
                    folder.display()
                )));
            }
            if !folder.pop() {
                break;
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if folder.is_dir() && std::fs::metadata(&folder)?.dev() != home {
                return Err(refusal(format!(
                    "{} is on another disk than the project's folder; a pull cannot move \
                     files between them",
                    folder.display()
                )));
            }
        }
        if folder.is_dir() {
            folders.insert(folder);
        }
    }
    for folder in folders {
        let probe = folder.join(PROBE);
        std::fs::File::create(&probe)
            .and_then(|_| std::fs::remove_file(&probe))
            .map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!("{} cannot be written to: {e}", folder.display()),
                )
            })?;
    }
    Ok(())
}

/// Makes the moves from `before` to `journal.manifest`, the incoming files
/// being in the journal's source. `journal.before` must be the project's
/// files as they are.
pub fn apply(state: &StateDir, journal: &Journal) -> std::io::Result<()> {
    preflight(state, &journal.before, &journal.manifest)?;
    begin(state, journal)?;
    finish(state, journal.clone())
}

/// Sets a pull's moves down before any is made: an empty `displaced`,
/// holding what an undo of this pull goes back to, and then the journal.
pub(super) fn begin(state: &StateDir, journal: &Journal) -> std::io::Result<()> {
    remove_dir(&state.displaced())?;
    std::fs::create_dir_all(state.displaced())?;
    write_json(
        &state.displaced().join("previous.json"),
        &Previous {
            state: journal.state_before.clone(),
            manifest: journal.before.clone(),
            after: journal.manifest.clone(),
        },
        false,
    )?;
    write_json(&state.journal(), journal, false)
}

/// Whether a pull was cut short and is waiting to be finished.
pub fn pending(state: &StateDir) -> bool {
    state.journal().is_file()
}

/// Finishes a pull that was cut short, if one was. True when there was one.
pub fn recover(state: &StateDir) -> std::io::Result<bool> {
    match read_json::<Journal>(&state.journal())? {
        Some(journal) => finish(state, journal).map(|()| true).map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!(
                    "a pull of this project was cut short and could not be finished ({e}). \
                     The project is part-way between two revisions until it is: deal with \
                     that and run this again"
                ),
            )
        }),
        None => Ok(false),
    }
}

/// Every move of `journal` not yet made, then what follows them. Each step
/// leaves what the next needs, and is skipped when already done.
fn finish(state: &StateDir, mut journal: Journal) -> std::io::Result<()> {
    let project = state.project();
    if !journal.moved {
        let source = source_dir(state, journal.source);
        let displaced = state.displaced().join("files");
        for path in removed(&journal.before, &journal.manifest) {
            let live = file_at(project, path);
            let kept = file_at(&displaced, path);
            // Kept already: a file there now was made since, and is not ours.
            if live.is_file() && !kept.exists() {
                rename(&live, &kept)?;
            }
        }
        // The folders that emptied go before any file comes in: one may be
        // where a file is to go.
        for path in removed(&journal.before, &journal.manifest) {
            prune(project, path);
        }
        for path in changed(&journal.before, &journal.manifest) {
            let incoming = file_at(&source, path);
            if !incoming.is_file() {
                // Already in place.
                continue;
            }
            let live = file_at(project, path);
            if live.is_dir() && holds_nothing(&live) {
                std::fs::remove_dir_all(&live)?;
            }
            if live.is_file() {
                let kept = file_at(&displaced, path);
                if kept.exists() {
                    // The project's own was kept before the pull was cut
                    // short; this one was made since. Kept beside it.
                    let mut beside = kept.into_os_string();
                    beside.push(".made-since");
                    rename(&live, Path::new(&beside))?;
                } else {
                    rename(&live, &kept)?;
                }
            }
            rename(&incoming, &live)?;
        }
        // From here no move is made again, whatever the source then holds.
        journal.moved = true;
        write_json(&state.journal(), &journal, false)?;
    }

    // The folders the moves emptied go: done each time, as it costs nothing
    // and a pull cut short may not have got to it.
    for path in removed(&journal.before, &journal.manifest) {
        prune(project, path);
    }

    // What was displaced is the pull's undo. Once it is `previous` there is
    // no `displaced`.
    if state.displaced().is_dir() {
        remove_dir(&state.previous())?;
        std::fs::rename(state.displaced(), state.previous())?;
    }
    match &journal.state {
        Some(recorded) => state.set_sync_state(recorded)?,
        None => state.clear_sync_state()?,
    }
    remove_dir(&state.staging())?;
    std::fs::remove_file(state.journal())
}

/// What undoing the last pull would go back to, when there is one to undo.
pub fn undoable(state: &StateDir) -> std::io::Result<Option<Previous>> {
    read_json::<Previous>(&state.previous().join("previous.json"))
}

/// Puts back what the last pull replaced: the files it changed or removed
/// as they were, and the files it added gone. A file the pull did not touch
/// is not touched by its undo. `current` is the project's files as they are
/// and `recorded` what they are recorded as. What this
/// replaces is kept in turn, so an undo can itself be undone. The project
/// is recorded as it was before the pull, so files that had been changed
/// here then are known to be changed here still.
pub fn undo(
    state: &StateDir,
    recorded: Option<SyncState>,
    current: &Manifest,
) -> std::io::Result<Option<SyncState>> {
    let Some(previous) = undoable(state)? else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "there is no pull to undo",
        ));
    };
    // Only what the pull touched goes back. A file it did not touch, made
    // or changed here since, stays as it is; so does one it touched whose
    // kept copy is not there.
    let kept = state.previous().join("files");
    let mut manifest = current.clone();
    for path in changed(&previous.manifest, &previous.after) {
        // Changed or added by the pull: as it was, or gone.
        match previous.manifest.files.get(path) {
            Some(entry) if current.files.get(path) == Some(entry) => {}
            Some(entry) => {
                if file_at(&kept, path).is_file() {
                    manifest.files.insert(path.clone(), entry.clone());
                }
            }
            None => {
                manifest.files.remove(path);
            }
        }
    }
    for path in removed(&previous.manifest, &previous.after) {
        // Removed by the pull: back, when it was kept.
        let entry = &previous.manifest.files[path];
        if current.files.get(path) != Some(entry) && file_at(&kept, path).is_file() {
            manifest.files.insert(path.clone(), entry.clone());
        }
    }
    apply(
        state,
        &Journal {
            source: Source::Previous,
            manifest,
            state: previous.state.clone(),
            before: current.clone(),
            state_before: recorded,
            moved: false,
        },
    )?;
    Ok(previous.state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::protocol::Revision;

    fn write(root: &Path, path: &str, contents: &str) {
        let file = file_at(root, path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, contents).unwrap();
    }

    fn read(root: &Path, path: &str) -> Option<String> {
        std::fs::read_to_string(file_at(root, path)).ok()
    }

    fn at(number: u64, manifest: &Manifest) -> Option<SyncState> {
        Some(SyncState {
            revision: Some(Revision {
                id: format!("rev-{number}"),
                number,
            }),
            digest: manifest.digest(),
        })
    }

    fn number(state: &StateDir) -> Option<u64> {
        state
            .sync_state()
            .unwrap()
            .and_then(|recorded| recorded.revision)
            .map(|revision| revision.number)
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
        state.set_sync_state(&at(1, &before).unwrap()).unwrap();
        let staged = state.staging().join("files");
        write(&staged, "songs/a/change.yaml", "new");
        write(&staged, "songs/c/new.wav", "added");
        let journal = Journal {
            source: Source::Staging,
            state: at(2, &manifest),
            manifest,
            state_before: at(1, &before),
            before,
            moved: false,
        };
        (dir, state, journal)
    }

    fn assert_at_revision_2(state: &StateDir, journal: &Journal) {
        let project = state.project();
        assert_eq!(Manifest::of_dir(project).unwrap(), journal.manifest);
        assert_eq!(read(project, "songs/a/change.yaml").as_deref(), Some("new"));
        assert!(!project.join("songs/b").exists(), "an emptied folder goes");
        assert!(!pending(state));
        assert!(!state.staging().exists());
        assert!(!state.displaced().exists());
        assert_eq!(state.sync_state().unwrap(), journal.state);
        // What the pull replaced is there to undo it with.
        let kept = state.previous().join("files");
        assert_eq!(read(&kept, "songs/a/change.yaml").as_deref(), Some("old"));
        assert_eq!(read(&kept, "songs/b/gone.wav").as_deref(), Some("gone"));
        assert_eq!(read(&kept, "mtrack.yaml"), None, "an unchanged file stays");
        let previous = undoable(state).unwrap().unwrap();
        assert_eq!(previous.manifest, journal.before);
        assert_eq!(previous.state, journal.state_before);
    }

    #[test]
    fn a_pull_puts_every_file_in_place_and_keeps_what_it_replaced() {
        let (_dir, state, journal) = staged_pull();
        apply(&state, &journal).unwrap();
        assert_at_revision_2(&state, &journal);
    }

    /// The moves a pull of [`staged_pull`] makes, in order, each by hand.
    fn moves() -> Vec<fn(&StateDir)> {
        fn mv(from: PathBuf, to: PathBuf) {
            rename(&from, &to).unwrap()
        }
        vec![
            |s| {
                mv(
                    file_at(s.project(), "songs/b/gone.wav"),
                    file_at(&s.displaced().join("files"), "songs/b/gone.wav"),
                )
            },
            |s| {
                mv(
                    file_at(s.project(), "songs/a/change.yaml"),
                    file_at(&s.displaced().join("files"), "songs/a/change.yaml"),
                )
            },
            |s| {
                mv(
                    file_at(&s.staging().join("files"), "songs/a/change.yaml"),
                    file_at(s.project(), "songs/a/change.yaml"),
                )
            },
            |s| {
                mv(
                    file_at(&s.staging().join("files"), "songs/c/new.wav"),
                    file_at(s.project(), "songs/c/new.wav"),
                )
            },
        ]
    }

    #[test]
    fn a_pull_cut_short_at_any_move_is_finished_on_recovery() {
        let steps = moves();
        for made in 0..=steps.len() {
            let (_dir, state, journal) = staged_pull();
            begin(&state, &journal).unwrap();
            for step in &steps[..made] {
                step(&state);
            }
            assert!(pending(&state));
            assert!(recover(&state).unwrap(), "after {made} moves");
            assert_at_revision_2(&state, &journal);
            assert!(!recover(&state).unwrap(), "nothing is left to finish");
        }
    }

    #[test]
    fn a_pull_cut_short_after_its_moves_is_finished_without_making_them_again() {
        // After each of the steps that follow the moves.
        type Step = fn(&StateDir, &Journal);
        let after: [Step; 3] = [
            |s, j| {
                let mut moved = j.clone();
                moved.moved = true;
                write_json(&s.journal(), &moved, false).unwrap();
            },
            |s, _| {
                remove_dir(&s.previous()).unwrap();
                std::fs::rename(s.displaced(), s.previous()).unwrap();
            },
            |s, j| s.set_sync_state(j.state.as_ref().unwrap()).unwrap(),
        ];
        for done in 1..=after.len() {
            let (_dir, state, journal) = staged_pull();
            begin(&state, &journal).unwrap();
            for step in moves() {
                step(&state);
            }
            for step in &after[..done] {
                step(&state, &journal);
            }
            assert!(recover(&state).unwrap(), "after {done} closing steps");
            assert_at_revision_2(&state, &journal);
        }
    }

    #[test]
    fn a_file_made_where_one_was_taken_out_is_kept_beside_it() {
        let (_dir, state, journal) = staged_pull();
        begin(&state, &journal).unwrap();
        // Cut short after the project's own file was kept; then someone
        // writes a file of that name before the pull is finished.
        moves()[1](&state);
        write(state.project(), "songs/a/change.yaml", "made since");
        assert!(recover(&state).unwrap());
        assert_eq!(
            read(state.project(), "songs/a/change.yaml").as_deref(),
            Some("new")
        );
        let kept = state.previous().join("files");
        assert_eq!(read(&kept, "songs/a/change.yaml").as_deref(), Some("old"));
        assert_eq!(
            read(&kept, "songs/a/change.yaml.made-since").as_deref(),
            Some("made since")
        );
    }

    #[test]
    fn an_undo_puts_the_project_back_and_can_be_undone() {
        let (_dir, state, journal) = staged_pull();
        let project = state.project().to_path_buf();
        apply(&state, &journal).unwrap();

        let back = undo(&state, journal.state.clone(), &journal.manifest).unwrap();
        assert_eq!(back, journal.state_before);
        assert_eq!(Manifest::of_dir(&project).unwrap(), journal.before);
        assert_eq!(
            read(&project, "songs/a/change.yaml").as_deref(),
            Some("old")
        );
        assert_eq!(read(&project, "songs/b/gone.wav").as_deref(), Some("gone"));
        assert!(!project.join("songs/c").exists());
        assert_eq!(number(&state), Some(1));

        let forward = undo(&state, journal.state_before.clone(), &journal.before).unwrap();
        assert_eq!(forward, journal.state);
        assert_eq!(Manifest::of_dir(&project).unwrap(), journal.manifest);
        assert_eq!(number(&state), Some(2));
    }

    #[test]
    fn an_undo_cut_short_at_any_move_is_finished_and_keeps_its_own_undo() {
        // An undo of [`staged_pull`]'s pull makes two moves out of the
        // project and two into it; cut short after each count of them.
        for made in 0..=4 {
            let (_dir, state, journal) = staged_pull();
            let project = state.project().to_path_buf();
            apply(&state, &journal).unwrap();
            let kept = state.previous().join("files");
            let displaced = state.displaced().join("files");

            // What `undo` sets down, then some of its moves by hand.
            let previous = undoable(&state).unwrap().unwrap();
            let undoing = Journal {
                source: Source::Previous,
                manifest: previous.manifest.clone(),
                state: previous.state.clone(),
                before: journal.manifest.clone(),
                state_before: journal.state.clone(),
                moved: false,
            };
            begin(&state, &undoing).unwrap();
            let by_hand: [(PathBuf, PathBuf); 4] = [
                (
                    file_at(&project, "songs/c/new.wav"),
                    file_at(&displaced, "songs/c/new.wav"),
                ),
                (
                    file_at(&project, "songs/a/change.yaml"),
                    file_at(&displaced, "songs/a/change.yaml"),
                ),
                (
                    file_at(&kept, "songs/a/change.yaml"),
                    file_at(&project, "songs/a/change.yaml"),
                ),
                (
                    file_at(&kept, "songs/b/gone.wav"),
                    file_at(&project, "songs/b/gone.wav"),
                ),
            ];
            for (from, to) in &by_hand[..made] {
                rename(from, to).unwrap();
            }

            assert!(recover(&state).unwrap(), "after {made} moves");
            assert_eq!(Manifest::of_dir(&project).unwrap(), journal.before);
            assert_eq!(number(&state), Some(1));
            // The undo can itself be undone: what it replaced was kept.
            let kept = state.previous().join("files");
            assert_eq!(
                read(&kept, "songs/a/change.yaml").as_deref(),
                Some("new"),
                "after {made} moves"
            );
            assert_eq!(read(&kept, "songs/c/new.wav").as_deref(), Some("added"));
            assert!(!recover(&state).unwrap());
        }
    }

    #[test]
    fn an_undo_goes_back_to_what_the_project_was_recorded_as() {
        // Files changed here, never uploaded, given up to a pull: the pull
        // undone, they are back and known to be changed here still.
        let (_dir, state, mut journal) = staged_pull();
        let synced = Manifest::default();
        journal.state_before = at(1, &synced);
        state.set_sync_state(&at(1, &synced).unwrap()).unwrap();
        apply(&state, &journal).unwrap();

        undo(&state, journal.state.clone(), &journal.manifest).unwrap();
        let recorded = state.sync_state().unwrap().unwrap();
        assert_eq!(Manifest::of_dir(state.project()).unwrap(), journal.before);
        assert_ne!(
            recorded.digest,
            journal.before.digest(),
            "the files put back are not what was last the hub's"
        );
        assert_eq!(recorded.digest, synced.digest());
    }

    #[test]
    fn an_undo_back_to_a_project_that_was_never_the_hubs_records_none() {
        let (_dir, state, mut journal) = staged_pull();
        journal.state_before = None;
        state.clear_sync_state().unwrap();
        apply(&state, &journal).unwrap();
        assert_eq!(number(&state), Some(2));
        undo(&state, journal.state.clone(), &journal.manifest).unwrap();
        assert_eq!(state.sync_state().unwrap(), None);
    }

    #[test]
    fn an_undo_leaves_alone_what_the_pull_did_not_touch() {
        let (_dir, state, journal) = staged_pull();
        let project = state.project().to_path_buf();
        apply(&state, &journal).unwrap();
        // A file the pull left as it was, changed here since.
        write(&project, "mtrack.yaml", "edited after the pull");
        let current = Manifest::of_dir(&project).unwrap();
        undo(&state, journal.state.clone(), &current).unwrap();
        assert_eq!(
            read(&project, "mtrack.yaml").as_deref(),
            Some("edited after the pull")
        );
        assert_eq!(
            read(&project, "songs/a/change.yaml").as_deref(),
            Some("old")
        );
        assert!(!project.join("songs/c").exists());
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
            source: Source::Staging,
            state: at(2, &manifest),
            manifest: manifest.clone(),
            state_before: at(1, &before),
            before: before.clone(),
            moved: false,
        };
        apply(&state, &journal).unwrap();
        assert_eq!(Manifest::of_dir(dir.path()).unwrap(), manifest);
        undo(&state, journal.state.clone(), &manifest).unwrap();
        assert_eq!(Manifest::of_dir(dir.path()).unwrap(), before);
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

    #[test]
    fn a_folder_of_other_files_where_a_file_is_to_go_stops_the_pull_before_it_starts() {
        let (_dir, state, journal) = staged_pull();
        // A folder the project's manifest cannot see (it holds only a file
        // under a name starting with a dot, which someone put there), where
        // the revision has a file.
        write(
            state.project(),
            "songs/c/new.wav/.someones-notes",
            "keep me",
        );
        let before = Manifest::of_dir(state.project()).unwrap();
        assert_eq!(before, journal.before, "the scan does not see it");
        let refused = apply(&state, &journal).unwrap_err();
        assert_eq!(
            refused.kind(),
            std::io::ErrorKind::InvalidInput,
            "{refused}"
        );
        assert!(!pending(&state), "nothing was begun");
        assert_eq!(Manifest::of_dir(state.project()).unwrap(), journal.before);
        assert_eq!(
            read(state.project(), "songs/a/change.yaml").as_deref(),
            Some("old")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_written_stops_the_pull_before_it_starts() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, state, journal) = staged_pull();
        let folder = state.project().join("songs/a");
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root writes anywhere: nothing to see then.
        let refused = std::fs::File::create(folder.join("probe")).is_err();
        let applied = apply(&state, &journal);
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        if refused {
            assert!(applied.is_err());
            assert!(!pending(&state), "nothing was begun");
            assert_eq!(Manifest::of_dir(state.project()).unwrap(), journal.before);
        }
    }

    #[test]
    fn a_pull_that_could_not_be_finished_says_what_state_the_project_is_in() {
        let (_dir, state, journal) = staged_pull();
        begin(&state, &journal).unwrap();
        // Something in the way that was not there when the pull was begun.
        std::fs::remove_file(file_at(state.project(), "songs/a/change.yaml")).unwrap();
        write(state.project(), "songs/a/change.yaml/in-the-way", "x");
        let failed = recover(&state).unwrap_err();
        assert!(failed.to_string().contains("cut short"), "{failed}");
        assert!(pending(&state), "it is still to be finished");
        // Dealt with, it finishes.
        std::fs::remove_dir_all(file_at(state.project(), "songs/a/change.yaml")).unwrap();
        assert!(recover(&state).unwrap());
        assert_eq!(Manifest::of_dir(state.project()).unwrap(), journal.manifest);
    }

    #[test]
    fn an_undo_leaves_alone_a_file_made_here_since_the_pull() {
        let (_dir, state, journal) = staged_pull();
        let project = state.project().to_path_buf();
        apply(&state, &journal).unwrap();
        write(&project, "songs/mine/song.yaml", "recorded tonight");
        let current = Manifest::of_dir(&project).unwrap();
        undo(&state, journal.state.clone(), &current).unwrap();
        assert_eq!(
            read(&project, "songs/mine/song.yaml").as_deref(),
            Some("recorded tonight")
        );
        assert_eq!(
            read(&project, "songs/a/change.yaml").as_deref(),
            Some("old")
        );
        // And undone again, it is still there.
        let current = Manifest::of_dir(&project).unwrap();
        undo(&state, state.sync_state().unwrap(), &current).unwrap();
        assert_eq!(
            read(&project, "songs/mine/song.yaml").as_deref(),
            Some("recorded tonight")
        );
        assert_eq!(
            read(&project, "songs/a/change.yaml").as_deref(),
            Some("new")
        );
    }

    /// A pull between two trees, each given as (path, contents), and back.
    fn pull_and_undo(before: &[(&str, &str)], after: &[(&str, &str)], extra: &[(&str, &str)]) {
        let dir = tempfile::tempdir().unwrap();
        for (path, contents) in before {
            write(dir.path(), path, contents);
        }
        let from = Manifest::of_dir(dir.path()).unwrap();
        for (path, contents) in extra {
            write(dir.path(), path, contents);
        }
        let target = tempfile::tempdir().unwrap();
        for (path, contents) in after {
            write(target.path(), path, contents);
        }
        let to = Manifest::of_dir(target.path()).unwrap();
        let state = StateDir::of(dir.path());
        for path in changed(&from, &to) {
            let contents = std::fs::read(file_at(target.path(), path)).unwrap();
            let staged = file_at(&state.staging().join("files"), path);
            std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
            std::fs::write(staged, contents).unwrap();
        }
        let journal = Journal {
            source: Source::Staging,
            state: at(2, &to),
            manifest: to.clone(),
            state_before: at(1, &from),
            before: from.clone(),
            moved: false,
        };
        apply(&state, &journal).unwrap();
        assert_eq!(Manifest::of_dir(dir.path()).unwrap(), to);
        undo(&state, journal.state.clone(), &to).unwrap();
        assert_eq!(Manifest::of_dir(dir.path()).unwrap(), from);
    }

    #[test]
    fn a_folder_becomes_a_file_and_a_file_a_folder_and_back() {
        // A folder of the project's files where the revision has a file.
        pull_and_undo(
            &[
                ("mtrack.yaml", "c"),
                ("extra/one.txt", "1"),
                ("extra/two.txt", "2"),
            ],
            &[("mtrack.yaml", "c"), ("extra", "now a file")],
            &[],
        );
        // A file where the revision has a folder.
        pull_and_undo(
            &[("mtrack.yaml", "c"), ("extra", "a file")],
            &[("mtrack.yaml", "c"), ("extra/deep/one.txt", "1")],
            &[],
        );
        // The same folder, holding the player's own cache as well, which is
        // nothing to keep.
        pull_and_undo(
            &[("mtrack.yaml", "c"), ("songs/one/song.yaml", "s")],
            &[("mtrack.yaml", "c"), ("songs/one", "now a file")],
            &[("songs/one/.mtrack-cache.json", "{}")],
        );
    }

    #[test]
    fn a_song_taken_out_leaves_no_folder_behind_for_its_cache() {
        let (_dir, state, journal) = staged_pull();
        write(state.project(), "songs/b/.mtrack-cache.json", "{}");
        apply(&state, &journal).unwrap();
        assert!(!state.project().join("songs/b").exists());
    }

    #[test]
    fn an_undo_cut_short_after_its_moves_does_not_take_its_own_undo_for_its_source() {
        // The undo's moves all made, then each of the steps that follow;
        // then recovery. After the second, `previous` holds what the undo
        // displaced, at the very paths its moves read from.
        for done in 1..=2 {
            let (_dir, state, journal) = staged_pull();
            let project = state.project().to_path_buf();
            apply(&state, &journal).unwrap();
            let previous = undoable(&state).unwrap().unwrap();
            let mut undoing = Journal {
                source: Source::Previous,
                manifest: previous.manifest.clone(),
                state: previous.state.clone(),
                before: journal.manifest.clone(),
                state_before: journal.state.clone(),
                moved: false,
            };
            begin(&state, &undoing).unwrap();
            let kept = state.previous().join("files");
            let displaced = state.displaced().join("files");
            for (from, to) in [
                (
                    file_at(&project, "songs/c/new.wav"),
                    file_at(&displaced, "songs/c/new.wav"),
                ),
                (
                    file_at(&project, "songs/a/change.yaml"),
                    file_at(&displaced, "songs/a/change.yaml"),
                ),
                (
                    file_at(&kept, "songs/a/change.yaml"),
                    file_at(&project, "songs/a/change.yaml"),
                ),
                (
                    file_at(&kept, "songs/b/gone.wav"),
                    file_at(&project, "songs/b/gone.wav"),
                ),
            ] {
                rename(&from, &to).unwrap();
            }
            undoing.moved = true;
            write_json(&state.journal(), &undoing, false).unwrap();
            if done == 2 {
                remove_dir(&state.previous()).unwrap();
                std::fs::rename(state.displaced(), state.previous()).unwrap();
            }

            assert!(recover(&state).unwrap(), "after {done} closing steps");
            assert_eq!(
                Manifest::of_dir(&project).unwrap(),
                journal.before,
                "after {done} closing steps: the undo's own undo was not moved back in"
            );
            assert_eq!(
                read(&project, "songs/a/change.yaml").as_deref(),
                Some("old")
            );
            let kept = state.previous().join("files");
            assert_eq!(read(&kept, "songs/a/change.yaml").as_deref(), Some("new"));
            assert_eq!(number(&state), Some(1));
        }
    }
}
