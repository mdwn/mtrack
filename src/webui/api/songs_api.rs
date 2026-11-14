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

//! A song's files: its config read, created, saved and deleted, its
//! waveform, tempo guess and file list, and a file imported into it. Plain
//! functions over the player and the songs directory (no web types), which
//! `ConfigService`'s song RPCs and the MCP server call; a refusal is a
//! [`SongError`]. The track upload routes, which carry file bytes and so stay
//! HTTP, are the only handlers here.

use axum::{http::StatusCode, response::IntoResponse};
use serde_json::json;

use super::super::config_io;
use super::super::safe_path::{SafePath, SafePathError, VerifiedRoot};
use super::super::state as ws_state;
use super::lighting_writes::Answer;
use crate::player::Player;
use crate::util::FaultKind;
use crate::webui::wire::songs::{FileKind, SongFile, TempoGuess, TempoGuessSource};
use crate::{config, songs};

/// Why a song call was refused or failed. [`SongError::status`] and
/// [`SongError::body`] are the answer the web API gave each, which MCP gives
/// still; `ConfigService` maps each to a status code.
#[derive(Debug)]
pub(crate) enum SongError {
    /// The request is wrong: a name or a path that is not safe, a file of a
    /// type the player does not play, a song that did not load.
    Invalid(String),
    /// A song config that is not valid: every problem with it, in order.
    InvalidConfig(Vec<String>),
    /// What was named is not there.
    NotFound(String),
    /// A path that leaves the directory it must stay in.
    Forbidden(String),
    /// A create reached what is already there.
    Exists(String),
    /// A song name another song holds.
    Taken(songs::SongNameTaken),
    /// The player or the files are not in the state the call needs: a song
    /// that is playing, a `song.yaml` that is another song's.
    Precondition(String),
    /// The song has what the call needs, but too little of it.
    Unprocessable(String),
    /// mtrack's own fault, or the filesystem's.
    Internal(String),
}

impl SongError {
    /// The error of a failed song lookup, by the one table from its kind.
    pub(crate) fn of_lookup(e: songs::SongLookupError) -> Self {
        let message = e.to_string();
        match e.kind() {
            FaultKind::NotFound => Self::NotFound(message),
            FaultKind::Precondition => Self::Precondition(message),
            FaultKind::Invalid => Self::Invalid(message),
            FaultKind::Internal => Self::Internal(message),
        }
    }

