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

//! Browsing the project folder and making songs of the folders in it: what
//! `ConfigService`'s browse, create-in-folder and bulk import RPCs call.
//! Plain functions over the player and the config file's path (no web
//! types); a refusal is a [`SongError`].

use super::super::safe_path::{SafePath, VerifiedRoot};
use super::songs_api::{blocking_io, reload_songs, songs_dir, SongError};
use crate::player::Player;
use crate::songs;
use crate::webui::wire::songs::{Browse, BrowseEntry, BulkImport, FileKind, ImportFailure};

/// Resolves the project root (parent of `config_path`) as a VerifiedRoot.
fn project_root(config_path: &std::path::Path) -> Result<VerifiedRoot, SongError> {
    let config_canonical = config_path
        .canonicalize()
        .map_err(|e| SongError::Internal(format!("Failed to resolve config path: {e}")))?;
    let parent = config_canonical.parent().ok_or_else(|| {
        SongError::Internal("Unable to determine config root directory".to_string())
    })?;
    Ok(VerifiedRoot::new(parent)?)
}

/// Resolves a project-relative path (e.g. "/songs") under the project root.
fn resolve_browse_path(root: &VerifiedRoot, path: &str) -> Result<SafePath, SongError> {
    if path.is_empty() || path == "/" {
        return Ok(root.as_safe_path());
    }
    let relative = path.strip_prefix('/').unwrap_or(path);
    SafePath::resolve(&root.as_path().join(relative), root)
        .map_err(|_| SongError::NotFound(format!("Not a directory: {path}")))
}

/// The directory at the project-relative `path`, as a path under the
/// project root `config_path` is in, refused unless it is one.
fn project_dir(
    config_path: &std::path::Path,
    path: &str,
    not_a_dir: impl FnOnce() -> SongError,
) -> Result<(VerifiedRoot, SafePath), SongError> {
    let root = project_root(config_path)?;
    let dir = resolve_browse_path(&root, path)?;
    if !dir.is_dir() {
        return Err(not_a_dir());
    }
    Ok((root, dir))
}

/// Lists files and directories at a project-relative path.
///
/// Restricted to the directory containing mtrack.yaml (the config root).
/// Empty `path` is the config root. Entries are sorted: directories first,
/// then files alphabetically.
pub(crate) fn browse_directory(
    config_path: &std::path::Path,
    path: &str,
) -> Result<Browse, SongError> {
    let (root, dir_safe) = project_dir(config_path, path, || {
        SongError::NotFound(format!("Not a directory: {path}"))
    })?;

    let root_canonical = root.as_path().to_path_buf();
    let dir_canonical = dir_safe.as_path().to_path_buf();

    // Convert an absolute path to a project-relative path (e.g., "/songs/foo").
    let to_relative = |abs: &std::path::Path| -> String {
        let suffix = abs
            .strip_prefix(&root_canonical)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if suffix.is_empty() {
            "/".to_string()
        } else {
            format!("/{suffix}")
        }
    };

    let mut entries: Vec<BrowseEntry> = Vec::new();
    let read = std::fs::read_dir(&dir_canonical)
        .map_err(|e| SongError::Internal(format!("Failed to read directory: {e}")))?;
    for entry in read.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Skip hidden files
        if songs::is_hidden(&path) {
            continue;
        }
        let is_dir = path.is_dir();
        entries.push(BrowseEntry {
            kind: if is_dir {
                FileKind::Directory
            } else {
                FileKind::of(&path)
            },
            path: to_relative(&path),
            name,
            is_dir,
        });
    }

    // Sort: directories first, then alphabetically by name
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(Browse {
        path: to_relative(&dir_canonical),
        root: root_canonical.to_string_lossy().into_owned(),
        entries,
    })
}

/// A song made of a folder.
pub(crate) struct CreatedSong {
    /// The song's config file.
    pub(crate) path: String,
    /// The song's name.
    pub(crate) song: String,
}

