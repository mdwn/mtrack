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
//! A project as a set of files: every file under the project's directory, by
//! path, with its size and SHA-256. Two copies of a project are the same when
//! their manifests are, and a copy is brought up to date by fetching the
//! files whose hashes differ.
//!
//! The project's directory is the one its player config is in
//! ([`crate::util::project_dir_of`]). Files mtrack keeps for itself are not
//! part of the project: anything whose name, or the name of a directory above
//! it, starts with a dot (the song cache, a rig's own state), and what an
//! interrupted write leaves beside a file ([`crate::util::is_write_leftover`]).

use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One file of a project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// The SHA-256 of the file's bytes, in lowercase hex.
    pub sha256: String,
    /// The file's length in bytes.
    pub size: u64,
}

/// Every file of a project, by its path from the project's directory:
/// components joined by `/` whatever the platform, in byte order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub files: BTreeMap<String, FileEntry>,
}

/// Why a path cannot be a project file's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathProblem {
    /// Empty, or with an empty component (`a//b`, a leading or trailing `/`).
    Empty,
    /// Absolute, or with a `.` or `..` component.
    Escapes,
    /// With a component mtrack keeps for itself: one starting with a dot, or
    /// a write's leftover.
    Reserved,
    /// With a character no file name here has: a backslash, a colon, or a
    /// control character.
    Character,
}

impl fmt::Display for PathProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PathProblem::Empty => "has an empty name in it",
            PathProblem::Escapes => "would leave the project's directory",
            PathProblem::Reserved => "is a name mtrack keeps for itself",
            PathProblem::Character => "has a character a file name here cannot",
        })
    }
}

/// Why a manifest cannot be a project's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestProblem {
    /// A path that cannot be a project file's.
    Path(String, PathProblem),
    /// A hash that is not 64 lowercase hex digits.
    Hash(String),
    /// Two paths that differ only by case: one file on a file system that
    /// does not tell them apart.
    CaseClash(String, String),
    /// A path that is both a file and a directory of other files.
    FileAndDirectory(String),
}

impl fmt::Display for ManifestProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestProblem::Path(path, problem) => write!(f, "{path:?} {problem}"),
            ManifestProblem::Hash(path) => write!(f, "{path:?} has no SHA-256"),
            ManifestProblem::CaseClash(a, b) => {
                write!(f, "{a:?} and {b:?} differ only by case")
            }
            ManifestProblem::FileAndDirectory(path) => {
                write!(f, "{path:?} is both a file and a directory")
            }
        }
    }
}

impl std::error::Error for ManifestProblem {}

/// Why a directory's manifest could not be made.
#[derive(Debug)]
pub enum ScanError {
    /// A file or directory could not be read.
    Io(PathBuf, std::io::Error),
    /// Something that is neither a file nor a directory, or a symbolic link:
    /// a project is the files in its directory, and a link's target is not.
    NotAFile(PathBuf),
    /// A name that is not UTF-8, or that [`valid_path`] refuses.
    Name(PathBuf),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScanError::Io(path, e) => write!(f, "{}: {e}", path.display()),
            ScanError::NotAFile(path) => write!(
                f,
                "{} is a link or a special file: a project is the files in its own directory",
                path.display()
            ),
            ScanError::Name(path) => write!(
                f,
                "{} has a name a project's file cannot have",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ScanError {}

/// Whether a name is one mtrack keeps for itself in a project's directory.
fn reserved(name: &str) -> bool {
    name.starts_with('.') || crate::util::is_write_leftover(Path::new(name))
}

/// Checks `path` as a project file's: relative, of plain names joined by `/`,
/// none of them mtrack's own. The one rule for a path read from a directory
/// and for one a manifest from elsewhere names, so a manifest cannot write
/// outside the directory it is applied to.
pub fn valid_path(path: &str) -> Result<(), PathProblem> {
    if path.is_empty() {
        return Err(PathProblem::Empty);
    }
    for component in path.split('/') {
        if component.is_empty() {
            return Err(PathProblem::Empty);
        }
        if component == "." || component == ".." {
            return Err(PathProblem::Escapes);
        }
        if component
            .chars()
            .any(|c| c == '\\' || c == ':' || c.is_control())
        {
            return Err(PathProblem::Character);
        }
        if reserved(component) {
            return Err(PathProblem::Reserved);
        }
    }
    Ok(())
}

/// The SHA-256 (lowercase hex) and length of the file at `path`.
pub fn hash_file(path: &Path) -> std::io::Result<FileEntry> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    let mut size = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok(FileEntry {
        sha256: hex(&hasher.finalize()),
        size,
    })
}

/// Bytes as lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    use fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

impl Manifest {
    /// The manifest of the project in `dir`: every file under it but
    /// mtrack's own (the module's rule). A link, a special file or a name
    /// that is not a plain UTF-8 one is an error, not something passed over:
    /// a copy made from the manifest would silently lack it.
    pub fn of_dir(dir: &Path) -> Result<Manifest, ScanError> {
        let mut files = BTreeMap::new();
        walk(dir, "", &mut |relative, path| {
            let hashed = hash_file(path).map_err(|e| ScanError::Io(path.to_path_buf(), e))?;
            files.insert(relative, hashed);
            Ok(())
        })?;
        Ok(Manifest { files })
    }