    /// The HTTP status the web API answered this with.
    pub(crate) fn status(&self) -> StatusCode {
        match self {
            Self::Invalid(_) | Self::InvalidConfig(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::Exists(_) | Self::Taken(_) | Self::Precondition(_) => StatusCode::CONFLICT,
            Self::Unprocessable(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// What the call says is wrong; a config's problems one after another.
    pub(crate) fn message(&self) -> String {
        match self {
            Self::InvalidConfig(errors) => errors.join("; "),
            Self::Taken(taken) => taken.to_string(),
            Self::Invalid(m)
            | Self::NotFound(m)
            | Self::Forbidden(m)
            | Self::Exists(m)
            | Self::Precondition(m)
            | Self::Unprocessable(m)
            | Self::Internal(m) => m.clone(),
        }
    }

    /// The JSON body the web API answered this with.
    pub(crate) fn body(&self) -> serde_json::Value {
        match self {
            Self::InvalidConfig(errors) => json!({ "errors": errors }),
            Self::Taken(taken) => json!({"error": taken.to_string(), "taken": taken.0}),
            other => json!({"error": other.message()}),
        }
    }
}

impl std::fmt::Display for SongError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl From<SafePathError> for SongError {
    fn from(e: SafePathError) -> Self {
        let message = e.to_string();
        match e {
            SafePathError::NotFound => Self::NotFound(message),
            SafePathError::OutsideRoot => Self::Forbidden(message),
            SafePathError::InvalidName => Self::Invalid(message),
            SafePathError::RootNotFound | SafePathError::Io(_) => Self::Internal(message),
        }
    }
}

/// An answer for a surface that is not gRPC: MCP's, and the upload routes'.
impl From<SongError> for Answer {
    fn from(e: SongError) -> Self {
        Answer {
            status: e.status(),
            body: e.body(),
        }
    }
}

impl From<SongError> for axum::response::Response {
    fn from(e: SongError) -> Self {
        Answer::from(e).into_response()
    }
}

/// A blocking filesystem operation on the blocking pool; its failure, and
/// the task's, as [`SongError::Internal`].
pub(crate) async fn blocking_io<F, T, E>(label: &str, f: F) -> Result<T, SongError>
where
    F: FnOnce() -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(SongError::Internal(format!("Failed to {label}: {e}"))),
        Err(e) => Err(SongError::Internal(format!("{label} task failed: {e}"))),
    }
}

/// Reloads the player's songs and playlists after a change on disk. A
/// reload that fails leaves the player with the songs it had, so the change
/// is reported as an error saying so, rather than as made.
pub(crate) async fn reload_songs(player: &Player) -> Result<(), SongError> {
    player.reload_songs().await.map_err(|e| {
        SongError::Internal(format!(
            "the change was written, but the player could not reload its songs: {e}"
        ))
    })
}

/// The names of the songs the player has loaded, or failed to: no new song
/// is given one of them.
fn taken_names(player: &Player) -> std::collections::HashSet<String> {
    super::helpers::player_song_names(player)
}

/// A path canonicalized, else as it is (a directory not made yet).
fn canonical_or_kept(path: std::path::PathBuf) -> std::path::PathBuf {
    path.canonicalize().unwrap_or(path)
}

/// The songs directory, canonicalized, from the player's song sources: read
/// on each call, so a runtime change to `songs:` reaches the files edited as
/// it reaches the reload.
/// codeql[rust/path-injection] Resolved from the player config file; not user-controlled.
pub(crate) fn songs_dir(player: &Player) -> std::path::PathBuf {
    canonical_or_kept(player.song_sources().unwrap_or_default().songs)
}

/// A song's directory `dir` relative to the songs directory `root`, "/"
/// between its parts: the song list's `base_dir`, which the web UI groups
/// songs by and the gRPC library sends as it. Empty when `dir` is not under
/// `root`.
pub(crate) fn base_dir(root: &std::path::Path, dir: &std::path::Path) -> String {
    dir.strip_prefix(root)
        .map(|relative| {
            relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default()
}

/// What a song's config read gives.
pub(crate) enum SongConfig {
    /// The config file, as written.
    File(String),
    /// The song is loaded but its config file is gone: what the player has.
    Gone(SongSummary),
}

/// A loaded song, for [`SongConfig::Gone`].
pub(crate) struct SongSummary {
    pub(crate) name: String,
    pub(crate) duration_ms: u64,
    pub(crate) duration_display: String,
    pub(crate) num_channels: u16,
    pub(crate) sample_format: String,
    pub(crate) tracks: Vec<String>,
}

/// Reads song `name`'s config: the file it was read from, loaded or not, by
/// the registry's one rule; for a loaded song with no file, a summary of it.
pub(crate) async fn read_song_config(player: &Player, name: &str) -> Result<SongConfig, SongError> {
    // Gathered from the song registry synchronously, so its Arc is dropped
    // before any await.
    let path = {
        let all_songs = player.songs();
        match all_songs.config_path(name).filter(|p| p.is_file()) {
            Some(path) => path,
            None => {
                return match all_songs.get(name) {
                    Ok(song) => Ok(SongConfig::Gone(SongSummary {
                        name: song.name().to_string(),
                        duration_ms: song.summary().duration.as_millis() as u64,
                        duration_display: crate::util::duration_minutes_seconds(
                            song.summary().duration,
                        ),
                        num_channels: song.num_channels(),
                        sample_format: format!("{}", song.sample_format()),
                        tracks: song.tracks().iter().map(|t| t.name().to_string()).collect(),
                    })),
                    Err(songs::SongLookupError::FailedToLoad { .. }) => Err(SongError::NotFound(
                        format!("No config file found for failed song: {name}"),
                    )),
                    Err(e) => Err(SongError::of_lookup(e)),
                };
            }
        }
    };
    blocking_io("read song config", move || {
        crate::util::read_to_string(&path)
    })
    .await
    .map(SongConfig::File)
}

/// Removes a song by deleting its song.yaml, and the song from the
/// playlist files that name it, then reloads the songs.
///
/// The song's audio, MIDI, and lighting files are left in place. Only the
/// song.yaml config file is removed, which unregisters the song from the
/// player. Refused while the song is playing.
pub(crate) async fn delete_song(player: &Player, name: &str) -> Result<(), SongError> {
    // Reject deletion if the song is currently playing.
    if player.is_playing().await {
        if let Some(current) = player.get_playlist().current() {
            if current.name() == name {
                return Err(SongError::Precondition(
                    "Cannot delete a song that is currently playing".to_string(),
                ));
            }
        }
    }

    // In turn with the song's edits, which would write a deleted song back,
    // and with the playlists' saves, whose files this edits too. Held from
    // the lookup, so the song found is the one on disk.
    let _song_writes = SONG_WRITES.lock().await;
    let _playlist_writes = super::playlists::PLAYLIST_WRITES.lock().await;

    // Resolve the song's YAML through the registry with path verification.
    let config_path = song_config_path(player, &songs_dir(player), name, None)?;
    if !config_path.exists() {
        return Err(SongError::NotFound(
            "song.yaml not found for this song".to_string(),
        ));
    }

    let sources = player.song_sources().unwrap_or_default();
    let playlists_dir = sources.playlists_dir.map(canonical_or_kept);
    let legacy_playlist = sources.legacy_playlist.map(canonical_or_kept);
    let song_name = name.to_string();
    blocking_io("delete song", move || {
        std::fs::remove_file(&config_path)?;
        // Remove the song from any playlist files on disk so they stay valid.
        remove_song_from_playlists(
            &song_name,
            playlists_dir.as_deref(),
            legacy_playlist.as_deref(),
        );
        Ok::<_, std::io::Error>(())
    })
    .await?;

    // Refresh the player's song state.
    reload_songs(player).await
}

/// A song's waveform peaks, track by track: from the cache when it has
/// them, else computed (and kept). The virtual tracks (metronome, pilot)
/// are rendered fresh either way: they follow the current config and are
/// cheap.
pub(crate) async fn song_waveform(
    player: &Player,
    cache: &ws_state::WaveformCache,
    name: &str,
) -> Result<Vec<ws_state::TrackPeaks>, SongError> {
    // Look up the song from the player's registry.
    let song = player.songs().get(name).map_err(SongError::of_lookup)?;

    let cached = cache.lock().get(name).cloned();
    if let Some(cached) = cached {
        let song_for_virtual = song.clone();
        let virtual_peaks = tokio::task::spawn_blocking(move || {
            ws_state::virtual_track_peaks(&song_for_virtual, 500)
        })
        .await
        .unwrap_or_default();
        return Ok(cached.into_iter().chain(virtual_peaks).collect());
    }

    let track_infos: Vec<ws_state::TrackInfo> = song
        .tracks()
        .iter()
        .map(|t| {
            (
                t.name().to_string(),
                t.file().to_path_buf(),
                t.file_channel(),
            )
        })
        .collect();
    let song_dir = song.base_path().to_path_buf();

    let cache = cache.clone();
    let song_name = name.to_string();
    let song_for_virtual = song.clone();
    tokio::task::spawn_blocking(move || {
        let peaks = ws_state::compute_waveform_peaks(&song_dir, &track_infos);
        cache.lock().insert(song_name, peaks.clone());
        let mut with_virtual = peaks;
        with_virtual.extend(ws_state::virtual_track_peaks(&song_for_virtual, 500));
        with_virtual
    })
    .await
    .map_err(|e| SongError::Internal(format!("Failed to compute waveform: {e}")))
}

/// Estimates a tempo map for a song.
///
/// Tries MIDI file first (authoritative tempo events), then falls back to
/// beat grid analysis (estimated from click track). The beat grid's start
/// offset is used in both cases. Not found if neither source is available.
pub(crate) fn song_tempo_guess(player: &Player, name: &str) -> Result<TempoGuess, SongError> {
    let song = player.songs().get(name).map_err(SongError::of_lookup)?;

    // Try MIDI file first — it has authoritative tempo/time-sig events.
    // Pass the beat grid for cross-correlation alignment.
    if let Some(midi_playback) = song.midi_playback() {
        if let Some(guessed) = crate::audio::midi_tempo::extract_tempo_from_midi(
            midi_playback.file_path(),
            song.beat_grid(),
        ) {
            return Ok(TempoGuess {
                source: TempoGuessSource::Midi,
                config: guessed.to_config(),
                tempo: guessed,
            });
        }
    }

    // Fall back to beat grid analysis
    let beat_grid = song
        .beat_grid()
        .ok_or_else(|| SongError::NotFound("Song has no MIDI file or beat grid".to_string()))?;

    // The grid's tempo map, as lighting derives it, is the suggestion.
    match beat_grid
        .to_tempo_map()
        .map(|map| crate::audio::midi_tempo::GuessedTempo::from_tempo_map(&map))
    {
        Some(guessed) => Ok(TempoGuess {
            source: TempoGuessSource::BeatGrid,
            config: guessed.to_config(),
            tempo: guessed,
        }),
        None => Err(SongError::Unprocessable(
            "Beat grid has too few beats to estimate tempo".to_string(),
        )),
    }
}

/// Lists the files in a song's directory, by name: audio, MIDI, and
/// lighting files with type classification.
/// Uses the same song lookup as other calls to resolve the correct base_path,
/// supporting songs in nested subdirectories.
pub(crate) async fn song_files(player: &Player, name: &str) -> Result<Vec<SongFile>, SongError> {
    let song = player.songs().get(name).map_err(SongError::of_lookup)?;
    let song_dir = song.base_path().to_path_buf();
    let named: Vec<std::path::PathBuf> = song.named_files().map(|p| p.to_path_buf()).collect();

    let mut files = blocking_io("read song dir", move || song_dir_files(&song_dir, &named)).await?;
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

/// The files in a song's directory, as [`song_files`] lists them:
/// each file directly in it but the song's config, and hidden files only
/// when the song's config names them (`named`, the files it names). In the
/// order the directory gives them; on the blocking pool.
pub(crate) fn song_dir_files(
    song_dir: &std::path::Path,
    named: &[std::path::PathBuf],
) -> std::io::Result<Vec<SongFile>> {
    // A hidden file the song's config names (`file: .click.wav`) is
    // the song's, and listed.
    let named: Vec<_> = named.iter().filter_map(|p| p.canonicalize().ok()).collect();
    let mut files: Vec<SongFile> = Vec::new();
    for entry in std::fs::read_dir(song_dir)?.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let filename = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Skip the song's config file, and hidden files it does not name
        if songs::is_song_config_name(&path)
            || (songs::is_hidden(&path) && !path.canonicalize().is_ok_and(|p| named.contains(&p)))
        {
            continue;
        }
        files.push(SongFile {
            kind: FileKind::of(&path),
            name: filename,
        });
    }
    Ok(files)
}

/// A file copied into a song.
pub(crate) struct ImportedFile {
    /// The file's name in the song.
    pub(crate) file: String,
    /// The name the song has: a new one's may not be the one asked for,
    /// when that is taken (#674).
    pub(crate) song: String,
}

/// Copies the file `source` from the server filesystem into song `name`'s
/// directory (made, with its config, when the song is new), then reloads
/// the songs. `config_path` is the player's config file: its directory is
/// the project root.
///
/// The file extension must be supported (audio, MIDI, or .light). The source
/// path must resolve to within the project root (the directory containing
/// mtrack.yaml) to prevent arbitrary file reads.
pub(crate) async fn import_file_to_song(
    player: &Player,
    config_path: &std::path::Path,
    name: &str,
    source: &str,
    filename: Option<&str>,
) -> Result<ImportedFile, SongError> {
    // Determine the project root (parent of mtrack.yaml).
    let config_canonical = config_path
        .canonicalize()
        .map_err(|e| SongError::Internal(format!("Failed to resolve config path: {e}")))?;
    let project_root = VerifiedRoot::new(
        config_canonical
            .parent()
            .ok_or_else(|| SongError::Internal("Unable to determine project root".to_string()))?,
    )?;

    // Resolve the source file path under the project root.
    let source = SafePath::resolve(std::path::Path::new(source), &project_root).map_err(|_| {
        SongError::Invalid("Source file does not exist or is outside the project directory".into())
    })?;
    if !source.is_file() {
        return Err(SongError::Invalid("Source path is not a file".to_string()));
    }

    let filename = filename
        .or_else(|| source.file_name().and_then(|n| n.to_str()))
        .unwrap_or("unknown");

    let songs_path = songs_dir(player);
    let exists = song_has_file(player, &songs_path, name, filename);
    validate_track_filename(filename, exists)?;

    let song_dir = resolve_or_create_song_dir(player, &songs_path, name)?;

    // codeql[rust/path-injection] dest_path is under song_dir (verified by resolve_or_create_song_dir),
    // filename is validated by validate_track_filename (no .. or / allowed).
    let dest_path = song_dir.join_filename(filename);
    let src = source.as_path().to_path_buf();
    blocking_io("copy file", move || std::fs::copy(&src, &dest_path)).await?;

    let created = ensure_song_config(player, &songs_path, song_dir.as_path()).await?;
    reload_songs(player).await?;

    Ok(ImportedFile {
        file: filename.to_string(),
        song: created.unwrap_or_else(|| name.to_string()),
    })
}

/// Held by each write of a song's `song.yaml` or one of its lighting files,
/// here and over MCP, and by an MCP edit from reading the file to writing
/// it back, so the edit is not made on a copy a save has replaced since:
/// that save would be lost (#578).
pub(crate) static SONG_WRITES: super::write_lock::WriteLock = super::write_lock::WriteLock::new();

/// Creates the new song `name` with the config `body` and reloads the songs
/// ([`create_song`], with [`SONG_WRITES`] held from the check that the song
/// is new: two creates of one name, or a create beside an MCP write making
/// it, would both find it missing and the later would replace the earlier).
pub(crate) async fn create_song_and_reload(
    player: &Player,
    name: &str,
    body: String,
) -> Result<(), SongError> {
    let _writing = SONG_WRITES.lock().await;
    create_song(player, &songs_dir(player), name, body).await?;
    reload_songs(player).await
}

/// Creates the new song `name` with the config `body`, by the one rule of
/// the web UI's create and MCP's `write_song` of a new song: the body
/// validated before any directory is made; the song directory made under
/// the songs root (`songs_path`); refused if it already has a config, or a
/// `song.yaml` of another kind ([`songs::song_config_to_write`]), or if
/// the name the body declares is another song's
/// ([`songs::check_song_name_free`]). The caller holds [`SONG_WRITES`] and
/// reloads the songs after. Answers the config written.
pub(crate) async fn create_song(
    player: &Player,
    songs_path: &std::path::Path,
    name: &str,
    body: String,
) -> Result<std::path::PathBuf, SongError> {
    let root = VerifiedRoot::new(songs_path)?;

    // Validate the YAML before creating any directories so we don't leave
    // orphan directories on disk when validation fails.
    let declared = validate_song_body(&body)?.name().to_string();
    // Not under a name another song holds (#674): the song is new, so
    // only the names taken matter.
    let taken = taken_names(player);
    let free = blocking_io("read the songs folder", {
        let songs_root = songs_path.to_path_buf();
        let declared = declared.clone();
        move || -> std::io::Result<_> {
            Ok(songs::check_song_name_free(
                std::path::Path::new(""),
                &declared,
                &songs_root,
                taken,
            ))
        }
    })
    .await?;
    free.map_err(SongError::Taken)?;

    let song_dir = SafePath::create_dir_nested(name, &root)?;
    let Some(config_path) = songs::song_config_to_write(song_dir.as_path()) else {
        return Err(SongError::Exists(format!("Song already exists: {name}")));
    };

    let cp = config_path.clone();
    blocking_io("write song config", move || {
        config_io::staged_write(&cp, &body)
    })
    .await?;
    Ok(config_path)
}

/// Validates and atomically writes song `name`'s config and reloads the
/// songs ([`save_song`], with [`SONG_WRITES`] held from the lookup: a song
/// deleted before the lock was taken is not written back).
///
/// The song directory must already exist (created via a create or a track
/// upload).
pub(crate) async fn save_song_and_reload(
    player: &Player,
    name: &str,
    body: String,
) -> Result<(), SongError> {
    let _writing = SONG_WRITES.lock().await;
    save_song(player, &songs_dir(player), name, body).await?;
    reload_songs(player).await
}

/// Writes the config `body` of the existing song `name`, by the one rule
/// of the web UI's save and MCP's `write_song`, `patch_song` and the song
/// light show tools' `lighting:` edits: to the file the song was loaded
/// from, in its folder, which must be there (not found otherwise); never
/// another song's config ([`songs::check_song_config_for`], #680); the body
/// validated; and not renamed onto a name another song holds
/// ([`songs::check_song_name_free`], #689). The caller holds
/// [`SONG_WRITES`] and reloads the songs after. Answers the config
/// written.
pub(crate) async fn save_song(
    player: &Player,
    songs_path: &std::path::Path,
    name: &str,
    body: String,
) -> Result<std::path::PathBuf, SongError> {
    // Look up the song in the registry to get its actual YAML (handles
    // nested folders and flat `<name>.yaml` songs).
    // Validated before writing to disk; the song it declares is what the
    // write is checked against, after the lookup.
    let parsed = validate_song_body(&body);
    let writing = parsed.as_ref().ok().map(|config| config.name());
    let config_path = song_config_path(player, songs_path, name, writing)?;
    let declared = parsed?.name().to_string();
    // Not renamed onto a name another song holds (#689).
    let taken = taken_names(player);
    let songs_root = songs_path.to_path_buf();
    let free = blocking_io("read the songs folder", {
        let config_path = config_path.clone();
        move || -> std::io::Result<_> {
            Ok(songs::check_song_name_free(
                &config_path,
                &declared,
                &songs_root,
                taken,
            ))
        }
    })
    .await?;
    free.map_err(SongError::Taken)?;

    let cp = config_path.clone();
    blocking_io("write song config", move || {
        config_io::staged_write(&cp, &body)
    })
    .await?;
    Ok(config_path)
}

/// What a track upload announces, checked as the upload is claimed is: the
/// file's name as [`validate_track_filename`] has it (a file not in the song
/// yet takes a new name's rules), and the song's directory, which a song not
/// there yet is given only when the bytes are used, so nothing is made here.
pub(crate) fn check_track_upload(
    player: &Player,
    name: &str,
    filename: &str,
) -> Result<(), SongError> {
    let songs_path = songs_dir(player);
    let exists = song_has_file(player, &songs_path, name, filename);
    validate_track_filename(filename, exists)?;
    if resolve_song_dir(player, &songs_path, name).is_err() {
        let root = VerifiedRoot::new(&songs_path)?;
        SafePath::validate_dir_to_create(root.as_path(), name)?;
        // A directory there already must stay inside the songs root
        // (a symlink out of it is refused), as making it would find.
        let target = root.as_path().join(name);
        if target.exists() {
            SafePath::resolve(&target, &root)?;
        }
    }
    Ok(())
}

/// A track file added to a song.
pub(crate) struct TrackAdded {
    /// The file's name in the song.
    pub(crate) file: String,
    /// A file of that name was there, and was replaced.
    pub(crate) replaced: bool,
}

/// Track files added to a song.
pub(crate) struct TracksAdded {
    /// The name the song has: a new one's may not be the one asked for, when
    /// that is taken (see [`import_file_to_song`]).
    pub(crate) song: String,
    pub(crate) files: Vec<TrackAdded>,
}

/// Adds the staged `files` to song `name`, whose directory (and config) is
/// made if the song is new, then reloads the songs. Each file is named as
/// the upload announced it and held to [`validate_track_filename`] again,
/// before anything is made or written. Files are written one after another:
/// one that fails leaves the ones before it written. The staged files are
/// deleted whether this succeeds or not.
pub(crate) async fn add_track_files(
    player: &Player,
    name: &str,
    files: Vec<crate::webui::transfers::Claimed>,
) -> Result<TracksAdded, SongError> {
    if files.is_empty() {
        return Err(SongError::Invalid("No files uploaded".to_string()));
    }
    let songs_path = songs_dir(player);
    for file in &files {
        let exists = song_has_file(player, &songs_path, name, &file.file_name);
        validate_track_filename(&file.file_name, exists)?;
    }
    let song_dir = resolve_or_create_song_dir(player, &songs_path, name)?;

    let targets: Vec<(crate::webui::transfers::Claimed, std::path::PathBuf)> = files
        .into_iter()
        .map(|file| {
            let path = song_dir.join_filename(&file.file_name);
            (file, path)
        })
        .collect();
    let added = blocking_io("write track", move || {
        let mut added = Vec::with_capacity(targets.len());
        for (file, path) in &targets {
            let replaced = path.exists();
            file.write_to(path)?;
            added.push(TrackAdded {
                file: file.file_name.clone(),
                replaced,
            });
        }
        Ok::<_, std::io::Error>(added)
    })
    .await?;

    let created = ensure_song_config(player, &songs_path, song_dir.as_path()).await?;
    reload_songs(player).await?;

    Ok(TracksAdded {
        song: created.unwrap_or_else(|| name.to_string()),
        files: added,
    })
}

// ---------------------------------------------------------------------------
// Song helper functions
// ---------------------------------------------------------------------------

/// Deserializes and validates a song config YAML body. Returns the parsed
/// config on success, or every validation problem with it.
fn validate_song_body(body: &str) -> Result<config::Song, SongError> {
    let tmp = tempfile::Builder::new()
        .suffix(".yaml")
        .tempfile()
        .map_err(|e| SongError::Internal(format!("Failed to create temp file: {e}")))?;
    std::fs::write(tmp.path(), body)
        .map_err(|e| SongError::Internal(format!("Failed to write temp file: {e}")))?;
    let song_config = config::Song::deserialize(tmp.path())
        .map_err(|e| SongError::InvalidConfig(vec![format!("{e}")]))?;
    if let Err(errors) = song_config.validate() {
        return Err(SongError::InvalidConfig(errors));
    }
    Ok(song_config)
}

/// Resolves an existing song directory by checking the player's registry first
/// (handles nested paths like `artist/album/song`), then falling back to a
/// direct path join. Returns an error if the song isn't found.
/// The YAML file a song is written to and deleted from: the one it was
/// loaded from ([`songs::Song::config_path_or_default`], a failed load's
/// likewise), or `song.yaml` in its folder for one the registry has not
/// seen; within the verified song folder, which is within the songs root.
/// Refused ([`SongError::Precondition`]) when that file is another song's
/// config ([`songs::check_song_config_for`], #680): a folder named as the
/// song whose `song.yaml` declares another, unless `writing` (the name the
/// config being written declares) is that song's own.
fn song_config_path(
    player: &Player,
    songs_path: &std::path::Path,
    name: &str,
    writing: Option<&str>,
) -> Result<std::path::PathBuf, SongError> {
    let song_dir = resolve_song_dir(player, songs_path, name)?;
    let path = player
        .songs()
        .config_path(name)
        .filter(|path| path.starts_with(song_dir.as_path()))
        .unwrap_or_else(|| song_dir.join_filename("song.yaml"));
    songs::check_song_config_for(&path, name, writing)
        .map_err(|e| SongError::Precondition(e.to_string()))?;
    Ok(path)
}

pub(crate) fn resolve_song_dir(
    player: &Player,
    songs_path: &std::path::Path,
    name: &str,
) -> Result<SafePath, SongError> {
    let root = VerifiedRoot::new(songs_path)?;

    // Check the registry first — handles songs in nested subdirectories.
    if let Some(song) = player
        .get_all_songs_playlist()
        .and_then(|p| p.get_song(name))
    {
        if let Ok(safe) = SafePath::resolve(song.base_path(), &root) {
            if safe.is_dir() {
                return Ok(safe);
            }
        }
    }

    // Check failures list — the song exists on disk but failed to load.
    let all_songs = player.songs();
    if let Some(failure) = all_songs.failure(name) {
        if let Ok(safe) = SafePath::resolve(failure.base_path(), &root) {
            if safe.is_dir() {
                return Ok(safe);
            }
        }
    }

    // Fall back to direct join for songs not yet in registry but on disk:
    // a relative path that stays under the root (no `..`, `\\` or NUL) and
    // names a folder within it, never the songs root itself, which a name
    // such as `.` or `Foo/..` would otherwise resolve to.
    if SafePath::validate_relative_segments(name).is_ok() {
        if let Ok(safe) = root.resolve(name) {
            if safe.is_dir() && safe.as_path() != root.as_path() {
                return Ok(safe);
            }
        }
    }

    Err(SongError::of_lookup(songs::SongLookupError::NotFound(
        name.to_string(),
    )))
}

/// Resolves a song directory from the registry, falling back to creating it
/// if the song isn't registered (for uploads to new songs).
pub(super) fn resolve_or_create_song_dir(
    player: &Player,
    songs_path: &std::path::Path,
    name: &str,
) -> Result<SafePath, SongError> {
    if let Ok(dir) = resolve_song_dir(player, songs_path, name) {
        return Ok(dir);
    }
    let root = VerifiedRoot::new(songs_path)?;
    Ok(SafePath::create_dir(&root.as_safe_path(), name, &root)?)
}

/// Removes a song name from all playlist YAML files on disk.
/// Silently skips files that can't be read or written.
fn remove_song_from_playlists(
    song_name: &str,
    playlists_dir: Option<&std::path::Path>,
    legacy_playlist_path: Option<&std::path::Path>,
) {
    let mut files_to_check: Vec<std::path::PathBuf> = Vec::new();

    // Each playlist's file, as the loader reads it (`playlist::playlist_files`).
    if let Some(dir) = playlists_dir {
        if dir.is_dir() {
            match crate::playlist::playlist_files(dir) {
                Ok(files) => files_to_check.extend(files.into_iter().map(|file| file.path)),
                Err(e) => tracing::warn!(
                    "{}: the playlists could not be listed, so \"{song_name}\" stays on them: {e}",
                    dir.display()
                ),
            }
        }
    }

    if let Some(legacy) = legacy_playlist_path {
        if legacy.is_file() {
            files_to_check.push(legacy.to_path_buf());
        }
    }

    for path in &files_to_check {
        let Ok(playlist_config) = config::Playlist::deserialize(path) else {
            continue;
        };
        let songs = playlist_config.songs();
        if !songs.iter().any(|s| s == song_name) {
            continue;
        }
        // Rebuild without the deleted song
        let filtered: Vec<String> = songs.iter().filter(|s| *s != song_name).cloned().collect();
        let updated = playlist_config.with_songs(&filtered);
        if let Ok(yaml) = crate::util::to_yaml_string(&updated) {
            let _ = crate::util::write_file(path, yaml.as_bytes());
        }
    }
}

/// Generates a song config for a song directory if it has none yet
/// ([`songs::song_config_to_write`]), named by the one rule for a new
/// song's name ([`songs::new_song`], as the bulk import names them): its
/// folder's, unless a song loaded, failed or configured under the songs
/// root has it (#674). One already there (`song.yaml`, `song.yml`, or one
/// of another kind) is left untouched so that manual edits (track names,
/// lighting config, etc.) are preserved. Answers the new song's name, which
/// is the one it is loaded as; `None` when the folder had a config.
pub(super) async fn ensure_song_config(
    player: &Player,
    songs_path: &std::path::Path,
    song_dir: &std::path::Path,
) -> Result<Option<String>, SongError> {
    let taken = taken_names(player);
    let (songs_path, song_dir) = (songs_path.to_path_buf(), song_dir.to_path_buf());
    // Off the async worker: the names configured under the songs root are
    // a walk of the whole tree, and the new config a scan of the audio.
    blocking_io("write the song config", move || {
        let song = songs::new_song(&songs_path, &song_dir, taken).map_err(|e| e.to_string())?;
        match song {
            Some(song) => song
                .write()
                .map(|()| Some(song.name))
                .map_err(|e| e.to_string()),
            None => Ok(None),
        }
    })
    .await
}

/// Whether song `name` has a file `filename` already, for
/// [`validate_track_filename`]. False for a song not yet created, or a
/// filename that is not safe to look up.
fn song_has_file(
    player: &Player,
    songs_path: &std::path::Path,
    name: &str,
    filename: &str,
) -> bool {
    SafePath::validate_name(filename).is_ok()
        && resolve_song_dir(player, songs_path, name)
            .is_ok_and(|dir| dir.join_filename(filename).exists())
}

/// Validates that a track filename is safe and has a supported extension.
/// One not already in the song (`exists` false) is a new file, and must also
/// pass the rules for new names; one there is replaced under its own name.
pub(super) fn validate_track_filename(filename: &str, exists: bool) -> Result<(), SongError> {
    if SafePath::validate_name_to_write(filename, exists).is_err() {
        return Err(SongError::Invalid("Invalid filename".to_string()));
    }

    let path = std::path::Path::new(filename);
    if FileKind::of(path) == FileKind::Other {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        return Err(SongError::Invalid(format!("Unsupported file type: .{ext}")));
    }

    Ok(())
}

#[cfg(test)]
mod test {
    /// A deleted song comes off each playlist's file as the loader reads
    /// it: a `.yml` alone is edited, and with both `.yaml` and `.yml` only
    /// the `.yaml`. `loud.YAML` is no playlist and `all_songs.yaml` no file
    /// of one, so both are left alone.
    #[test]
    fn remove_song_from_playlists_edits_each_names_file() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        let both = "songs:\n- Gone\n- Kept\n";
        for file in [
            "old.yml",
            "set.yaml",
            "set.yml",
            "loud.YAML",
            "all_songs.yaml",
        ] {
            std::fs::write(dir.join(file), both).unwrap();
        }
        super::remove_song_from_playlists("Gone", Some(dir), None);
        let read = |file: &str| std::fs::read_to_string(dir.join(file)).unwrap();
        for file in ["old.yml", "set.yaml"] {
            assert!(!read(file).contains("Gone"), "{file}: {}", read(file));
            assert!(read(file).contains("Kept"), "{file}: {}", read(file));
        }
        assert_eq!(read("set.yml"), both);
        assert_eq!(read("loud.YAML"), both);
        assert_eq!(read("all_songs.yaml"), both);
    }
}