/// Auto-generates a song.yaml in the project-relative directory `path`, and
/// reloads the songs.
///
/// The backend scans the directory for audio/MIDI/lighting files and
/// generates the song config automatically, including per-channel track
/// splitting for stereo and multichannel audio files. `name` overrides the
/// song's name.
pub(crate) async fn create_song_in_directory(
    player: &Player,
    config_path: &std::path::Path,
    path: &str,
    name: Option<&str>,
) -> Result<CreatedSong, SongError> {
    let (_, dir_safe) = project_dir(config_path, path, || {
        SongError::Invalid("Path is not a directory".to_string())
    })?;
    let dir_canonical = dir_safe.as_path().to_path_buf();

    let Some(config_path) = songs::song_config_to_write(&dir_canonical) else {
        return Err(SongError::Exists(
            "a song config already exists in this directory".to_string(),
        ));
    };

    // The song's name, by the one rule for a new song's (#674): one given
    // must not be taken by a song loaded, failed or configured under the
    // songs root (refused, naming it); with none, the folder's name, else its
    // path below the songs root, else the first free suffix. Off the async
    // worker: the configured names are a walk of the songs tree.
    let taken = super::helpers::player_song_names(player);
    let songs_root = songs_dir(player);
    let asked = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string);
    let naming = blocking_io("read the songs folder", {
        let dir = dir_canonical.clone();
        let config_path = config_path.clone();
        move || -> std::io::Result<Result<String, songs::SongNameTaken>> {
            match asked {
                Some(name) => {
                    Ok(
                        songs::check_song_name_free(&config_path, &name, &songs_root, taken)
                            .map(|()| name),
                    )
                }
                None => Ok(Ok(songs::new_song(&songs_root, &dir, taken)?.map_or_else(
                    || crate::util::filename_display(&dir).to_string(),
                    |song| song.name,
                ))),
            }
        }
    })
    .await?;
    let name = naming.map_err(SongError::Taken)?;

    // Use Song::initialize to scan the directory and build the config with proper
    // channel-aware track splitting (stereo -> L/R, multichannel -> per-channel).
    // Same blocking decode/analysis as the bulk path, for one song.
    let scan = tokio::task::spawn_blocking({
        let dir = dir_canonical.clone();
        move || {
            songs::Song::initialize(&dir)
                .map(|song| song.get_config())
                .map_err(|e| e.to_string())
        }
    })
    .await;
    let mut song_config = match scan {
        Ok(Ok(config)) => config,
        Ok(Err(e)) => {
            return Err(SongError::Invalid(format!("Failed to scan directory: {e}")));
        }
        Err(e) => {
            return Err(SongError::Internal(format!("Directory scan failed: {e}")));
        }
    };

    song_config.set_name(&name);

    // Flattened before the reload's await: the save's error is not Send.
    song_config
        .save(&config_path)
        .map_err(|e| SongError::Internal(format!("Failed to save song config: {e}")))?;
    // Refresh the player's all-songs playlist so newly imported songs appear.
    reload_songs(player).await?;
    Ok(CreatedSong {
        path: config_path.to_string_lossy().into_owned(),
        song: name,
    })
}

/// Creates a song.yaml in each song folder under the project-relative
/// directory `path` ([`import_song_dirs`]), and reloads the songs once if
/// any was made. Answers a summary of created, skipped, and failed
/// directories.
pub(crate) async fn bulk_import(
    player: &Player,
    config_path: &std::path::Path,
    path: &str,
) -> Result<BulkImport, SongError> {
    let (_, dir_safe) = project_dir(config_path, path, || {
        SongError::Invalid("Path is not a directory".to_string())
    })?;
    let dir_canonical = dir_safe.as_path().to_path_buf();

    // `Song::initialize` decodes and analyses each candidate's click track, so
    // a first import of a large library is minutes of blocking CPU and file
    // I/O. Off the async worker, or it parks a runtime thread for the duration.
    // codeql[rust/path-injection] dir_canonical is canonicalized and
    // verified via starts_with(root_canonical) above, preventing traversal.
    // The songs the player has loaded, or failed to, under the songs root:
    // no new song is named as one of them.
    let taken = super::helpers::player_song_names(player);
    let songs_root = songs_dir(player);
    let scan =
        tokio::task::spawn_blocking(move || import_song_dirs(&dir_canonical, &songs_root, taken))
            .await;

    let (created, skipped, failed) = match scan {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            return Err(SongError::Internal(format!(
                "Failed to read directory: {e}"
            )));
        }
        Err(e) => {
            return Err(SongError::Internal(format!("Import scan failed: {e}")));
        }
    };

    // Refresh the player's song state once after all imports
    if !created.is_empty() {
        reload_songs(player).await?;
    }

    Ok(BulkImport {
        created,
        skipped,
        failed,
    })
}

/// Imports each song folder under `root` that has no config to keep
/// ([`songs::new_songs`]), named past the songs in `taken` and those
/// configured under `songs_root` too; one with a config is skipped. Each is
/// reported by its path below `root`. An unreadable `root` is an error.
fn import_song_dirs(
    root: &std::path::Path,
    songs_root: &std::path::Path,
    taken: std::collections::HashSet<String>,
) -> std::io::Result<(Vec<String>, Vec<String>, Vec<ImportFailure>)> {
    let found = songs::new_songs(root, songs::taken_song_names(songs_root, taken))?;
    let display_name = |dir: &std::path::Path| {
        dir.strip_prefix(root)
            .unwrap_or(dir)
            .to_string_lossy()
            .to_string()
    };
    let skipped = found.kept.iter().map(|dir| display_name(dir)).collect();
    let mut created: Vec<String> = Vec::new();
    let mut failed: Vec<ImportFailure> = Vec::new();
    for song in &found.new {
        match song.write() {
            Ok(()) => created.push(display_name(&song.dir)),
            Err(e) => failed.push(ImportFailure {
                name: display_name(&song.dir),
                error: format!("{}", e),
            }),
        }
    }
    Ok((created, skipped, failed))
}