    /// Checks a manifest that came from elsewhere before anything is done
    /// with it: every path a project file's ([`valid_path`]), every hash a
    /// SHA-256, no two paths one file where case is not told apart, and no
    /// path both a file and a directory.
    pub fn validate(&self) -> Result<(), ManifestProblem> {
        let mut folded: BTreeMap<String, &str> = BTreeMap::new();
        for (path, entry) in &self.files {
            valid_path(path).map_err(|p| ManifestProblem::Path(path.clone(), p))?;
            if entry.sha256.len() != 64
                || !entry
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(ManifestProblem::Hash(path.clone()));
            }
            // Every directory a file is in, and the file, by folded name.
            let mut at = 0;
            loop {
                let end = path[at..].find('/').map(|i| at + i);
                let prefix = &path[..end.unwrap_or(path.len())];
                let key = prefix.to_lowercase();
                match folded.get(&key) {
                    Some(seen) if *seen != prefix => {
                        return Err(ManifestProblem::CaseClash(
                            (*seen).to_string(),
                            prefix.to_string(),
                        ));
                    }
                    Some(_) => {}
                    None => {
                        folded.insert(key, prefix);
                    }
                }
                match end {
                    Some(end) => {
                        if self.files.contains_key(prefix) {
                            return Err(ManifestProblem::FileAndDirectory(prefix.to_string()));
                        }
                        at = end + 1;
                    }
                    None => break,
                }
            }
        }
        Ok(())
    }

    /// One hash for the whole manifest: the SHA-256 of its paths, hashes and
    /// sizes in order. Two manifests are equal exactly when theirs are.
    pub fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        for (path, entry) in &self.files {
            hasher.update(path.as_bytes());
            hasher.update([0]);
            hasher.update(entry.sha256.as_bytes());
            hasher.update([0]);
            hasher.update(entry.size.to_be_bytes());
        }
        hex(&hasher.finalize())
    }

    /// The total length of the files, in bytes.
    pub fn size(&self) -> u64 {
        self.files.values().map(|entry| entry.size).sum()
    }
}

/// Visits every file of the project in `dir`, by the module's rule, with
/// its project path.
fn walk(
    dir: &Path,
    prefix: &str,
    visit: &mut dyn FnMut(String, &Path) -> Result<(), ScanError>,
) -> Result<(), ScanError> {
    let entries = std::fs::read_dir(dir).map_err(|e| ScanError::Io(dir.to_path_buf(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| ScanError::Io(dir.to_path_buf(), e))?;
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            return Err(ScanError::Name(path));
        };
        if reserved(&name) {
            continue;
        }
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if valid_path(&relative).is_err() {
            return Err(ScanError::Name(path));
        }
        // The entry's own type, not what a link points at.
        let kind = entry
            .file_type()
            .map_err(|e| ScanError::Io(path.clone(), e))?;
        if kind.is_dir() {
            walk(&path, &relative, visit)?;
        } else if kind.is_file() {
            visit(relative, &path)?;
        } else {
            return Err(ScanError::NotAFile(path));
        }
    }
    Ok(())
}

/// Every file of the project in `dir` with its length and when it was last
/// written: what [`Manifest::of_dir`] would read, without reading it. Taken
/// before a manifest and again later, two that are equal say no file was
/// added, removed or written in between.
pub fn stamps(
    dir: &Path,
) -> Result<BTreeMap<String, (u64, Option<std::time::SystemTime>)>, ScanError> {
    let mut found = BTreeMap::new();
    walk(dir, "", &mut |relative, path| {
        let metadata = std::fs::metadata(path).map_err(|e| ScanError::Io(path.to_path_buf(), e))?;
        found.insert(relative, (metadata.len(), metadata.modified().ok()));
        Ok(())
    })?;
    Ok(found)
}

/// What loading a project found: what the player would say on starting with
/// it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// The songs that loaded.
    pub songs: usize,
    /// The playlists that loaded.
    pub playlists: usize,
    /// What did not load, each as the file and why. The player starts
    /// without these; they are not a reason it cannot play.
    pub problems: Vec<(PathBuf, String)>,
    /// Files the songs that loaded are made of that are not the project's
    /// ([`Manifest`]'s rule): outside its directory, or under a name mtrack
    /// keeps for itself. A copy of the project made from its manifest would
    /// lack them.
    pub outside: Vec<PathBuf>,
}

