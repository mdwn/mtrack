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

use std::path::Path;
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

/// Applies to `replacement` the ownership and mode that `dest` should have once
/// `replacement` is renamed over it: those of the existing file at `dest`, or —
/// when `dest` does not exist yet — those implied by its parent directory.
///
/// mtrack rewrites its own configuration in place — profiles, playlists, song
/// YAML, light shows, the song cache — by writing a temp file and renaming it
/// over the original. The rename swaps in a new inode, so without this the file
/// on disk ends up with the temp file's ownership and mode (the invoking user,
/// mode 0600) rather than the ones it had a moment earlier. Run mtrack once
/// under `sudo` to debug something and every file it saved is left `root:root`
/// and mode 0600, locking the normal user out of their own configuration.
///
/// Call this after writing the temp file and before renaming it into place.
pub fn preserve_ownership(replacement: &std::fs::File, dest: &Path) -> std::io::Result<()> {
    match Ownership::of(dest) {
        Ok(existing) => existing.apply(replacement, dest),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match parent_of(dest) {
            Some(parent) => Ownership::of(parent)?
                .for_file_in_dir()
                .apply(replacement, dest),
            // No parent to inherit from; the process defaults are all we have.
            None => Ok(()),
        },
        Err(e) => Err(e),
    }
}

/// Like [`std::fs::create_dir_all`], but every directory it creates adopts the
/// ownership and mode of the nearest ancestor that already existed, rather than
/// of the invoking user — for the same reason as [`preserve_ownership`]. A path
/// that already exists is left untouched.
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

/// [`create_dir_all`] for callers on a Tokio runtime, which keeps the
/// filesystem work off the async worker.
pub async fn create_dir_all_async(path: &Path) -> std::io::Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || create_dir_all(&path))
        .await
        .map_err(std::io::Error::other)?
}

/// Writes `contents` to `path`, creating it if needed.
///
/// An existing file keeps its ownership and mode — this truncates in place
/// rather than replacing the inode — and a newly created one inherits them from
/// its directory, again for the reason described on [`preserve_ownership`].
pub fn write_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let is_new = !path.try_exists()?;
    let mut file = std::fs::File::create(path)?;
    if is_new {
        if let Some(parent) = parent_of(path) {
            Ownership::of(parent)?
                .for_file_in_dir()
                .apply(&file, path)?;
        }
    }
    file.write_all(contents)
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

        file.set_permissions(std::fs::Permissions::from_mode(self.mode))?;

        // Skip the syscall in the overwhelmingly common case of writing a file
        // we already own — chown only matters when the ids actually differ.
        let current = file.metadata()?;
        if current.uid() == self.uid && current.gid() == self.gid {
            return Ok(());
        }

        // Only a privileged process can give a file away, so failure here is
        // expected whenever a user edits a file owned by someone else, and is
        // not something they can act on. Log it and keep the write.
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
        Ok(())
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
    fn preserve_ownership_keeps_the_mode_of_the_replaced_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("config.yaml");
        std::fs::write(&dest, "old").unwrap();
        set_mode(&dest, 0o640);

        let tmp = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        preserve_ownership(tmp.as_file(), &dest).unwrap();
        tmp.persist(&dest).unwrap();

        assert_eq!(mode_of(&dest), 0o640);
    }

    #[test]
    fn preserve_ownership_keeps_the_owner_of_the_replaced_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("config.yaml");
        std::fs::write(&dest, "old").unwrap();
        let before = std::fs::metadata(&dest).unwrap();

        let tmp = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        preserve_ownership(tmp.as_file(), &dest).unwrap();
        tmp.persist(&dest).unwrap();

        // The suite runs unprivileged, so this can only assert that a same-user
        // write leaves ownership alone. The cross-user case is the one `apply`
        // logs about, and exercising it needs root.
        let after = std::fs::metadata(&dest).unwrap();
        assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
    }

    #[test]
    fn preserve_ownership_derives_a_new_files_mode_from_its_directory() {
        for (dir_mode, want) in [(0o755, 0o644), (0o700, 0o600), (0o775, 0o664)] {
            let dir = tempfile::tempdir().unwrap();
            set_mode(dir.path(), dir_mode);
            let dest = dir.path().join("new.yaml");

            let tmp = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
            preserve_ownership(tmp.as_file(), &dest).unwrap();
            tmp.persist(&dest).unwrap();

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
    fn write_file_derives_a_new_files_mode_from_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        set_mode(dir.path(), 0o750);
        let path = dir.path().join("new.yaml");

        write_file(&path, b"contents").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "contents");
        assert_eq!(mode_of(&path), 0o640);
        set_mode(dir.path(), 0o755);
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