/// Whether `file` is one of the files of the project in `project`, both
/// resolved: inside it, by a path a manifest can name.
fn in_project(project: &Path, file: &Path) -> bool {
    let file = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    file.strip_prefix(project).is_ok_and(|relative| {
        relative.to_str().is_some_and(|relative| {
            valid_path(&relative.replace(std::path::MAIN_SEPARATOR, "/")).is_ok()
        })
    })
}

/// Loads the project whose player config is at `config_path` as the player
/// does at startup, without any hardware: the config and its profiles, every
/// song, every playlist. An error is a project the player cannot start with
/// at all (its config does not read, its songs cannot be listed); what it
/// starts without is in the report.
pub fn check(config_path: &Path) -> Result<Report, Box<dyn std::error::Error>> {
    let player = crate::config::Player::deserialize(config_path)?;
    let layers = player.layer_set();
    let default_metronome = player.metronome().is_some_and(|m| m.enabled);
    let songs = crate::songs::get_all_songs_with_defaults(
        &player.songs(config_path),
        default_metronome,
        &layers,
    )?;
    let mut report = Report {
        songs: songs.len(),
        ..Report::default()
    };
    let project = crate::util::project_dir_of(config_path);
    let project = project.canonicalize().unwrap_or(project);
    for song in songs.sorted_list() {
        let files = song
            .config_path()
            .into_iter()
            .chain(song.tracks().iter().map(|track| track.file()))
            .chain(song.midi_playback().map(|midi| midi.file_path()));
        for file in files {
            if !in_project(&project, file) && !report.outside.iter().any(|seen| seen == file) {
                report.outside.push(file.to_path_buf());
            }
        }
    }
    for failure in songs.failures() {
        report.problems.push((
            failure.config_path().to_path_buf(),
            failure.error().to_string(),
        ));
    }
    if let Some(dir) = player.playlists_dir(config_path).filter(|dir| dir.is_dir()) {
        for file in crate::playlist::playlist_files(&dir)? {
            let loaded = crate::config::Playlist::deserialize(&file.path)
                .map_err(|e| e.to_string())
                .and_then(|config| {
                    crate::playlist::Playlist::new(&file.name, &config, songs.clone())
                        .map_err(|e| e.to_string())
                });
            match loaded {
                Ok(_) => report.playlists += 1,
                Err(e) => report.problems.push((file.path, e)),
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, path: &str, contents: &str) {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn a_manifest_lists_every_file_by_slash_path_with_its_hash() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "mtrack.yaml", "songs: songs\n");
        write(dir.path(), "songs/one/song.yaml", "name: One\n");
        let manifest = Manifest::of_dir(dir.path()).unwrap();
        let paths: Vec<&str> = manifest.files.keys().map(String::as_str).collect();
        assert_eq!(paths, ["mtrack.yaml", "songs/one/song.yaml"]);
        let entry = &manifest.files["mtrack.yaml"];
        assert_eq!(entry.size, 13);
        // sha256("songs: songs\n")
        assert_eq!(entry.sha256, hex(&Sha256::digest(b"songs: songs\n")),);
        assert_eq!(manifest.size(), 13 + 10);
    }

    #[test]
    fn mtracks_own_files_are_not_the_projects() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "mtrack.yaml", "a");
        write(dir.path(), ".mtrack-hub/rig.json", "{}");
        write(dir.path(), "songs/one/.mtrack-cache.json", "{}");
        write(dir.path(), "songs/one/song.yaml.123.mtrack-new", "x");
        write(dir.path(), "songs/one/song.yaml.mtrack-original", "x");
        let manifest = Manifest::of_dir(dir.path()).unwrap();
        assert_eq!(manifest.files.keys().collect::<Vec<_>>(), ["mtrack.yaml"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_an_error_not_a_file_left_out() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "mtrack.yaml", "a");
        std::os::unix::fs::symlink(dir.path().join("mtrack.yaml"), dir.path().join("link.yaml"))
            .unwrap();
        assert!(matches!(
            Manifest::of_dir(dir.path()),
            Err(ScanError::NotAFile(_))
        ));
    }

    #[test]
    fn the_digest_moves_with_any_file() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.yaml", "one");
        let before = Manifest::of_dir(dir.path()).unwrap();
        assert_eq!(
            before.digest(),
            Manifest::of_dir(dir.path()).unwrap().digest()
        );
        write(dir.path(), "a.yaml", "two");
        let after = Manifest::of_dir(dir.path()).unwrap();
        assert_ne!(before.digest(), after.digest());
        assert_ne!(before.digest(), Manifest::default().digest());
    }

    #[test]
    fn a_path_must_stay_inside_and_be_plain() {
        assert_eq!(valid_path("songs/one/song.yaml"), Ok(()));
        assert_eq!(valid_path("v1..2/a"), Ok(()));
        assert_eq!(valid_path(""), Err(PathProblem::Empty));
        assert_eq!(valid_path("/etc/passwd"), Err(PathProblem::Empty));
        assert_eq!(valid_path("songs//a"), Err(PathProblem::Empty));
        assert_eq!(valid_path("songs/"), Err(PathProblem::Empty));
        assert_eq!(valid_path("../a"), Err(PathProblem::Escapes));
        assert_eq!(valid_path("songs/../../a"), Err(PathProblem::Escapes));
        assert_eq!(valid_path("./a"), Err(PathProblem::Escapes));
        assert_eq!(valid_path("songs\\..\\a"), Err(PathProblem::Character));
        assert_eq!(valid_path("C:/a"), Err(PathProblem::Character));
        assert_eq!(valid_path("a\nb"), Err(PathProblem::Character));
        assert_eq!(
            valid_path(".mtrack-hub/rig.json"),
            Err(PathProblem::Reserved)
        );
        assert_eq!(valid_path("songs/.hidden"), Err(PathProblem::Reserved));
        assert_eq!(
            valid_path("songs/a.yaml.mtrack-original"),
            Err(PathProblem::Reserved)
        );
    }

    fn manifest(paths: &[&str]) -> Manifest {
        Manifest {
            files: paths
                .iter()
                .map(|path| {
                    (
                        path.to_string(),
                        FileEntry {
                            sha256: "0".repeat(64),
                            size: 0,
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn a_manifest_from_elsewhere_is_checked_whole() {
        assert_eq!(manifest(&["a.yaml", "songs/b/c.wav"]).validate(), Ok(()));
        assert!(matches!(
            manifest(&["../a"]).validate(),
            Err(ManifestProblem::Path(_, PathProblem::Escapes))
        ));
        assert!(matches!(
            manifest(&["Songs/a", "songs/b"]).validate(),
            Err(ManifestProblem::CaseClash(_, _))
        ));
        assert!(matches!(
            manifest(&["a.WAV", "a.wav"]).validate(),
            Err(ManifestProblem::CaseClash(_, _))
        ));
        assert!(matches!(
            manifest(&["songs", "songs/a"]).validate(),
            Err(ManifestProblem::FileAndDirectory(_))
        ));
        let mut bad_hash = manifest(&["a"]);
        bad_hash.files.get_mut("a").unwrap().sha256 = "ABC".to_string();
        assert_eq!(
            bad_hash.validate(),
            Err(ManifestProblem::Hash("a".to_string()))
        );
    }

    #[test]
    fn the_example_project_loads_without_hardware() {
        let report = check(Path::new("../../examples/mtrack.yaml")).unwrap();
        assert!(report.songs > 0, "{report:?}");
        assert!(report.playlists > 0, "{report:?}");
    }

    #[test]
    fn stamps_move_when_a_file_is_written_added_or_removed() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.yaml", "one");
        write(dir.path(), ".hidden", "x");
        let before = stamps(dir.path()).unwrap();
        assert_eq!(before.keys().collect::<Vec<_>>(), ["a.yaml"]);
        assert_eq!(before, stamps(dir.path()).unwrap());
        write(dir.path(), "a.yaml", "longer");
        assert_ne!(before, stamps(dir.path()).unwrap());
        write(dir.path(), "a.yaml", "one");
        write(dir.path(), "b.yaml", "two");
        assert_ne!(before.len(), stamps(dir.path()).unwrap().len());
    }

    #[test]
    fn a_songs_file_outside_the_project_is_reported() {
        let elsewhere = tempfile::tempdir().unwrap();
        crate::testutil::write_wav(
            elsewhere.path().join("click.wav"),
            vec![vec![1_i32; 4410]],
            44100,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "mtrack.yaml", "songs: songs\n");
        write(
            dir.path(),
            "songs/one/song.yaml",
            &format!(
                "kind: song\nname: One\ntracks:\n  - name: click\n    file: {}\n",
                elsewhere.path().join("click.wav").display()
            ),
        );
        let report = check(&dir.path().join("mtrack.yaml")).unwrap();
        assert_eq!(report.songs, 1, "{report:?}");
        assert_eq!(report.outside.len(), 1, "{report:?}");
        assert!(report.outside[0].ends_with("click.wav"));

        // The example project's songs are all its own.
        let example = check(Path::new("../../examples/mtrack.yaml")).unwrap();
        assert!(example.outside.is_empty(), "{example:?}");
    }

    #[test]
    fn a_config_that_does_not_read_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "mtrack.yaml", "songs: [not, a, path]\n");
        assert!(check(&dir.path().join("mtrack.yaml")).is_err());
    }
}
