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

use crate::webui::wire::fixture_types::{
    BeamBrief, FixtureTypeDeleted, FixtureTypeEntry, FixtureTypeGdtf, FixtureTypeList,
    FixtureTypeSaved, GdtfBeam, GdtfInspection, GdtfMode, GdtfSummary, LightingFileError, ModeUse,
    SceneFixture, StrobeRange, TypeVenue, TypeVenueFixture, VenueDetail, VenueList, VenueScene,
};
use crate::webui::wire::messages::FixturePlacement;
use crate::webui::wire::preview::{
    LightFileInfo, LightingGroup, LightingGroups, LightingValidation,
};
use axum::http::StatusCode;
use serde_json::json;

use super::super::config_io;
use super::super::safe_path::VerifiedRoot;
use super::lighting_writes::{self, Answer};
use crate::lighting;
use crate::lighting::files::Kind;

/// The canonical project root the lighting endpoints write into.
pub(crate) fn project_root(config_path: &std::path::Path) -> Result<std::path::PathBuf, Answer> {
    let canonical = config_path.canonicalize().map_err(|_| {
        Answer::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to resolve config path",
        )
    })?;
    Ok(canonical
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf())
}

/// The name an uploaded archive is kept under in the library: the final
/// component of whatever the browser sent, since it becomes a path
/// component. None when there is no such component.
pub(crate) fn archive_file_name(raw: &str) -> Option<String> {
    std::path::Path::new(raw)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
}

// The archive's fixture name as a fixture type name: the importer's rule,
// so the name the picker suggests is the name an import writes.
use lighting::import::suggested_type_name;

/// A GDTF archive's modes, as its fixture page shows them: each mode's
/// footprint, what a show can do in it (capabilities, cells, strobe range),
/// its channel map and the distiller's warnings, or the reason it cannot be
/// imported.
fn inspect_description(description: &lighting::gdtf::Description) -> GdtfInspection {
    // Distilling every mode is a few milliseconds each (design 12.2), so the
    // picker can say what each one lets a show do before anything is written.
    let suggested = suggested_type_name(&description.name);
    let summaries = lighting::gdtf::mode_summaries(description);
    let modes = summaries
        .iter()
        .enumerate()
        .map(|(index, summary)| {
            let mut mode = GdtfMode {
                name: summary.name.clone(),
                footprint: summary.footprint,
                capabilities: None,
                wants: None,
                cells: None,
                strobe_range: None,
                channels: None,
                refused: None,
            };
            // The importer finds a mode by name and takes the first match, so
            // a repeat could only ever import its namesake.
            let repeats = summaries[..index].iter().any(|m| m.name == summary.name);
            let distilled = if repeats {
                Err("another mode has this name; an import would take the first".to_string())
            } else {
                lighting::gdtf::distill(description, &summary.name, &suggested)
                    .map_err(|e| e.to_string())
            };
            match distilled {
                Ok(distilled) => {
                    let fixture_type = &distilled.fixture_type;
                    let mut info = lighting::effects::FixtureInfo::new(
                        suggested.clone(),
                        1,
                        1,
                        suggested.clone(),
                        fixture_type.channels().clone(),
                        None,
                    );
                    info.cells = fixture_type.cells().to_vec();
                    let fit = lighting::fit::FitFixture::from_info(&info, &[]);
                    mode.capabilities = Some(fit.capability_names());
                    mode.wants = Some(fit.wants());
                    mode.cells = Some(fixture_type.cells().len());
                    mode.strobe_range =
                        fixture_type
                            .max_strobe_frequency()
                            .map(|max_hz| StrobeRange {
                                min_hz: fixture_type.min_strobe_frequency(),
                                max_hz,
                            });
                    let mut channels: Vec<(u16, String)> = fixture_type
                        .channels()
                        .iter()
                        .map(|(name, offset)| (*offset, name.clone()))
                        .collect();
                    channels.sort();
                    mode.channels = Some(channels);
                }
                Err(reason) => mode.refused = Some(reason),
            }
            mode
        })
        .collect();
    GdtfInspection { modes }
}

/// What a GDTF upload announces: the name it will be kept under in the
/// library (see [`archive_file_name`]), or the refusal.
pub(crate) fn check_gdtf_upload(file_name: &str) -> Result<String, Answer> {
    archive_file_name(file_name).ok_or_else(|| Answer::bad("No file uploaded"))
}

/// Imports the staged GDTF archive `file` through the shared importer (the
/// same one behind the CLI and MCP): a copy into lighting/library/, where it
/// is a fixture (design §22), and a warmed rig for its page. One step: a
/// venue fixture can use any of its modes straight away. Answers the import
/// report (`already_imported` when the same archive was imported before and
/// nothing was written; `renamed_from` when the archive's fixture name was
/// taken). `name` is a name for the fixture; given, a record pins it. A
/// different archive of the same file name already in the library, or a
/// record name or file that is taken, is a 409; anything else refused is a
/// 400. The staged file is deleted whether this succeeds or not.
pub(crate) async fn import_gdtf(
    config_path: &std::path::Path,
    file: crate::webui::transfers::Claimed,
    name: Option<&str>,
) -> Result<lighting::import::GdtfImport, Answer> {
    let project = project_root(config_path)?;
    // Where the engine reads fixture types on this host, as the CLI and MCP
    // import into (#673): its name check and a pinned record go there.
    let types_dir = lighting_dir(config_path, None, LightingDir::FixtureTypes)?;
    let name = name.map(str::to_string);
    // The inner Result survives the blocking call so an import refusal (bad
    // archive, name collision, archive conflict) surfaces as the caller error
    // it is, not a 500.
    let report = super::helpers::blocking_answer("import gdtf", move || {
        let bytes = file.read().map_err(|e| e.to_string())?;
        // A pinned name writes a fixture type record, in turn with the
        // other fixture type and venue saves (#578).
        let _guard = VENUE_WRITES.blocking_lock();
        Ok::<_, String>(
            lighting::import::import_gdtf_bytes(
                &bytes,
                &file.file_name,
                name.as_deref(),
                &project,
                &types_dir,
            )
            .map_err(|e| lighting_writes::import_error(e.as_ref())),
        )
    })
    .await?
    .map_err(|(status, body)| Answer { status, body })?;
    Ok(report)
}

/// Recursively finds light show files (`.light`, in any case:
/// `songs::FileKind::of`) under a directory.
pub(crate) fn find_light_files(
    base: &std::path::Path,
    dir: &std::path::Path,
    results: &mut Vec<LightFileInfo>,
) -> Result<(), std::io::Error> {
    // codeql[rust/path-injection] dir is always state.songs_path(), set at startup.
    if !dir.is_dir() {
        return Ok(());
    }
    // codeql[rust/path-injection] dir is always state.songs_path(), set at startup.
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            find_light_files(base, &path, results)?;
        } else if crate::songs::FileKind::is_lighting(&path) {
            let relative = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            results.push(LightFileInfo {
                name,
                path: relative,
            });
        }
    }
    Ok(())
}

/// The 404 for a lighting file `name` that is not there.
fn lighting_file_not_found(name: &str) -> Answer {
    Answer::new(
        StatusCode::NOT_FOUND,
        format!("Lighting file not found: {name}"),
    )
}

/// The text of a song's light show: `name` is the file's path under the
/// songs directory `songs_path` (as the listing returns it). A path that
/// leaves the songs directory, by `..`, an absolute name or a link, is not
/// found.
pub(crate) async fn read_light_show(
    songs_path: &std::path::Path,
    name: &str,
) -> Result<String, Answer> {
    use super::super::safe_path::SafePath;

    let root = VerifiedRoot::new(songs_path)?;
    let safe = SafePath::resolve(&songs_path.join(name), &root)
        .map_err(|_| lighting_file_not_found(name))?;

    let path = safe.as_path().to_path_buf();
    super::helpers::blocking_answer("read lighting file", move || {
        crate::util::read_to_string(&path)
    })
    .await
}

/// The one songs-root rule for a song's light show, written, deleted or
/// validated, by the web UI and MCP alike: the folder `path` is in is
/// inside the songs root `songs_root`, canonically, as
/// [`SafePath::resolve`](super::super::safe_path::SafePath::resolve) holds
/// every path the lighting endpoints take. A folder not there yet is held
/// to it by its nearest part that is there, canonicalized (a symlinked
/// `lighting/` pointing outside is outside), the rest of it plain names
/// (no `..`, and no link that leads nowhere, which a write would follow).
///
/// Answers the folder resolved, joined with `path`'s own name, which is
/// not resolved: a link there is the link, as the song loader names it,
/// so its `include`s resolve where the loader resolves them, and a delete
/// removes the link, never what it leads to. A write follows it, so
/// [`light_show_to_write`] holds what it leads to as well. 403 outside
/// the root.
pub(crate) fn light_show_in_songs_root(
    root: &VerifiedRoot,
    path: &std::path::Path,
) -> Result<std::path::PathBuf, Answer> {
    use super::super::safe_path::{SafePath, SafePathError};

    let outside = || Answer::from(SafePathError::OutsideRoot);
    let (Some(folder), Some(file_name)) = (path.parent(), path.file_name()) else {
        return Err(outside());
    };
    let mut missing = Vec::new();
    let mut existing = folder;
    loop {
        match SafePath::resolve(existing, root) {
            Ok(safe) => {
                let mut resolved = safe.as_path().to_path_buf();
                resolved.extend(missing.iter().rev());
                resolved.push(file_name);
                return Ok(resolved);
            }
            Err(SafePathError::NotFound) => {
                // A link that leads nowhere: what a write made through it
                // would land on is not known to be inside.
                if std::fs::symlink_metadata(existing).is_ok() {
                    return Err(outside());
                }
                match (existing.parent(), existing.file_name()) {
                    (Some(parent), Some(name)) => {
                        missing.push(name.to_os_string());
                        existing = parent;
                    }
                    // A `..` past a part that is not there, or nothing
                    // there at all: nowhere that can be shown to be inside.
                    _ => return Err(outside()),
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Where a write of the light show `path` lands, held to the songs root:
/// [`light_show_in_songs_root`], and, the file being a link, which the
/// write follows, what it leads to inside the songs root too (403
/// otherwise). A link that leads nowhere is refused: the write would make
/// its target, wherever that is. Both the name written and the file a
/// link leads to must be `.light` files (400 otherwise): a show is never
/// written over a song's config or anything else.
fn light_show_to_write(
    root: &VerifiedRoot,
    path: &std::path::Path,
) -> Result<std::path::PathBuf, Answer> {
    use super::super::safe_path::{SafePath, SafePathError};

    let not_a_show = || Answer::bad("Can only write .light files");
    let is_show = crate::songs::FileKind::is_lighting;
    let path = light_show_in_songs_root(root, path)?;
    if !is_show(&path) {
        return Err(not_a_show());
    }
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        let target = SafePath::resolve(&path, root).map_err(|e| match e {
            SafePathError::NotFound => Answer::from(SafePathError::OutsideRoot),
            e => Answer::from(e),
        })?;
        if !is_show(target.as_path()) {
            return Err(not_a_show());
        }
    }
    Ok(path)
}

/// Refuses a light show path, resolved as the handler resolves it, that is
/// inside the venues or fixture types directory the engine reads on this
/// host: those files are written by name, by the one rule for them
/// ([`lighting_writes`]), never as a song's light show. Compared by
/// identity ([`crate::util::is_within_dir`]), so no spelling (`..`, a
/// symlink, another case) gets around it.
async fn refuse_venue_or_type_file(
    config_path: &std::path::Path,
    path: &std::path::Path,
    name: &str,
) -> Result<(), lighting_writes::Answer> {
    let config_path = config_path.to_path_buf();
    let project = crate::util::project_dir_of(&config_path);
    let (path, name) = (path.to_path_buf(), name.to_string());
    let refused = tokio::task::spawn_blocking(move || {
        // A config that does not load is a 500, as at every endpoint that
        // reads these directories: the defaults may not be this host's.
        let dirs = configured_lighting_dirs(&config_path)?;
        Ok::<_, String>(
            [
                (Kind::Venue, dirs.venues),
                (Kind::FixtureType, dirs.fixture_types),
            ]
            .into_iter()
            .find(|(_, dir)| crate::util::is_within_dir(&path, &project.join(dir))),
        )
    })
    .await
    .unwrap_or(Ok(None))
    .map_err(|e| lighting_writes::Answer::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    match refused {
        Some((kind, dir)) => Err(lighting_writes::Answer::bad(format!(
            "{name} is in the {noun}s directory ({dir}): a {noun} is saved by name, on the \
             Lighting page or with MCP's {noun} tools; this endpoint is for song light shows",
            noun = kind.noun()
        ))),
        None => Ok(()),
    }
}

/// Saves a song's light show: `name` is the file's path under the songs
/// directory `songs_path`, `body` its text ([`save_light_show`]). A new
/// file goes in a folder already there, as the editor saves it.
pub(crate) async fn put_light_show(
    player: &crate::player::Player,
    config_path: &std::path::Path,
    songs_path: &std::path::Path,
    name: &str,
    body: String,
) -> Result<(), Answer> {
    let root = VerifiedRoot::new(songs_path)?;
    let verified_path = match light_show_in_songs_root(&root, &songs_path.join(name)) {
        Ok(path) if path.parent().is_some_and(std::path::Path::is_dir) => path,
        Ok(_) => return Err(lighting_file_not_found(name)),
        Err(e) => return Err(e),
    };

    // Held from reading the song's tempo: a song save changing it reloads
    // the songs under this lock.
    let _writing = super::songs_api::SONG_WRITES.lock().await;
    let tempo = song_tempo_for_path(player, &verified_path);
    save_light_show(
        config_path,
        &root,
        &verified_path,
        name,
        body,
        tempo.as_ref(),
    )
    .await
}

/// Writes a song's light show `body` to `path` (shown as `name`), by the
/// one rule of the web UI's light show save and MCP's
/// `write_song_lighting` and `patch_song_lighting`: inside `songs_root`
/// ([`light_show_to_write`]); never a file in the venues or fixture
/// types directory ([`refuse_venue_or_type_file`]); parsed first (400 with
/// its `errors`, each `Invalid light show DSL: ...`, as the venue and
/// fixture type saves say it) against `tempo`, the tempo its
/// song loads it with (a show inheriting its song's tempo is valid content
/// that a tempo-less parse rejects), its `include`s resolved where it is
/// written and kept inside `root`; its directory made only then. The
/// caller holds [`super::songs_api::SONG_WRITES`].
pub(crate) async fn save_light_show(
    config_path: &std::path::Path,
    root: &VerifiedRoot,
    path: &std::path::Path,
    name: &str,
    body: String,
    tempo: Option<&crate::tempo::TempoMap>,
) -> Result<(), Answer> {
    // The venues and fixture types refusal first: it says what to do
    // instead, where the songs-root one, for those directories outside the
    // songs root, says only no.
    refuse_venue_or_type_file(config_path, path, name).await?;
    let path = &light_show_to_write(root, path)?;
    if let Err(errors) =
        config_io::validate_light_show(&body, Some(path), Some(root.as_path()), tempo)
    {
        let errors: Vec<String> = errors
            .into_iter()
            .map(|e| format!("Invalid light show DSL: {e}"))
            .collect();
        return Err(Answer {
            status: StatusCode::BAD_REQUEST,
            body: json!({"errors": errors}),
        });
    }

    let vp = path.to_path_buf();
    super::helpers::blocking_answer("write lighting file", move || {
        if let Some(parent) = vp.parent() {
            crate::util::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        config_io::staged_write(&vp, &body)
    })
    .await
}

/// Deletes a song's `.light` file from disk: `name` is its path under the
/// songs directory `songs_path`. By the one rule of MCP's
/// `delete_song_lighting` too ([`check_light_show_delete`]): a link is
/// removed, never what it leads to.
pub(crate) async fn delete_light_show(
    config_path: &std::path::Path,
    songs_path: &std::path::Path,
    name: &str,
) -> Result<(), Answer> {
    let root = VerifiedRoot::new(songs_path)?;
    // In turn with the file's writes: an MCP patch reading it before the
    // delete would write it back after.
    let _writing = super::songs_api::SONG_WRITES.lock().await;
    let path = check_light_show_delete(config_path, &root, &songs_path.join(name), name).await?;
    remove_light_show(songs_path, &path).await
}

/// Whether the file at `path` (shown as `name`) may be deleted as a song's
/// light show, by the one rule of the web UI's delete and MCP's
/// `delete_song_lighting`: a `.light` file (400 otherwise), never one in
/// the venues or fixture types directory ([`refuse_venue_or_type_file`]),
/// in a folder inside `songs_root` ([`light_show_in_songs_root`]), and
/// there (404 otherwise). Answers the file to remove: a link itself, not
/// what it leads to, whose name is the one checked.
pub(crate) async fn check_light_show_delete(
    config_path: &std::path::Path,
    root: &VerifiedRoot,
    path: &std::path::Path,
    name: &str,
) -> Result<std::path::PathBuf, Answer> {
    // The name checked is the one removed: the resolved path keeps it.
    if !crate::songs::FileKind::is_lighting(path) {
        return Err(Answer::bad("Can only delete .light files"));
    }
    // As a save: the venues and fixture types refusal, which says what to
    // do instead, before the songs root's. Of where the file is, its
    // folder: a link there is removed, not what it leads to, so where it
    // leads does not matter.
    let folder = path.parent().unwrap_or(path);
    refuse_venue_or_type_file(config_path, folder, name).await?;
    let path = light_show_in_songs_root(root, path)?;
    if std::fs::symlink_metadata(&path).is_err() {
        return Err(lighting_file_not_found(name));
    }
    Ok(path)
}

/// Removes a song's light show file, [`check_light_show_delete`] first,
/// off the async runtime: the file itself, a link and not what it leads
/// to, held to `songs_root` again ([`light_show_in_songs_root`]), as it
/// is checked some time before.
pub(crate) async fn remove_light_show(
    songs_root: &std::path::Path,
    path: &std::path::Path,
) -> Result<(), Answer> {
    let root = VerifiedRoot::new(songs_root)?;
    // codeql[rust/path-injection] the folder is verified via SafePath::resolve against the songs root.
    let path = light_show_in_songs_root(&root, path)?;
    super::helpers::blocking_answer("delete lighting file", move || std::fs::remove_file(&path))
        .await
}

/// Checks light show text without saving it. `song` names the song the text
/// belongs to: measure-based timing does not parse at all without a tempo
/// map, so validating a bar/beat show without it reports a failure the save
/// path does not agree with. `file` is the lighting file (as
/// [`put_light_show`] names it) the text is a draft of, so its `include`s
/// resolve where saving it would. The problems are the answer; a `file`
/// that leads outside the songs directory is the error.
pub(crate) fn validate_light_show_text(
    player: &crate::player::Player,
    songs_path: &std::path::Path,
    body: &str,
    song: Option<&str>,
    file: Option<&str>,
) -> Result<LightingValidation, Answer> {
    let song = song.and_then(|name| player.songs().get(name).ok());
    let tempo = song.as_ref().and_then(|song| song.lighting_tempo_map());
    let path = match file {
        Some(file) => {
            let root = VerifiedRoot::new(songs_path)?;
            Some(light_show_in_songs_root(&root, &songs_path.join(file))?)
        }
        None => song.as_ref().map(|song| song.lighting_draft_path()),
    };
    Ok(
        match config_io::validate_light_show(
            body,
            path.as_deref(),
            Some(songs_path),
            tempo.as_ref(),
        ) {
            Ok(()) => LightingValidation {
                valid: true,
                errors: None,
            },
            Err(errors) => LightingValidation {
                valid: false,
                errors: Some(errors),
            },
        },
    )
}

// ---------------------------------------------------------------------------
// Fixture Type & Venue CRUD endpoints
// ---------------------------------------------------------------------------

/// One of the project's lighting directories. A request names it (`?dir=`
/// and the like), or it is the one the engine reads on this host, by the
/// shared reader ([`crate::config::Player::lighting_dirs_at`]) the CLI and
/// MCP ask (#673): the defaults only where the config sets none.
#[derive(Clone, Copy, Debug)]
pub(super) enum LightingDir {
    FixtureTypes,
    Venues,
}

/// The lighting directories the engine reads on this host, by the shared
/// reader; a config that does not load is the error to answer (a 500), the
/// one rule of the web endpoints for it.
pub(super) fn configured_lighting_dirs(
    config_path: &std::path::Path,
) -> Result<crate::config::lighting::ProjectDirs, String> {
    // codeql[rust/path-injection] config_path is set at startup, not user input.
    crate::config::Player::lighting_dirs_at(config_path)
        .map_err(|e| format!("the config does not load: {e}"))
}

/// The project-relative directory `which` a request names, else the one
/// the engine reads on this host. A config that does not load is a 500,
/// as it is to MCP: the defaults might not be where this host's files are.
pub(super) fn lighting_dir(
    config_path: &std::path::Path,
    requested: Option<&str>,
    which: LightingDir,
) -> Result<String, Answer> {
    if let Some(dir) = requested.filter(|d| !d.is_empty()) {
        return Ok(dir.to_string());
    }
    let dirs = configured_lighting_dirs(config_path)
        .map_err(|e| Answer::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(match which {
        LightingDir::FixtureTypes => dirs.fixture_types,
        LightingDir::Venues => dirs.venues,
    })
}

/// The file a fixture type of this name is declared in, if one is, in the
/// directory or any subdirectory: the file the lighting system loads it
/// from, by the same reader ([`lighting::project_files::type_files`]). The
/// file's stem need not match the name: an import writes
/// `astera_pixelbrick.fixture` for "Astera-PixelBrick", and a file written
/// by hand is called whatever its author liked. A file that does not parse
/// is never searched for it.
pub(super) fn existing_fixture_type_file(
    dir: &std::path::Path,
    name: &str,
) -> Option<std::path::PathBuf> {
    lighting::project_files::type_files(dir)
        .get(name)
        .map(|declared| declared.file.clone())
}

/// [`existing_fixture_type_file`], off the async runtime.
pub(super) async fn locate_fixture_type_file(
    dir: &std::path::Path,
    name: &str,
) -> Result<Option<std::path::PathBuf>, Answer> {
    let (dir, name) = (dir.to_path_buf(), name.to_string());
    super::helpers::blocking_answer("locate fixture type file", move || {
        Ok::<_, String>(existing_fixture_type_file(&dir, &name))
    })
    .await
}

/// Serialises every read-check-write of a venue file, so two saves cannot
/// interleave between the version check and the write; and every write of a
/// fixture type file, which a fixture's settings save rewrites with its
/// venues. MCP's venue and fixture type writes take it too, and its edits
/// hold it from their read to their write, or a save made between the two
/// would be lost (#578). The saves here run on a blocking thread, and take
/// it with `blocking_lock`.
pub(crate) static VENUE_WRITES: super::write_lock::WriteLock = super::write_lock::WriteLock::new();

/// The version of a venue file: a hash of its bytes. A client sends back the
/// one it read (`If-Match`); a save whose version is no longer the file's is
/// refused, the same optimistic concurrency the config store has for inline
/// profiles.
pub(crate) fn content_version(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// The 409 for a save made against a version that is no longer current.
pub(crate) fn stale_venue_response(name: &str, current: Option<String>) -> lighting_writes::Answer {
    lighting_writes::Answer {
        status: StatusCode::CONFLICT,
        body: json!({
            "error": format!(
                "venue \"{name}\" changed since you loaded it (another tab, page or an edit by \
                 hand); it has been reloaded, so reapply your change"
            ),
            "conflict": true,
            "version": current,
        }),
    }
}

/// Resolves a lighting directory path relative to the project root: the
/// one the request names (from a query param), else this host's
/// ([`lighting_dir`]). Returns an error response if the project root cannot
/// be canonicalized or the resolved path would escape it.
pub(super) fn resolve_lighting_dir(
    config_path: &std::path::Path,
    override_dir: Option<&str>,
    which: LightingDir,
) -> Result<std::path::PathBuf, Answer> {
    use super::super::safe_path::{SafePath, VerifiedRoot};

    let root = VerifiedRoot::new(&project_root(config_path)?)?;

    let relative = lighting_dir(config_path, override_dir, which)?;

    Ok(SafePath::validate_relative(&relative, &root)?)
}

/// Checks a referential type's archive the way the lighting system will when
/// it expands one: the path is project-relative, stays inside the project, and
/// is a file that is actually there. Nothing is distilled — that happens at
/// load, through the cache — but a `.fixture` whose archive is missing or
/// outside the project cannot load, and a save is the last moment where the
/// person still has the text in front of them to fix it.
pub(super) fn validate_referential_archives(
    root: &std::path::Path,
    types: &std::collections::HashMap<String, lighting::types::FixtureType>,
) -> Result<(), lighting_writes::Answer> {
    let canonical_root = root.canonicalize().map_err(|_| {
        lighting_writes::Answer::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to resolve project root",
        )
    })?;
    for (name, fixture_type) in types {
        if let Some(source) = fixture_type.source() {
            resolve_referential_archive(&canonical_root, name, source)?;
        }
    }
    Ok(())
}

/// The project root, canonicalized, for [`resolve_referential_archive`].
pub(super) fn canonical_project_root(root: &std::path::Path) -> Result<std::path::PathBuf, Answer> {
    root.canonicalize().map_err(|_| {
        Answer::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to resolve project root",
        )
    })
}

/// One referential type's archive, resolved as the lighting system resolves
/// it: joined to the project root, canonicalized, inside the project, and a
/// file. The 400 names the type and the path as the `.fixture` writes it.
fn resolve_referential_archive(
    canonical_root: &std::path::Path,
    name: &str,
    source: &lighting::types::GdtfSource,
) -> Result<std::path::PathBuf, lighting_writes::Answer> {
    let archive = canonical_root.join(&source.path);
    let canonical = archive.canonicalize().map_err(|e| {
        lighting_writes::Answer::bad(format!(
            "fixture type \"{}\" references a GDTF archive that cannot be read: {}: {}",
            name, source.path, e
        ))
    })?;
    if !canonical.starts_with(canonical_root) {
        return Err(lighting_writes::Answer::bad(format!(
            "fixture type \"{}\" references a GDTF archive outside the project: {}",
            name, source.path
        )));
    }
    if !canonical.is_file() {
        return Err(lighting_writes::Answer::bad(format!(
            "fixture type \"{}\" references a GDTF archive that is not a file: {}",
            name, source.path
        )));
    }
    Ok(canonical)
}

/// Validates that a fixture type or venue name is safe for use as a filename.
pub(crate) fn validate_lighting_name(name: &str) -> Result<(), lighting_writes::Answer> {
    validate_lighting_name_to_write(name, true)
}

/// As [`validate_lighting_name`], for a fixture type or venue being written:
/// one not already on disk (`exists` false) is being created, and its name
/// must also pass the rules for new names.
pub(crate) fn validate_lighting_name_to_write(
    name: &str,
    exists: bool,
) -> Result<(), lighting_writes::Answer> {
    use super::super::safe_path::SafePath;
    if SafePath::validate_name_to_write(name, exists).is_err() {
        return Err(lighting_writes::Answer::bad("Invalid name"));
    }
    if RESERVED_NAMES.contains(&name) {
        return Err(lighting_writes::Answer::bad(format!(
            "`{name}` is reserved: it is the name of an API route, so a file called that could not be addressed"
        )));
    }
    Ok(())
}

/// Path segments the lighting API uses as routes beside `{name}`.
const RESERVED_NAMES: &[&str] = &["lock", "validate", "activate"];

/// Every fixture type of the directory `dir` (else this host's), its files
/// that did not read, and, for a GDTF type, what its archive says and the
/// venues in `venues_dir` (else this host's) that use it. A directory that
/// is not there lists nothing; one that leads outside the project is
/// refused.
pub(crate) async fn list_fixture_types(
    config_path: &std::path::Path,
    dir: Option<&str>,
    venues_dir: Option<&str>,
) -> Result<FixtureTypeList, Answer> {
    let dir = resolve_lighting_dir(config_path, dir, LightingDir::FixtureTypes)?;
    let venues_dir = resolve_lighting_dir(config_path, venues_dir, LightingDir::Venues)?;
    let root = canonical_project_root(&project_root(config_path)?)?;
    let all = super::helpers::blocking_answer("load fixture types", move || {
        let mut all = std::collections::HashMap::new();
        let mut referential = Vec::new();
        // The project's one reader of fixture types (the lighting system
        // loads through it too): every type file in the directory and its
        // subdirectories, then every GDTF in the library no record points
        // at. A type's file travels with it: the form a type is in decides
        // how it may be edited.
        let project = lighting::project_files::fixture_types(&root, Some(&dir));
        let shown = |path: &std::path::Path| lighting::project_files::display_in(&dir, path);
        let mut errors: Vec<LightingFileError> = project
            .files
            .problems
            .iter()
            .map(|p| LightingFileError {
                file: shown(&p.file),
                error: p.error.clone(),
            })
            .collect();
        for declared in &project.files.items {
            let fixture_type = &declared.item;
            if let Some(source) = fixture_type.source() {
                referential.push((declared.name.clone(), source.clone()));
            }
            all.insert(
                declared.name.clone(),
                FixtureTypeEntry {
                    referential: fixture_type.source().is_some(),
                    // A native type's addresses, for the venue editor's
                    // patch; a GDTF fixture's are its mode's, so null here.
                    footprint: Some(
                        fixture_type
                            .source()
                            .is_none()
                            .then(|| fixture_type.footprint()),
                    ),
                    rich: fixture_type.uses_rich_channels(),
                    fixture_type: fixture_type.clone(),
                    file: Some(shown(&declared.file)),
                    extension: Some(
                        declared
                            .file
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or_default()
                            .to_string(),
                    ),
                    owner: project.files.record.owner(&declared.name).map(|owner| {
                        lighting_writes::wire_owner(
                            lighting::files::Kind::FixtureType,
                            &dir,
                            &owner,
                        )
                    }),
                    form_editable: crate::util::read_to_string(&declared.file).is_ok_and(|text| {
                        lighting::fixture_patch::form_expresses(&text, &declared.name)
                    }),
                    gdtf: None,
                    dsl: None,
                },
            );
        }
        let library = project.library;
        // An archive that will not read is the listing's error; a name
        // collision is already resolved (the second fixture is renamed).
        for finding in &library.findings {
            if finding.kind == "gdtf-unreadable" {
                errors.push(LightingFileError {
                    file: finding.file.clone(),
                    error: finding.message.clone(),
                });
            }
        }
        for library_type in &library.types {
            if all.contains_key(&library_type.name) {
                continue;
            }
            let declared = lighting::library::declared(library_type);
            referential.push((
                library_type.name.clone(),
                declared
                    .source()
                    .cloned()
                    .expect("a library type is GDTF-sourced"),
            ));
            all.insert(
                library_type.name.clone(),
                FixtureTypeEntry {
                    referential: true,
                    footprint: Some(None),
                    rich: false,
                    fixture_type: declared,
                    file: None,
                    extension: None,
                    owner: None,
                    form_editable: false,
                    gdtf: None,
                    dsl: None,
                },
            );
        }
        // What each referential type's archive says, for its card. One bad
        // archive is that card's `gdtf: null`, never the listing's failure.
        let usage = fixtures_by_type(&venues_dir);
        let cache = lighting::distill::DistillCache::new(root.join("lighting").join(".cache"));
        let mut archives = ArchiveCache::default();
        for (name, source) in referential {
            let fixtures: Vec<Option<&str>> = usage
                .get(&name)
                .into_iter()
                .flatten()
                .flat_map(|(_, f)| f.iter().map(|(_, mode)| mode.as_deref()))
                .collect();
            let parsed = archives.get(&root, &name, &source);
            let summary = parsed.map(|(bytes, description)| {
                gdtf_list_summary(&cache, bytes, description, &name, &fixtures)
            });
            if let Some(entry) = all.get_mut(&name) {
                entry.gdtf = Some(summary);
            }
        }
        Ok::<_, String>((all, errors))
    })
    .await?;
    let (all, errors) = all;
    Ok(FixtureTypeList {
        fixture_types: all,
        errors,
    })
}

/// One fixture type, with its file's text: a GDTF in the library with no
/// record is a fixture with no file. NOT_FOUND-class (404) when no file or
/// archive declares `name`.
pub(crate) async fn read_fixture_type(
    config_path: &std::path::Path,
    name: &str,
    dir: Option<&str>,
) -> Result<FixtureTypeEntry, Answer> {
    validate_lighting_name(name)?;
    let name = name.to_string();
    let dir = resolve_lighting_dir(config_path, dir, LightingDir::FixtureTypes)?;
    let found = {
        let (dir, name) = (dir.clone(), name.clone());
        super::helpers::blocking_answer("read fixture type", move || {
            let read = lighting::project_files::type_files(&dir);
            let (Some(declared), Some(owner)) = (read.get(&name), read.record.owner(&name)) else {
                return Ok(None);
            };
            let content = crate::util::read_to_string(&declared.file)?;
            Ok::<_, std::io::Error>(Some((
                declared.item.clone(),
                declared.file.clone(),
                lighting_writes::wire_owner(lighting::files::Kind::FixtureType, &dir, &owner),
                content,
            )))
        })
        .await?
    };
    let Some((ft, file_path, owner, content)) = found else {
        // A GDTF in the library with no record: a fixture with no file.
        let root = canonical_project_root(&project_root(config_path)?)?;
        let (types_dir, wanted) = (dir.clone(), name.clone());
        let found = super::helpers::blocking_answer("read the GDTF library", move || {
            Ok::<_, String>(
                lighting::library::unrecorded_types(&root, Some(&types_dir))
                    .get(&wanted)
                    .map(lighting::library::declared),
            )
        })
        .await?;
        return match found {
            Some(declared) => Ok(FixtureTypeEntry {
                referential: true,
                footprint: None,
                rich: false,
                fixture_type: declared,
                file: None,
                extension: None,
                owner: None,
                form_editable: false,
                gdtf: None,
                dsl: Some(None),
            }),
            None => Err(Answer::new(
                StatusCode::NOT_FOUND,
                format!("Fixture type not found: {}", name),
            )),
        };
    };
    Ok(FixtureTypeEntry {
        referential: ft.source().is_some(),
        footprint: None,
        rich: ft.uses_rich_channels(),
        fixture_type: ft,
        file: Some(crate::util::filename_display(&file_path).to_string()),
        extension: Some(
            file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_string(),
        ),
        owner: Some(owner),
        form_editable: lighting::fixture_patch::form_expresses(&content, &name),
        gdtf: None,
        dsl: Some(Some(content)),
    })
}

/// The GDTF source of a fixture type: its record's, or — for a GDTF in the
/// library with no record — its archive's, from the one function the engine
/// loads them with. `Ok(None)`: no such type; `Err`: a native type (the
/// 404 the caller answers).
async fn gdtf_source_of(
    root: &std::path::Path,
    dir: &std::path::Path,
    name: &str,
) -> Result<Option<lighting::types::GdtfSource>, Answer> {
    if let Some(file_path) = locate_fixture_type_file(dir, name).await? {
        let content = super::helpers::blocking_answer("read fixture type", move || {
            crate::util::read_to_string(&file_path)
        })
        .await?;
        let types = lighting::parser::parse_fixture_types(&content).map_err(|e| {
            Answer::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to parse fixture type: {}", e),
            )
        })?;
        let Some(fixture_type) = types.get(name) else {
            return Ok(None);
        };
        return match fixture_type.source() {
            Some(source) => Ok(Some(source.clone())),
            None => Err(Answer::new(
                StatusCode::NOT_FOUND,
                format!("fixture type \"{}\" is not a GDTF type", name),
            )),
        };
    }
    let (root, dir, name) = (root.to_path_buf(), dir.to_path_buf(), name.to_string());
    super::helpers::blocking_answer("read the GDTF library", move || {
        Ok::<_, String>(
            lighting::library::unrecorded_types(&root, Some(&dir))
                .get(&name)
                .and_then(|t| lighting::library::declared(t).source().cloned()),
        )
    })
    .await
}

/// What a GDTF fixture's
/// archive holds, for its page: every mode as the mode picker shows it,
/// the venue fixtures using it in their
/// own modes, and the rig model and thumbnail the 3D view draws (paths in
/// the asset store) — drawn in the first mode it can drive. The rig is
/// written through the expansion cache as the lighting system would write
/// it; a rig that cannot be made is not an error here — the modes are still
/// worth showing.
pub(crate) async fn fixture_type_gdtf(
    config_path: &std::path::Path,
    name: &str,
    dir: Option<&str>,
    venues_dir: Option<&str>,
) -> Result<FixtureTypeGdtf, Answer> {
    validate_lighting_name(name)?;
    let name = name.to_string();
    let dir = resolve_lighting_dir(config_path, dir, LightingDir::FixtureTypes)?;
    let root = canonical_project_root(&project_root(config_path)?)?;
    let Some(source) = gdtf_source_of(&root, &dir, &name).await? else {
        return Err(Answer::new(
            StatusCode::NOT_FOUND,
            format!("Fixture type not found: {}", name),
        ));
    };
    let archive = resolve_referential_archive(&root, &name, &source)?;
    let venues_dir = resolve_lighting_dir(config_path, venues_dir, LightingDir::Venues)?;

    // Reading, parsing, distilling every mode and (cold) writing the rig are
    // all blocking work. The inner Result is the parse failure, a 400.
    let outcome = super::helpers::blocking_answer("inspect GDTF", move || {
        let bytes = crate::util::read_file(&archive)?;
        let description = match lighting::gdtf::parse_archive(&bytes) {
            Ok(description) => description,
            Err(e) => return Ok::<_, std::io::Error>(Err(e.to_string())),
        };
        let cache = lighting::distill::DistillCache::new(root.join("lighting").join(".cache"));
        // A GDTF fixture has no mode of its own: it is drawn in the first
        // mode the archive offers that distils — the body is the same in
        // every mode.
        let rig_mode = drawn_mode(&description, &name);
        // Only the type's name goes in the log: everything else here is the
        // archive's own text.
        let rig = rig_mode.as_deref().and_then(|mode| {
            match cache.ensure_rig(&bytes, mode, || Ok(&description)) {
                Ok((rig, _warnings)) => Some(rig),
                Err(_) => {
                    tracing::warn!(fixture_type = %name, "no rig model for the fixture type's details view");
                    None
                }
            }
        });
        // The thumbnail sits beside the rig file, under the archive's hash.
        let thumbnail = rig.as_deref().and_then(|rel| {
            let model = cache.load_rig(rel).ok()?;
            let file = model.thumbnail?;
            let (dir, _) = rel.rsplit_once('/')?;
            Some(format!("{dir}/{file}"))
        });
        let beam = pinned_beam(&description, rig_mode.as_deref()).map(|b| GdtfBeam {
            beam_type: b.beam_type.clone(),
            beam_angle: b.beam_angle,
            field_angle: b.field_angle,
            luminous_flux: b.luminous_flux,
            color_temperature: b.color_temperature,
            power: b.power_consumption,
        });
        // Each fixture in its own mode, spelled as the archive spells it
        // (null when it names none, or none of the archive's).
        let venues = fixtures_by_type(&venues_dir)
            .remove(&name)
            .unwrap_or_default()
            .into_iter()
            .map(|(venue, fixtures)| TypeVenue {
                name: venue,
                fixtures: fixtures
                    .into_iter()
                    .map(|(fixture, mode)| TypeVenueFixture {
                        name: fixture,
                        mode: mode.as_deref().and_then(|m| resolve_mode(&description, m)),
                    })
                    .collect(),
            })
            .collect();
        Ok(Ok(FixtureTypeGdtf {
            archive: source.path,
            rig,
            thumbnail,
            beam,
            about: description.about.clone(),
            venues,
            inspection: inspect_description(&description),
        }))
    })
    .await?;
    outcome.map_err(|e| Answer::bad(format!("Not a parseable GDTF: {}", e)))
}

/// The beam a type's facts quote: the first `Beam` in the mode's own
/// geometry tree (through its geometry references), else the archive's
/// first. A fixture's beams are usually alike, and the mode's tree is the
/// one the fixture draws.
fn pinned_beam<'a>(
    description: &'a lighting::gdtf::Description,
    mode: Option<&str>,
) -> Option<&'a lighting::gdtf::BeamData> {
    mode.and_then(|m| description.modes.iter().find(|x| x.name == m))
        .and_then(|m| first_beam_under(description, &m.geometry, 0))
        .or_else(|| description.geometries.iter().find_map(|g| g.beam.as_ref()))
}

/// The first beam at or under a top-level geometry, in document order. A
/// reference is followed to the geometry it instances, a few levels deep at
/// most (GDTF does not nest them further, and a cycle must not spin).
fn first_beam_under<'a>(
    description: &'a lighting::gdtf::Description,
    top: &str,
    depth: usize,
) -> Option<&'a lighting::gdtf::BeamData> {
    if depth > 4 {
        return None;
    }
    let geometries = &description.geometries;
    let root = geometries
        .iter()
        .position(|g| g.parent.is_none() && g.name == top)?;
    // A parent precedes its children, so one pass in order finds the
    // subtree.
    let mut inside = vec![false; geometries.len()];
    for (index, node) in geometries.iter().enumerate().skip(root) {
        if index != root && !node.parent.is_some_and(|p| inside[p]) {
            continue;
        }
        inside[index] = true;
        if let Some(beam) = &node.beam {
            return Some(beam);
        }
        if let Some(beam) = node
            .reference
            .as_deref()
            .and_then(|r| first_beam_under(description, r, depth + 1))
        {
            return Some(beam);
        }
    }
    None
}

/// The fixtures of each type across a venues directory's files (both
/// extensions), by type, then venue name, then fixture in patch order. A
/// file that does not parse, or a missing directory, contributes nothing:
/// this answers "who uses it", not whether the venues are well.
fn fixtures_by_type(dir: &std::path::Path) -> FixturesByType {
    let venues: std::collections::BTreeMap<String, lighting::types::Venue> =
        lighting::project_files::venues(dir)
            .items
            .into_iter()
            .map(|d| (d.name, d.item))
            .collect();
    let mut by_type = FixturesByType::new();
    for (venue_name, venue) in venues {
        let mut of_type: std::collections::BTreeMap<&str, Vec<(String, Option<String>)>> =
            std::collections::BTreeMap::new();
        for fixture in venue.fixtures_by_patch() {
            of_type.entry(fixture.fixture_type()).or_default().push((
                fixture.name().to_string(),
                fixture.mode().map(str::to_string),
            ));
        }
        for (fixture_type, fixtures) in of_type {
            by_type
                .entry(fixture_type.to_string())
                .or_default()
                .push((venue_name.clone(), fixtures));
        }
    }
    by_type
}

/// Type name → (venue name → (fixture name, the mode its line names)).
type FixturesByType =
    std::collections::HashMap<String, Vec<(String, Vec<(String, Option<String>)>)>>;

/// A mode as the archive spells it, matched as the lighting system matches
/// one (exact, then normalized); `None` when it names no mode of the archive.
fn resolve_mode(description: &lighting::gdtf::Description, mode: &str) -> Option<String> {
    lighting::gdtf::match_mode(description, mode)
        .ok()
        .map(|m| m.name)
}

/// The mode a GDTF fixture is drawn in (its 3D view, thumbnail and quoted
/// beam): the first mode of the archive that distils — the importer warms
/// exactly that rig.
fn drawn_mode(description: &lighting::gdtf::Description, type_name: &str) -> Option<String> {
    lighting::import::first_drivable_mode(description, type_name)
}

/// GDTF archives read and parsed once per listing, by the path a `.fixture`
/// names; `None` for one that is missing, outside the project, or does not
/// parse.
#[derive(Default)]
struct ArchiveCache {
    parsed: std::collections::HashMap<String, Option<(Vec<u8>, lighting::gdtf::Description)>>,
}

impl ArchiveCache {
    fn get(
        &mut self,
        root: &std::path::Path,
        name: &str,
        source: &lighting::types::GdtfSource,
    ) -> Option<&(Vec<u8>, lighting::gdtf::Description)> {
        self.parsed
            .entry(source.path.clone())
            .or_insert_with(|| {
                let archive = resolve_referential_archive(root, name, source).ok()?;
                let bytes = crate::util::read_file(&archive).ok()?;
                let description = lighting::gdtf::parse_archive(&bytes).ok()?;
                Some((bytes, description))
            })
            .as_ref()
    }
}

/// A referential type's card in the listing: what the fixture is, its mode
/// count, the pinned mode and its beam, and how many venue fixtures use it.
/// The thumbnail is quoted only when the rig is already in the store — a
/// listing writes nothing.
fn gdtf_list_summary(
    cache: &lighting::distill::DistillCache,
    bytes: &[u8],
    description: &lighting::gdtf::Description,
    type_name: &str,
    fixtures: &[Option<&str>],
) -> GdtfSummary {
    // The fixture is drawn, and its beam quoted, in the first mode it can
    // drive: a GDTF fixture has no mode of its own.
    let drawn = drawn_mode(description, type_name);
    let beam = pinned_beam(description, drawn.as_deref()).map(|b| BeamBrief {
        beam_type: b.beam_type.clone(),
        angle: b.beam_angle,
    });
    // The thumbnail of the rig the details view draws, if it was drawn.
    let thumbnail = drawn.and_then(|mode| {
        let rel = lighting::distill::DistillCache::rig_path(bytes, &mode);
        let rig = cache.load_rig(&rel).ok()?;
        let file = rig.thumbnail?;
        let (dir, _) = rel.rsplit_once('/')?;
        Some(format!("{dir}/{file}"))
    });
    // How many venue fixtures use each mode, by the archive's spelling (a
    // mode that resolves to none is counted as written).
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for mode in fixtures.iter().flatten() {
        let mode = resolve_mode(description, mode).unwrap_or_else(|| mode.to_string());
        *counts.entry(mode).or_default() += 1;
    }
    let mut in_use: Vec<(String, usize)> = counts.into_iter().collect();
    in_use.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    GdtfSummary {
        fixture: description.name.clone(),
        manufacturer: description.manufacturer.clone(),
        modes: description.modes.len(),
        beam,
        thumbnail,
        used_by: fixtures.len(),
        in_use: in_use
            .into_iter()
            .map(|(mode, count)| ModeUse { mode, count })
            .collect(),
    }
}

/// The channel-map form of a fixture type, as the editor's form sends it:
/// each channel's name and offset, and the strobe figures when it states
/// them. It can say no more than that, so a save of it is refused for a
/// type that holds more (`form_expresses`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ChannelMap {
    pub channels: std::collections::BTreeMap<String, u32>,
    pub max_strobe_frequency: Option<f64>,
    pub min_strobe_frequency: Option<f64>,
    pub strobe_dmx_offset: Option<u32>,
}

/// What a fixture type save writes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TypeBody {
    /// The whole file's DSL, which must declare this one type and nothing
    /// else.
    Text(String),
    /// The channel-map form.
    ChannelMap(ChannelMap),
}

/// A save of one fixture type, as a caller asks it.
pub(crate) struct TypeSaveAsk<'a> {
    pub name: &'a str,
    /// A rename's old name.
    pub from: Option<&'a str>,
    pub dir: Option<&'a str>,
    pub venues_dir: Option<&'a str>,
    /// The form asked for (`light` or `fixture`); anything else is refused.
    pub ext: Option<&'a str>,
    pub body: TypeBody,
}

/// Creates, updates or renames a fixture type.
///
/// By the one rule for type files ([`super::lighting_writes`]): a new name
/// is claimed at `<fixture types dir>/<stem>.fixture`; an existing one is
/// written only where its file is that type's alone, and refused (409, with
/// the file and why) otherwise. `from` renames, as for venues, and, as the
/// fixture settings' rename does, retypes every venue line naming the old
/// name in `venues_dir` (refused, nothing written, when any venue file
/// cannot be; the answer's `rename` says what changed). A `.light` whose
/// new text needs a `.fixture` (rich channels, a GDTF reference, or `ext`
/// `fixture`) moves to `<stem>.fixture`. There is no version to send: the
/// file is replaced as it stands, as it always was.
pub(crate) async fn put_fixture_type(
    player: &std::sync::Arc<crate::player::Player>,
    config_path: &std::path::Path,
    ask: TypeSaveAsk<'_>,
) -> Result<FixtureTypeSaved, Answer> {
    let TypeSaveAsk {
        name,
        from,
        dir,
        venues_dir,
        ext,
        body,
    } = ask;
    validate_lighting_name(name)?;
    if let Some(from) = from {
        validate_lighting_name(from)?;
    }
    let dir = resolve_lighting_dir(config_path, dir, LightingDir::FixtureTypes)?;

    let from_form = matches!(body, TypeBody::ChannelMap(_));

    // Whatever form the body is in, an `ext` naming no known form is a
    // mistake worth reporting rather than quietly ignored.
    let ext = fixture_type_extension(ext)?;

    let dsl = match body {
        TypeBody::ChannelMap(form) => channel_map_to_dsl(name, &form).map_err(Answer::bad)?,
        TypeBody::Text(text) => text,
    };

    let root = canonical_project_root(&project_root(config_path)?)?;
    let venues_dir = resolve_lighting_dir(config_path, venues_dir, LightingDir::Venues)?;
    let save = lighting_writes::TypeSave {
        root,
        dir,
        venues_dir: venues_dir.clone(),
        name: name.to_string(),
        from: from.map(str::to_string),
        ext,
        from_form,
        body: lighting_writes::Body::Text(dsl),
    };
    let rename = super::helpers::blocking_answer("write fixture type", move || {
        Ok::<_, String>(lighting_writes::save_fixture_type(save))
    })
    .await??
    .rename;
    let venue_error = match &rename {
        Some(_) => {
            super::fixture_settings::reload_types_and_venues(player, config_path, name, &venues_dir)
                .await
        }
        None => None,
    };

    Ok(FixtureTypeSaved {
        status: "saved".to_string(),
        name: name.to_string(),
        rename,
        venue_error,
    })
}

/// The form a fixture type save asks for (`?ext=`): `light` or `fixture`,
/// or none; anything else is a 400.
pub(crate) fn fixture_type_extension(
    ext: Option<&str>,
) -> Result<Option<&'static str>, lighting_writes::Answer> {
    match ext {
        None | Some("") => Ok(None),
        Some("light") => Ok(Some("light")),
        Some("fixture") => Ok(Some("fixture")),
        Some(other) => Err(lighting_writes::Answer::bad(format!(
            "Unknown fixture type extension: {}",
            other
        ))),
    }
}

/// Deletes a fixture type file: only one that is the type's alone (409
/// otherwise, with the file and why). A GDTF in the library with no record
/// is its archive, deleted.
pub(crate) async fn delete_fixture_type(
    config_path: &std::path::Path,
    name: &str,
    dir: Option<&str>,
) -> Result<FixtureTypeDeleted, Answer> {
    validate_lighting_name(name)?;
    let dir = resolve_lighting_dir(config_path, dir, LightingDir::FixtureTypes)?;
    let root = canonical_project_root(&project_root(config_path)?)?;
    let deleted = name.to_string();
    let outcome = super::helpers::blocking_answer("delete fixture type", move || {
        Ok::<_, String>(lighting_writes::delete_fixture_type(&root, &dir, &deleted))
    })
    .await??;
    Ok(FixtureTypeDeleted {
        status: "deleted".to_string(),
        name: name.to_string(),
        archive_removed: outcome.archive_removed,
        archive_kept: outcome.archive_kept,
    })
}

/// The tempo a `.light` file at this path will be loaded with, found by
/// matching the path against the loaded songs' directories.
fn song_tempo_for_path(
    player: &crate::player::Player,
    path: &std::path::Path,
) -> Option<crate::tempo::TempoMap> {
    // Both sides resolved: `path` is, and a song's folder need not be (one
    // built from a path through a linked songs folder). The plain
    // comparison first, so the folders are canonicalized only when no
    // song's matches as written.
    let songs = player.songs().list();
    let song = songs
        .iter()
        .find(|song| path.starts_with(song.base_path()))
        .or_else(|| {
            songs.iter().find(|song| {
                song.base_path()
                    .canonicalize()
                    .is_ok_and(|base| path.starts_with(base))
            })
        })?;
    song.lighting_tempo_map()
}

/// Returns the group names valid as cue targets, with the fixtures each
/// currently resolves to in the loaded venue.
///
/// These are the logical groups declared under `dmx.lighting.groups`. Venues no
/// longer define groups of their own — fixtures carry tags, and logical groups
/// select on them — so this is the only list a show can target.
pub(crate) async fn lighting_groups(
    player: &crate::player::Player,
    config_path: &std::path::Path,
) -> Result<LightingGroups, Answer> {
    let Some(system) = player
        .dmx_engine()
        .and_then(|dmx| dmx.broadcast_handles().lighting_system)
    else {
        // No DMX device configured — authoring on a laptop, which is the normal
        // case for editing a show. The groups are declared in the player config,
        // not by the engine, so the names are still knowable; only the fixtures
        // each resolves to are not.
        return groups_from_config(config_path).await;
    };

    // The lighting system's mutex is shared with the effects loop thread, and
    // resolution mutates its cache, so this belongs on the blocking pool.
    let groups = tokio::task::spawn_blocking(move || {
        let mut guard = system.lock();
        let names: Vec<String> = guard
            .logical_groups_iter()
            .map(|(n, _)| n.clone())
            .collect();
        let mut out: Vec<LightingGroup> = names
            .into_iter()
            .map(|name| LightingGroup {
                fixtures: guard.resolve_logical_group_graceful(&name),
                name,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    })
    .await;

    match groups {
        Ok(groups) => Ok(LightingGroups {
            groups,
            resolved: None,
        }),
        Err(e) => Err(Answer::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to resolve groups: {e}"),
        )),
    }
}

/// The facts behind the Lighting overview's five checks: does the show
/// reach the lights?
///
/// Facts, not verdicts. The UI turns them into ready / needs attention /
/// blocked / unknown, so the wording and the rules live in one place. Every
/// piece is something mtrack already knows: the loaded lighting system, the
/// songs' parsed shows, the same lint MCP's `validate_lighting` runs, and the
/// olad patch probe. All of it is blocking (the lighting-system mutex is shared
/// with the effects loop, the probe is HTTP), so it runs off the async worker.
pub(crate) async fn readiness(
    player: std::sync::Arc<crate::player::Player>,
) -> Result<crate::webui::wire::readiness::Readiness, Answer> {
    tokio::task::spawn_blocking(move || readiness_report(&player))
        .await
        .map_err(|e| {
            Answer::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to gather readiness: {e}"),
            )
        })
}

/// The facts and suggestions behind the "Fit shows" page: for each group the
/// shows use, what it needs and what would satisfy it; the focus points the
/// shows aim at that the venue lacks; and the universes with no output. The
/// suggestion rules live in `lighting::fit`, shared with MCP's
/// `suggest_group_tags`. Blocking (lighting-system mutex, olad probe), so off
/// the async worker.
pub(crate) async fn fit(
    player: std::sync::Arc<crate::player::Player>,
) -> Result<crate::lighting::fit::FitReport, Answer> {
    tokio::task::spawn_blocking(move || crate::lighting::fit::FitReport::gather(&player))
        .await
        .map_err(|e| {
            Answer::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to gather fit: {e}"),
            )
        })
}

/// What a song's shows would be doing at each of `times` (song seconds),
/// offline: the Preview mode's data.
///
/// Each evaluation carries the live `state` message's `fixtures`, `poses` and
/// `cells` (built by the same helper, so the 3D scene is fed one shape from
/// both sources) plus `active_effects` with the groups each cue named. The
/// fixtures no cue targets are the same for every instant and come once,
/// beside the evaluations, as `untouched`. Runs the song's registered shows
/// through `evaluate_with_system` — the function MCP's `evaluate_show` uses —
/// and never touches the DMX engine's effect engine, so a playing song plays
/// on.
pub(crate) async fn evaluate_song(
    player: &crate::player::Player,
    song: &str,
    times: &[f64],
) -> Result<crate::webui::wire::preview::Preview, Answer> {
    let error = |status: StatusCode, message: String| Answer::new(status, message);
    if times.iter().any(|t| !t.is_finite() || *t < 0.0) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "times must be non-negative seconds".to_string(),
        ));
    }

    let songs = player.songs();
    // A song whose lighting fails to parse never loads: the registry
    // keeps its failure, and the message says why.
    let song_loaded = songs.get(song)?;
    let shows = crate::lighting::evaluate::registered_shows(&song_loaded);
    if shows.is_empty() {
        return Err(error(
            StatusCode::BAD_REQUEST,
            format!("song `{song}` has no lighting loaded"),
        ));
    }
    let tempo = song_loaded.lighting_tempo_map();
    // Duration::from_secs_f64 panics on what it cannot hold; the evaluator
    // bounds the count and the latest instant itself. Collected rather than
    // sized up front: a capacity taken from the request is what CodeQL flags.
    let times: Vec<std::time::Duration> = match times
        .iter()
        .map(|t| crate::util::duration_from_secs(*t))
        .collect::<Option<Vec<_>>>()
    {
        Some(times) => times,
        None => {
            return Err(error(
                StatusCode::BAD_REQUEST,
                crate::lighting::evaluate::EvaluateError::TimeOutOfRange.to_string(),
            ))
        }
    };
    let lighting_system = player
        .dmx_engine()
        .and_then(|dmx| dmx.broadcast_handles().lighting_system);

    // The lighting system is a `parking_lot` mutex shared with the effects
    // loop, and evaluation is pure CPU besides: off the async worker.
    let evaluated = tokio::task::spawn_blocking(move || {
        crate::lighting::evaluate::evaluate_with_system(
            shows,
            tempo.as_ref(),
            &times,
            lighting_system.as_deref(),
        )
    })
    .await;
    let evaluated = match evaluated {
        Ok(Ok(evaluated)) => evaluated,
        Ok(Err(e)) => return Err(error(StatusCode::BAD_REQUEST, e.to_string())),
        Err(e) => {
            return Err(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to evaluate: {e}"),
            ))
        }
    };

    use crate::webui::wire::preview::{Preview, PreviewEffect, PreviewEvaluation};
    let untouched = evaluated
        .venue_fixtures
        .iter()
        .filter(|name| !evaluated.targeted.contains(*name))
        .cloned()
        .collect();
    let evaluations = evaluated
        .evaluations
        .iter()
        .map(|evaluation| PreviewEvaluation {
            time: evaluation.time.as_secs_f64(),
            frame: super::super::state::stage_frame(&evaluation.fixtures, &evaluation.poses),
            active_effects: evaluation
                .active_effects
                .iter()
                .map(|effect| PreviewEffect {
                    id: effect.id.clone(),
                    groups: evaluated.effect_groups.get(&effect.id).cloned(),
                    kind: effect.effect_type.to_string(),
                    layer: effect.layer.keyword().to_string(),
                    elapsed: effect.elapsed.as_secs_f64(),
                    duration: effect.duration.as_secs_f64(),
                    fixtures: effect.fixtures.clone(),
                })
                .collect(),
        })
        .collect();
    Ok(Preview {
        song: song.to_string(),
        evaluations,
        untouched,
        venue_error: evaluated.venue_error,
    })
}

/// The readiness facts for the player's running profile.
fn readiness_report(player: &crate::player::Player) -> crate::webui::wire::readiness::Readiness {
    use crate::lighting::readiness::{group_names, VenueFacts};
    use crate::webui::wire::readiness::*;
    use std::collections::BTreeSet;

    // Shows are reported with or without DMX: a laptop with no DMX output is
    // where shows are written, and a show that does not load is broken there
    // too. Everything that needs a venue or an output is empty without one.
    let dmx = player.dmx_engine();
    let configured = dmx
        .as_ref()
        .map(|d| d.configured_universes())
        .unwrap_or_default();

    // Every song with lighting, and the shows its files hold.
    let songs = player.songs();
    let with_lighting = crate::lighting::readiness::songs_with_lighting(&songs);
    let all_shows: Vec<lighting::parser::LightShow> = with_lighting
        .iter()
        .flat_map(|(_, shows)| shows.iter().cloned())
        .collect();
    let names = group_names(&all_shows);

    // The lighting system, under its lock for as short a time as it takes to
    // read what the sections below need.
    let mut facts = VenueFacts::default();
    let mut venue = None;
    let mut in_use: BTreeSet<String> = BTreeSet::new();
    let mut unresolved = Vec::new();
    let mut library = Vec::new();
    let mut used_universes: BTreeSet<u16> = BTreeSet::new();
    if let Some(system) = dmx
        .as_ref()
        .and_then(|d| d.broadcast_handles().lighting_system)
    {
        let mut guard = system.lock();
        facts = VenueFacts::collect(&mut guard, &names, Some(&configured));
        // The library's own findings: two archives that would share a name,
        // an archive that cannot be read. Neither stops a load.
        library = guard.library_findings().to_vec();
        if let Some(current) = guard.get_current_venue() {
            let mut fixtures: Vec<&lighting::types::Fixture> =
                current.fixtures().values().collect();
            fixtures.sort_by(|a, b| a.name().cmp(b.name()));
            for fixture in &fixtures {
                used_universes.insert(fixture.universe());
                match guard.fixture_problem(current.name(), fixture) {
                    None => {
                        in_use.insert(fixture.fixture_type().to_string());
                    }
                    Some(reason) => unresolved.push(UnresolvedFixture {
                        fixture: fixture.name().to_string(),
                        fixture_type: fixture.fixture_type().to_string(),
                        reason,
                    }),
                }
            }
            venue = Some(ReadinessVenue {
                name: current.name().to_string(),
                fixtures: fixtures.len(),
                placed: fixtures.iter().filter(|f| f.position().is_some()).count(),
                focus_points: current.focus_points().keys().cloned().collect(),
            });
        }
    }

    let groups: Vec<ReadinessGroup> = names
        .iter()
        .filter(|_| dmx.is_some())
        .map(|name| ReadinessGroup {
            name: name.clone(),
            fixtures: facts.group_fixture_counts.get(name).copied().unwrap_or(0),
            songs: with_lighting
                .iter()
                .filter(|(_, shows)| group_names(shows).contains(name))
                .map(|(song, _)| song.name().to_string())
                .collect(),
        })
        .collect();

    let mut shows: Vec<ReadinessShow> = with_lighting
        .iter()
        .map(|(song, shows)| {
            let warnings: Vec<_> = facts
                .lint(shows, Some(song))
                .into_iter()
                // Venue-level: the Output and Venue sections already carry
                // them, and repeating them under every song would say each
                // once per song.
                .filter(|w| {
                    !matches!(
                        w.kind,
                        "unconfigured-universe" | "patch-overlap" | "patch-overrun"
                    )
                })
                .collect();
            let files: Vec<String> = song
                .dsl_lighting_shows()
                .iter()
                .map(|dsl| {
                    let path = dsl.file_path();
                    path.strip_prefix(song.base_path())
                        .unwrap_or(path)
                        .display()
                        .to_string()
                })
                .collect();
            ReadinessShow {
                song: song.name().to_string(),
                files,
                error: None,
                warnings,
            }
        })
        .collect();
    // A song whose show does not parse never loads (`Song::new` fails as a
    // whole), so it is not in the list above: the player recorded the failure,
    // with the parser's own message, and that is the error.
    for failure in songs.failures() {
        if failure.concerns_lighting() {
            shows.push(ReadinessShow {
                song: failure.name().to_string(),
                files: Vec::new(),
                error: Some(failure.error().to_string()),
                warnings: Vec::new(),
            });
        }
    }
    shows.sort_by(|a, b| a.song.cmp(&b.song));

    let unconfigured: Vec<u16> = used_universes
        .iter()
        .copied()
        .filter(|u| !configured.contains(u))
        .collect();

    // Ask olad about the universes the venue streams to, when there are any,
    // else about everything configured. A universe with no output configured is
    // never streamed to, so asking olad about it would only repeat the
    // `unconfigured` finding. Nothing to ask means no answer, not a reachable
    // olad.
    let to_probe: Vec<u16> = if used_universes.is_empty() {
        configured.clone()
    } else {
        used_universes
            .iter()
            .copied()
            .filter(|u| configured.contains(u))
            .collect()
    };
    let olad = match &dmx {
        Some(dmx) if !to_probe.is_empty() => {
            let report = dmx.olad().probe(&to_probe);
            Some(OladPatch {
                reachable: report.reachable,
                unpatched: report.unpatched,
            })
        }
        _ => None,
    };

    // The venue's own findings: whether it loads at all (one fixture that
    // cannot be driven fails the whole venue, and nothing lights), and
    // fixtures patched over each other or past the universe's end.
    let mut report = Readiness {
        dmx: dmx.is_some(),
        venue,
        venue_error: facts.venue_error,
        patch_warnings: facts.patch_warnings,
        fixture_types: ReadinessTypes {
            in_use: in_use.into_iter().collect(),
            unresolved,
            library,
        },
        groups,
        shows,
        output: ReadinessOutput {
            universes: used_universes.into_iter().collect(),
            unconfigured,
            olad,
        },
        checks: Vec::new(),
    };
    report.checks = report.verdicts();
    report
}

/// The logical group names declared under `dmx.lighting.groups`, read from the
/// player config when no engine is running to resolve them against.
async fn groups_from_config(config_path: &std::path::Path) -> Result<LightingGroups, Answer> {
    // codeql[rust/path-injection] config_path is set at startup, not user input.
    let path = config_path.to_path_buf();
    // A parse failure is reported, not swallowed. `.ok()` here answered
    // "no groups" for a config that does not load at all — the same silent
    // empty list this fallback exists to remove, one layer down, and the reason
    // a missing `dmx.universes` looked like a rig with nothing declared.
    let loaded = tokio::task::spawn_blocking(move || {
        crate::config::Player::deserialize(&path)
            .map(|player| {
                player
                    .host_lighting()
                    .map(|lighting| {
                        let mut names: Vec<String> = lighting.groups().keys().cloned().collect();
                        names.sort();
                        names
                    })
                    .unwrap_or_default()
            })
            .map_err(|e| e.to_string())
    })
    .await;

    let names = match loaded {
        Ok(Ok(names)) => names,
        Ok(Err(e)) => {
            return Err(Answer::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "no DMX engine is running, and the player config could not be read \
                     to list the declared groups: {e}"
                ),
            ));
        }
        Err(e) => {
            return Err(Answer::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("reading the player config failed: {e}"),
            ));
        }
    };

    let groups = names
        .into_iter()
        .map(|name| LightingGroup {
            name,
            fixtures: vec![],
        })
        .collect();
    Ok(LightingGroups {
        groups,
        resolved: Some(false),
    })
}

/// Every venue of the directory `dir` (else this host's) with its file and
/// its version, and the files that did not read. A directory that is not
/// there lists nothing; one that leads outside the project is refused.
pub(crate) async fn list_venues(
    config_path: &std::path::Path,
    dir: Option<&str>,
) -> Result<VenueList, Answer> {
    let dir = resolve_lighting_dir(config_path, dir, LightingDir::Venues)?;
    if !dir.is_dir() {
        return Ok(VenueList {
            venues: Default::default(),
            owners: Default::default(),
            errors: None,
            versions: None,
        });
    }
    let all = super::helpers::blocking_answer("load venues", move || {
        // The lighting system's own reader: both extensions, the directory
        // and its subdirectories, the first file to claim a name keeps it.
        let read = lighting::project_files::venues(&dir);
        let mut all = std::collections::HashMap::new();
        let mut versions = std::collections::HashMap::new();
        let mut owners = std::collections::HashMap::new();
        for declared in read.items {
            if let Ok(bytes) = crate::util::read_file(&declared.file) {
                versions.insert(declared.name.clone(), content_version(&bytes));
            }
            if let Some(owner) = read.record.owner(&declared.name) {
                owners.insert(
                    declared.name.clone(),
                    lighting_writes::wire_owner(Kind::Venue, &dir, &owner),
                );
            }
            all.insert(declared.name, declared.item);
        }
        let errors: Vec<LightingFileError> = read
            .problems
            .iter()
            .map(|p| LightingFileError {
                file: lighting::project_files::display_in(&dir, &p.file),
                error: p.error.clone(),
            })
            .collect();
        Ok::<_, String>((all, owners, errors, versions))
    })
    .await?;
    let (all, owners, errors, versions) = all;
    Ok(VenueList {
        venues: all,
        owners,
        errors: Some(errors),
        versions: Some(versions),
    })
}

/// One venue, from the file the loader read it from, with that file's text
/// and version. A file that does not parse is never searched for it. 404
/// when no file declares `name`.
pub(crate) async fn read_venue(
    config_path: &std::path::Path,
    name: &str,
    dir: Option<&str>,
) -> Result<VenueDetail, Answer> {
    validate_lighting_name(name)?;
    let dir = resolve_lighting_dir(config_path, dir, LightingDir::Venues)?;
    let found = {
        let (dir, name) = (dir.clone(), name.to_string());
        super::helpers::blocking_answer("read venue file", move || {
            let read = lighting::project_files::venues(&dir);
            let Some(declared) = read.get(&name) else {
                return Ok(None);
            };
            crate::util::read_file(&declared.file)
                .map(|bytes| Some((declared.item.clone(), bytes)))
                .map_err(|e| format!("cannot read {}: {e}", declared.file.display()))
        })
        .await?
    };
    let Some((venue, bytes)) = found else {
        return Err(Answer::new(
            StatusCode::NOT_FOUND,
            format!("Venue not found: {}", name),
        ));
    };
    Ok(VenueDetail {
        venue,
        version: content_version(&bytes),
        dsl: String::from_utf8_lossy(&bytes).into_owned(),
    })
}

/// A venue save, as a caller asks it.
pub(crate) struct VenueSaveAsk<'a> {
    pub name: &'a str,
    /// A rename's old name.
    pub from: Option<&'a str>,
    pub dir: Option<&'a str>,
    /// The version the venue was read at; a file at another is refused (409,
    /// `conflict`).
    pub expected: Option<String>,
    pub body: VenueBody,
}

/// What a venue save writes.
pub(crate) enum VenueBody {
    /// The whole file.
    Text(String),
    /// The structured venue, which patches the file.
    Form(VenueForm),
}

/// Creates, updates or renames a venue.
///
/// By the one rule for venue files ([`super::lighting_writes`]): a new name
/// is claimed at `<venues dir>/<stem>.venue`; an existing one is written
/// only where its file is that venue's alone, and refused (409, with the
/// file and why) otherwise. `from` renames: a file at the old name's
/// canonical place moves to the new name's, any other is renamed in place.
/// A structured venue patches the file (never regenerates an existing one);
/// a text body is the whole file, and must declare this one venue and
/// nothing else. An `expected` version, when given, must be the file's
/// current one (409, `conflict`, otherwise); the answer carries the new
/// `version`.
pub(crate) async fn put_venue(
    player: &std::sync::Arc<crate::player::Player>,
    config_path: &std::path::Path,
    ask: VenueSaveAsk<'_>,
) -> Result<crate::webui::wire::venue::VenueSaved, Answer> {
    let VenueSaveAsk {
        name,
        from,
        dir,
        expected,
        body,
    } = ask;
    validate_lighting_name(name)?;
    if let Some(from) = from {
        validate_lighting_name(from)?;
    }
    let dir = resolve_lighting_dir(config_path, dir, LightingDir::Venues)?;

    let (body, needs_new_format) = match body {
        VenueBody::Form(form) => {
            let venue = venue_from_form(name, &form).map_err(Answer::bad)?;
            let needs = lighting_writes::needs_venue_extension(&venue);
            let renamed_to = name.to_string();
            let subject = from.unwrap_or(name).to_string();
            let renaming = !lighting::files::same_name(&subject, name);
            // An existing file is patched, not regenerated: comments, TODO
            // lines and the file's own layout survive a save from the UI.
            let edit: lighting_writes::Edit = Box::new(move |current| {
                let patched = match current {
                    // Patched under the name it has, then renamed.
                    Some(text) => venue_from_form(&subject, &form)
                        .and_then(|as_is| {
                            lighting::venue_patch::patch_venue(text, &subject, &as_is)
                        })
                        .and_then(|patched| {
                            if renaming {
                                lighting::venue_patch::rename_venue(&patched, &subject, &renamed_to)
                            } else {
                                Ok(patched)
                            }
                        }),
                    None => venue_form_to_dsl(&renamed_to, &form),
                };
                patched.map_err(lighting_writes::Answer::bad)
            });
            (lighting_writes::Body::Edit(edit), Some(needs))
        }
        VenueBody::Text(text) => (lighting_writes::Body::Text(text), None),
    };

    let save = lighting_writes::VenueSave {
        project: crate::util::project_dir_of(config_path),
        dir: dir.clone(),
        name: name.to_string(),
        from: from.map(str::to_string),
        expected,
        needs_new_format,
        body,
    };
    let saved = super::helpers::blocking_answer("write venue", move || {
        Ok::<_, String>(lighting_writes::save_venue(save))
    })
    .await??;
    let version = content_version(saved.text.as_bytes());

    let reloaded = reload_if_current_venue(player, name).await;
    let venue_error = venue_error_of(player, config_path, name, &dir).await;

    Ok(crate::webui::wire::venue::VenueSaved {
        status: "saved".to_string(),
        name: name.to_string(),
        reloaded,
        version,
        venue_error,
    })
}

/// After a venue save: when the saved venue is the current one and no
/// longer loads — a fixture whose type or mode cannot be driven fails the
/// whole venue, and then nothing lights — the fixture and why, as
/// `{venue, fixture, reason}`; `None` otherwise. The save stands either
/// way; this is what the editor shows beside it. Read from the running
/// engine after its reload, or, with no engine (a laptop), from the
/// project's files for the venue the config names as current. `player`'s
/// config is at `config_path`; MCP's writes answer it too.
pub(crate) async fn venue_error_of(
    player: &std::sync::Arc<crate::player::Player>,
    config_path: &std::path::Path,
    name: &str,
    venues_dir: &std::path::Path,
) -> Option<crate::lighting::findings::VenueError> {
    let player = player.clone();
    let config_path = config_path.to_path_buf();
    let venue = name.to_string();
    let venues_dir = venues_dir.to_path_buf();
    let found = tokio::task::spawn_blocking(move || {
        if let Some(system) = player.broadcast_handles().and_then(|h| h.lighting_system) {
            let guard = system.lock();
            if guard.current_venue() != Some(venue.as_str()) {
                return None;
            }
            return guard.venue_problem(&venue);
        }
        // codeql[rust/path-injection] config_path is set at startup, not user input.
        let config = crate::config::Player::deserialize(&config_path).ok()?;
        let lighting = config.host_lighting()?;
        if lighting.current_venue() != Some(venue.as_str()) {
            return None;
        }
        let root = canonical_project_root(&project_root(&config_path).ok()?).ok()?;
        let types = lighting.fixture_types_dir();
        let system = system_from_files(&root, &root.join(types), &venues_dir).ok()?;
        system.venue_problem(&venue)
    })
    .await
    .ok()
    .flatten();
    found.map(|(fixture, reason)| crate::lighting::findings::VenueError {
        venue: name.to_string(),
        fixture,
        reason,
    })
}

/// After a venue file changed on disk: if it is the venue the running engine
/// is playing against, re-read it and tell every stage watcher. Returns whether that happened. A reload failure is logged,
/// not returned — the save itself is durable and the loader will say the
/// same thing at next startup.
pub(crate) async fn reload_if_current_venue(
    player: &std::sync::Arc<crate::player::Player>,
    name: &str,
) -> bool {
    // The lighting system's lock is a blocking mutex and the reload reads
    // and parses files, so both stay off the async workers.
    let player = player.clone();
    let venue = name.to_string();
    let outcome = tokio::task::spawn_blocking(move || {
        let is_current = player
            .broadcast_handles()
            .and_then(|h| h.lighting_system)
            .is_some_and(|system| system.lock().current_venue() == Some(venue.as_str()));
        if !is_current {
            return Ok(false);
        }
        player
            .reload_current_venue()
            .map(|()| true)
            .map_err(|e| e.to_string())
    })
    .await;
    match outcome {
        Ok(Ok(reloaded)) => reloaded,
        Ok(Err(e)) => {
            tracing::warn!(venue = name, error = %e, "venue saved, but the running engine could not reload it");
            false
        }
        Err(e) => {
            tracing::warn!(venue = name, error = %e, "venue reload task failed");
            false
        }
    }
}

/// Deletes a venue file: only one that is the venue's alone (409
/// otherwise, with the file and why).
pub(crate) async fn delete_venue(
    config_path: &std::path::Path,
    name: &str,
    dir: Option<&str>,
) -> Result<(), Answer> {
    validate_lighting_name(name)?;
    let dir = resolve_lighting_dir(config_path, dir, LightingDir::Venues)?;
    let wanted = name.to_string();
    super::helpers::blocking_answer("delete venue", move || {
        Ok::<_, String>(lighting_writes::delete_venue(&dir, &wanted))
    })
    .await??;
    Ok(())
}

// ---------------------------------------------------------------------------
// Lighting helpers
// ---------------------------------------------------------------------------

/// Converts the channel-map form of a fixture type to DSL.
fn channel_map_to_dsl(name: &str, form: &ChannelMap) -> Result<String, String> {
    let mut dsl = format!("fixture_type \"{name}\" {{\n");
    dsl.push_str(&format!("  channels: {}\n", form.channels.len()));
    dsl.push_str("  channel_map: {\n");

    let mut entries: Vec<(&String, &u32)> = form.channels.iter().collect();
    entries.sort_by_key(|(_, offset)| **offset);
    // The editor's own check of its rows (`venue_rows`), its first finding
    // the refusal.
    let rows: Vec<_> = entries
        .iter()
        .map(|(ch_name, offset)| lighting::venue_rows::ChannelRow {
            name: ch_name.to_string(),
            offset: Some(f64::from(**offset)),
        })
        .collect();
    if let Some(found) = lighting::venue_rows::channel_row_problems(&rows).first() {
        use lighting::venue_rows::ChannelProblem;
        let name = &rows[found.row as usize].name;
        return Err(match found.problems[0] {
            ChannelProblem::NoName => "A channel has no name".to_string(),
            ChannelProblem::DuplicateName => format!("Channel '{name}' listed more than once"),
            ChannelProblem::BadOffset => {
                format!("Channel '{name}' offset must be from 1 to 65535")
            }
        });
    }
    for (i, (ch_name, ch_offset)) in entries.iter().enumerate() {
        let comma = if i + 1 < entries.len() { "," } else { "" };
        dsl.push_str(&format!("    \"{ch_name}\": {ch_offset}{comma}\n"));
    }
    dsl.push_str("  }\n");

    if let Some(v) = form.max_strobe_frequency {
        dsl.push_str(&format!("  max_strobe_frequency: {v}\n"));
    }
    if let Some(v) = form.min_strobe_frequency {
        dsl.push_str(&format!("  min_strobe_frequency: {v}\n"));
    }
    if let Some(v) = form.strobe_dmx_offset {
        dsl.push_str(&format!("  strobe_dmx_offset: {v}\n"));
    }

    dsl.push_str("}\n");
    Ok(dsl)
}

/// The addresses each fixture of a venue occupies, from the files, with no
/// DMX engine needed (venues are authored on laptops): each fixture's span
/// from its own mode's expansion (a native type's channels, a GDTF type's
/// mode distilled through the cache), and the venue's overlaps and overruns
/// as `lighting::patch` finds them. A fixture whose type or mode does not
/// load has a `null` footprint and is left out of the checks — never guessed
/// at. The venue inspector checks a candidate address or mode against these
/// spans before it saves. 404 for a venue the files do not declare.
pub(crate) async fn get_venue_patch(
    config_path: &std::path::Path,
    name: &str,
    dir: Option<&str>,
    fixture_types_dir: Option<&str>,
) -> Result<crate::webui::wire::venue::VenuePatch, Answer> {
    validate_lighting_name(name)?;
    let venues_dir = resolve_lighting_dir(config_path, dir, LightingDir::Venues)?;
    let types_dir =
        resolve_lighting_dir(config_path, fixture_types_dir, LightingDir::FixtureTypes)?;
    let root = canonical_project_root(&project_root(config_path)?)?;
    let name = name.to_string();
    let outcome = super::helpers::blocking_answer("read venue patch", move || {
        let system = system_from_files(&root, &types_dir, &venues_dir)?;
        let Some(venue) = system
            .venues_iter()
            .find(|(n, _)| **n == name)
            .map(|(_, v)| v)
        else {
            return Ok::<_, String>(None);
        };
        Ok(Some(venue_patch(&system, venue)))
    })
    .await?;
    outcome.ok_or_else(|| Answer::new(StatusCode::NOT_FOUND, "Venue not found"))
}

/// What the 3D view draws for a venue, read from the files with no engine
/// needed: the shape of the stage's metadata for the current venue (each
/// fixture's type, mode, position, rotation and rig; the venue's focus
/// points and scenery), so a venue that is not the current one can be looked
/// at in 3D. Loading the system distils each (archive, mode) rig into the
/// store, as the engine's load does; a fixture whose type or mode does not
/// load has no rig and is drawn generically. 404 for a venue the files do
/// not declare.
pub(crate) async fn get_venue_scene(
    config_path: &std::path::Path,
    name: &str,
    dir: Option<&str>,
    fixture_types_dir: Option<&str>,
) -> Result<VenueScene, Answer> {
    validate_lighting_name(name)?;
    let venues_dir = resolve_lighting_dir(config_path, dir, LightingDir::Venues)?;
    let types_dir =
        resolve_lighting_dir(config_path, fixture_types_dir, LightingDir::FixtureTypes)?;
    let root = canonical_project_root(&project_root(config_path)?)?;
    let name = name.to_string();
    let outcome = super::helpers::blocking_answer("read venue scene", move || {
        let system = system_from_files(&root, &types_dir, &venues_dir)?;
        let Some(venue) = system
            .venues_iter()
            .find(|(n, _)| **n == name)
            .map(|(_, v)| v)
        else {
            return Ok::<_, String>(None);
        };
        Ok(Some(venue_scene(&system, venue)))
    })
    .await?;
    outcome.ok_or_else(|| Answer::new(StatusCode::NOT_FOUND, "Venue not found"))
}

/// The scene answer for one venue of a loaded system.
fn venue_scene(
    system: &lighting::system::LightingSystem,
    venue: &lighting::types::Venue,
) -> VenueScene {
    let fixtures = venue
        .fixtures()
        .iter()
        .map(|(name, fixture)| {
            let rig = system
                .resolve_fixture_type(fixture)
                .ok()
                .and_then(|t| t.rig().map(str::to_string));
            let scene = SceneFixture {
                placement: FixturePlacement {
                    tags: fixture.tags().to_vec(),
                    fixture_type: fixture.fixture_type().to_string(),
                    position: fixture.position(),
                    rotation: fixture.rotation(),
                    beam_angle: fixture.beam_angle(),
                    rig,
                },
                mode: fixture.mode().map(String::from),
            };
            (name.clone(), scene)
        })
        .collect();
    VenueScene {
        fixtures,
        venue: super::super::state::stage_venue(system, venue),
    }
}

/// A lighting system loaded from a project's type and venue files alone, as
/// the engine would load them, with no venue made current (so the loader
/// reports nothing on its behalf).
pub(super) fn system_from_files(
    root: &std::path::Path,
    types_dir: &std::path::Path,
    venues_dir: &std::path::Path,
) -> Result<lighting::system::LightingSystem, String> {
    let display = |p: &std::path::Path| p.to_string_lossy().into_owned();
    let config = crate::config::Lighting::new(
        None,
        None,
        Some(crate::config::lighting::Directories::new(
            types_dir.is_dir().then(|| display(types_dir)),
            venues_dir.is_dir().then(|| display(venues_dir)),
        )),
    );
    let mut system = lighting::system::LightingSystem::new();
    system.load(&config, root).map_err(|e| e.to_string())?;
    Ok(system)
}

/// The patch answer for one venue of a loaded system.
fn venue_patch(
    system: &lighting::system::LightingSystem,
    venue: &lighting::types::Venue,
) -> crate::webui::wire::venue::VenuePatch {
    use crate::webui::wire::venue::{PatchedFixture, VenuePatch};
    let spans = venue
        .fixtures_by_patch()
        .into_iter()
        .map(|fixture| PatchedFixture {
            fixture: fixture.name().to_string(),
            universe: fixture.universe(),
            address: fixture.start_channel(),
            footprint: system
                .resolve_fixture_type(fixture)
                .ok()
                .map(|t| t.footprint()),
            fixture_type: fixture.fixture_type().to_string(),
            mode: fixture.mode().map(String::from),
        })
        .collect();
    let known = system.patch_spans(venue);
    VenuePatch {
        spans,
        overlaps: lighting::patch::venue_overlaps(&known),
        overruns: lighting::patch::venue_overruns(&known),
    }
}

/// A venue as a caller structures it, to be saved.
#[derive(Clone, Debug, Default)]
pub(crate) struct VenueForm {
    pub fixtures: Vec<FormFixture>,
    pub focus_points: std::collections::BTreeMap<String, lighting::types::Vec3>,
    pub source: Option<lighting::types::VenueSource>,
}

/// A fixture of a [`VenueForm`].
#[derive(Clone, Debug, Default)]
pub(crate) struct FormFixture {
    pub name: String,
    pub fixture_type: String,
    pub universe: u32,
    pub start_channel: u32,
    pub tags: Vec<String>,
    pub position: Option<lighting::types::Vec3>,
    pub rotation: Option<lighting::types::Vec3>,
    pub mode: Option<String>,
    pub beam_angle: Option<f64>,
}

/// Converts a structured venue definition to DSL format.
fn venue_form_to_dsl(name: &str, form: &VenueForm) -> Result<String, String> {
    venue_from_form(name, form).map(|venue| format!("{venue}\n"))
}

/// Builds the venue a structured save describes.
fn venue_from_form(name: &str, form: &VenueForm) -> Result<lighting::types::Venue, String> {
    use lighting::types::{Fixture, Vec3, Venue};

    /// A point whose figures are all finite numbers.
    fn finite(v: Vec3, what: &str) -> Result<Vec3, String> {
        if v.iter().all(|x| x.is_finite()) {
            Ok(v)
        } else {
            Err(format!("{what} must be [x, y, z] numbers"))
        }
    }
    fn optional_vec3(v: Option<Vec3>, what: &str) -> Result<Option<Vec3>, String> {
        v.map(|v| finite(v, what)).transpose()
    }

    let rows: Vec<_> = form
        .fixtures
        .iter()
        .map(|fix| lighting::venue_rows::VenueRow {
            name: fix.name.clone(),
            fixture_type: fix.fixture_type.clone(),
            universe: Some(f64::from(fix.universe)),
            start_channel: Some(f64::from(fix.start_channel)),
            mode: None,
        })
        .collect();
    // A row the editor could not name or address is refused, not saved as
    // a fixture nobody can address (or dropped without a word), by the
    // editor's own check of its rows (`venue_rows`).
    if let Some(found) = lighting::venue_rows::venue_row_problems(&rows, |_| false).first() {
        use lighting::venue_rows::RowProblem;
        let row = &rows[found.row as usize];
        let name = &row.name;
        return Err(match found.problems[0] {
            RowProblem::NoName => "A fixture has no name".to_string(),
            RowProblem::DuplicateName => format!("Fixture '{name}' listed more than once"),
            RowProblem::NoType | RowProblem::NoMode => {
                format!("Fixture '{name}' has no fixture type")
            }
            RowProblem::BadUniverse if row.universe == Some(0.0) => {
                format!("Fixture '{name}': universe must be 1 or more")
            }
            RowProblem::BadUniverse => "Fixture 'universe' out of range".to_string(),
            RowProblem::BadAddress if row.start_channel == Some(0.0) => {
                format!("Fixture '{name}': start channel must be 1 or more")
            }
            RowProblem::BadAddress => "Fixture 'start_channel' out of range".to_string(),
        });
    }

    let mut by_name = std::collections::HashMap::new();
    for (fix, row) in form.fixtures.iter().zip(&rows) {
        let (fix_name, fix_type) = (row.name.as_str(), row.fixture_type.as_str());
        let fixture = Fixture::new(
            fix_name.to_string(),
            fix_type.to_string(),
            // Both in 1..=65535, by the check above.
            row.universe.unwrap_or_default() as u16,
            row.start_channel.unwrap_or_default() as u16,
            fix.tags.clone(),
        )
        .with_position(optional_vec3(fix.position, "position")?)
        .with_rotation(optional_vec3(fix.rotation, "rotation")?)
        // What the unit really throws, in degrees (a drawing hint); absent
        // is the type's own. Dropping it here would silently lose it.
        .with_beam_angle(match fix.beam_angle {
            None => None,
            Some(angle) => Some(
                Some(angle)
                    .filter(|v| lighting::fixture_data::beam_angle_valid(*v))
                    .ok_or_else(|| {
                        format!(
                            "Fixture '{fix_name}': 'beam_angle' must be a number of degrees, \
                             greater than 0 and at most {}, or null",
                            lighting::fixture_data::MAX_BEAM_ANGLE
                        )
                    })?,
            ),
        })
        // The fixture's own GDTF mode (§21); absent is the type's default.
        // Dropping it here would silently re-mode the fixture.
        .with_mode(match &fix.mode {
            None => None,
            Some(mode) => Some(
                Some(mode.as_str())
                    .filter(|m| !m.trim().is_empty())
                    .ok_or("Fixture 'mode' must be a non-empty string or null")?
                    .to_string(),
            ),
        });
        by_name.insert(fix_name.to_string(), fixture);
    }

    let mut focus_points = std::collections::BTreeMap::new();
    for (focus_name, point) in &form.focus_points {
        focus_points.insert(focus_name.clone(), finite(*point, "focus point")?);
    }
    let source = match &form.source {
        None => None,
        Some(source) => Some(lighting::types::VenueSource {
            mvr: source.mvr.clone(),
            origin: finite(source.origin, "source origin")?,
        }),
    };

    Ok(Venue::new(name.to_string(), by_name)
        .with_focus_points(focus_points)
        .with_source(source))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn a_path_is_within_a_dir_by_identity() {
        let dir = tempfile::tempdir().unwrap();
        let venues = dir.path().join("lighting/venues");
        std::fs::create_dir_all(&venues).unwrap();
        std::fs::create_dir_all(dir.path().join("other")).unwrap();
        std::os::unix::fs::symlink(&venues, dir.path().join("alias")).unwrap();
        let within = |p: &str| crate::util::is_within_dir(&dir.path().join(p), &venues);
        assert!(within("lighting/venues/new.light"));
        assert!(within("lighting/venues/deep/er/new.light"));
        assert!(within("other/../lighting/venues/new.light"));
        assert!(within("alias/new.light"));
        assert!(!within("other/new.light"));
        assert!(!within("lighting/new.light"));
        assert!(!crate::util::is_within_dir(
            &dir.path().join("x"),
            &dir.path().join("missing")
        ));
    }

    #[test]
    fn find_light_files_discovers_files() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        std::fs::create_dir(base.join("song1")).unwrap();
        std::fs::write(base.join("song1/show.light"), "content").unwrap();
        std::fs::write(base.join("top.light"), "content").unwrap();
        std::fs::write(base.join("not_a_light.txt"), "content").unwrap();

        let mut results = Vec::new();
        find_light_files(base, base, &mut results).unwrap();

        assert_eq!(results.len(), 2);
        let paths: Vec<&str> = results.iter().map(|r| r.path.as_str()).collect();
        assert!(paths.contains(&"song1/show.light"));
        assert!(paths.contains(&"top.light"));
    }

    /// A light show's extension is read in any case: `Show.LIGHT` is listed,
    /// as `songs --init` registers it.
    #[test]
    fn find_light_files_reads_extensions_in_any_case() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Show.LIGHT"), "content").unwrap();
        std::fs::write(dir.path().join("show.lights"), "content").unwrap();

        let mut results = Vec::new();
        find_light_files(dir.path(), dir.path(), &mut results).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "Show.LIGHT");
        assert_eq!(results[0].name, "Show");
    }

    #[test]
    fn find_light_files_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut results = Vec::new();
        find_light_files(dir.path(), dir.path(), &mut results).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn find_light_files_extracts_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("my_show.light"), "content").unwrap();

        let mut results = Vec::new();
        find_light_files(dir.path(), dir.path(), &mut results).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "my_show");
    }

    #[test]
    fn find_light_files_deeply_nested() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        let deep_dir = base.join("a").join("b").join("c");
        std::fs::create_dir_all(&deep_dir).unwrap();
        std::fs::write(deep_dir.join("deep.light"), "content").unwrap();

        let mut results = Vec::new();
        find_light_files(base, base, &mut results).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "a/b/c/deep.light");
    }

    #[test]
    fn find_light_files_nonexistent_dir() {
        let results_vec = &mut Vec::new();
        let result = find_light_files(
            std::path::Path::new("/nonexistent"),
            std::path::Path::new("/nonexistent"),
            results_vec,
        );
        assert!(result.is_ok());
        assert!(results_vec.is_empty());
    }

    #[test]
    fn find_light_files_multiple_extensions_only_light() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        std::fs::write(base.join("show.light"), "content").unwrap();
        std::fs::write(base.join("show.yaml"), "content").unwrap();
        std::fs::write(base.join("show.txt"), "content").unwrap();
        std::fs::write(base.join("show.mid"), "content").unwrap();

        let mut results = Vec::new();
        find_light_files(base, base, &mut results).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "show");
    }

    // -----------------------------------------------------------------------
    // Unit tests: validate_lighting_name
    // -----------------------------------------------------------------------

    #[test]
    fn validate_lighting_name_valid() {
        assert!(validate_lighting_name("my-fixture").is_ok());
        assert!(validate_lighting_name("Venue_01").is_ok());
        assert!(validate_lighting_name("simple").is_ok());
    }

    #[test]
    fn validate_lighting_name_rejects_route_segments() {
        for name in ["lock", "validate", "activate"] {
            assert!(validate_lighting_name(name).is_err(), "{name}");
        }
        assert!(validate_lighting_name("locker").is_ok());
    }

    #[test]
    fn validate_lighting_name_invalid_empty() {
        assert!(validate_lighting_name("").is_err());
    }

    #[test]
    fn validate_lighting_name_invalid_dots() {
        assert!(validate_lighting_name("..").is_err());
        assert!(validate_lighting_name("a/../b").is_err());
    }

    #[test]
    fn validate_lighting_name_invalid_slashes() {
        assert!(validate_lighting_name("a/b").is_err());
        assert!(validate_lighting_name("a\\b").is_err());
    }

    #[test]
    fn validate_lighting_name_invalid_null() {
        assert!(validate_lighting_name("a\0b").is_err());
    }

    // -----------------------------------------------------------------------
    // Unit tests: fixture_type_json_to_dsl
    // -----------------------------------------------------------------------

    #[test]
    fn channel_map_to_dsl_basic() {
        let form = ChannelMap {
            channels: [("red", 1), ("green", 2), ("blue", 3)]
                .into_iter()
                .map(|(n, o)| (n.to_string(), o))
                .collect(),
            ..Default::default()
        };
        let dsl = channel_map_to_dsl("TestFixture", &form).unwrap();
        assert!(dsl.contains("fixture_type \"TestFixture\""));
        assert!(dsl.contains("channels: 3"));
        assert!(dsl.contains("\"red\": 1"));
        assert!(dsl.contains("\"green\": 2"));
        assert!(dsl.contains("\"blue\": 3"));
        // Verify the DSL actually parses.
        let types = lighting::parser::parse_fixture_types(&dsl).unwrap();
        assert!(types.contains_key("TestFixture"));
    }

    #[test]
    fn channel_map_to_dsl_with_strobe() {
        let form = ChannelMap {
            channels: [("dimmer", 1), ("strobe", 2)]
                .into_iter()
                .map(|(n, o)| (n.to_string(), o))
                .collect(),
            max_strobe_frequency: Some(25.0),
            min_strobe_frequency: Some(0.5),
            strobe_dmx_offset: Some(10),
        };
        let dsl = channel_map_to_dsl("StrobeLight", &form).unwrap();
        assert!(dsl.contains("max_strobe_frequency: 25"));
        assert!(dsl.contains("min_strobe_frequency: 0.5"));
        assert!(dsl.contains("strobe_dmx_offset: 10"));
        // Verify the DSL actually parses.
        let types = lighting::parser::parse_fixture_types(&dsl).unwrap();
        let ft = types.get("StrobeLight").unwrap();
        assert_eq!(ft.max_strobe_frequency(), Some(25.0));
    }

    // -----------------------------------------------------------------------
    // Unit tests: venue_form_to_dsl
    // -----------------------------------------------------------------------

    /// A fixture row of a venue form: `name`, `fixture_type`, 1:`start_channel`.
    fn form_fixture(name: &str, fixture_type: &str, start_channel: u32) -> FormFixture {
        FormFixture {
            name: name.to_string(),
            fixture_type: fixture_type.to_string(),
            universe: 1,
            start_channel,
            ..Default::default()
        }
    }

    fn form_of(fixtures: Vec<FormFixture>) -> VenueForm {
        VenueForm {
            fixtures,
            ..Default::default()
        }
    }

    #[test]
    fn venue_form_to_dsl_basic() {
        let form = form_of(vec![form_fixture("Spot1", "GenericPar", 1)]);
        let dsl = venue_form_to_dsl("TestVenue", &form).unwrap();
        assert!(dsl.contains("venue \"TestVenue\""));
        assert!(dsl.contains("fixture \"Spot1\" GenericPar @ 1:1"));
        // Verify the DSL actually parses.
        let venues = lighting::parser::parse_venues(&dsl).unwrap();
        assert!(venues.contains_key("TestVenue"));
    }

    #[test]
    fn venue_form_to_dsl_carries_beam_angle() {
        let venue = |beam: Option<f64>| {
            form_of(vec![FormFixture {
                rotation: Some([60.0, 0.0, 0.0]),
                beam_angle: beam,
                ..form_fixture("Brick1", "Brick", 1)
            }])
        };
        let dsl = venue_form_to_dsl("V", &venue(Some(60.0))).unwrap();
        assert!(
            dsl.contains("rotation (60, 0, 0) beam_angle 60\n"),
            "after rotation, no trailing .0: {dsl}"
        );
        let dsl = venue_form_to_dsl("V", &venue(Some(22.5))).unwrap();
        assert!(dsl.contains("beam_angle 22.5"), "{dsl}");

        let dsl = venue_form_to_dsl("V", &venue(None)).unwrap();
        assert!(!dsl.contains("beam_angle"), "{dsl}");

        for bad in [0.0, -3.0, 181.0, f64::NAN, f64::INFINITY] {
            let err = venue_form_to_dsl("V", &venue(Some(bad))).unwrap_err();
            assert!(
                err.contains("'beam_angle'") && err.contains("Brick1"),
                "{bad}: {err}"
            );
        }
    }

    #[test]
    fn a_venue_with_a_beam_angle_needs_the_venue_extension() {
        let venue = lighting::parser::parse_venues(
            "venue \"v\" {\n  fixture \"A\" T @ 1:1 beam_angle 60\n}\n",
        )
        .unwrap()
        .remove("v")
        .unwrap();
        assert!(lighting_writes::needs_venue_extension(&venue));
    }

    #[test]
    fn venue_form_to_dsl_with_tags() {
        let form = form_of(vec![FormFixture {
            tags: vec!["front".to_string(), "wash".to_string()],
            ..form_fixture("Wash1", "Par", 1)
        }]);
        let dsl = venue_form_to_dsl("Tagged", &form).unwrap();
        assert!(dsl.contains("tags [\"front\", \"wash\"]"));
        // Verify the DSL actually parses.
        let venues = lighting::parser::parse_venues(&dsl).unwrap();
        let v = venues.get("Tagged").unwrap();
        let w1 = v.fixtures().get("Wash1").unwrap();
        assert_eq!(w1.tags(), &["front", "wash"]);
    }

    #[test]
    fn venue_form_to_dsl_emits_no_groups() {
        // Venue groups are gone: a form has none to send, and the file the
        // parser reads back has none.
        let form = form_of(vec![form_fixture("L1", "Par", 1)]);
        let dsl = venue_form_to_dsl("Grouped", &form).unwrap();
        assert!(
            !dsl.contains("group"),
            "groups should not be emitted: {dsl}"
        );
        lighting::parser::parse_venues(&dsl).expect("emitted DSL must parse");
    }

    #[test]
    fn venue_form_to_dsl_refuses_a_fixture_with_no_name() {
        let form = form_of(vec![form_fixture("", "Par", 1)]);
        assert!(venue_form_to_dsl("Bad", &form)
            .unwrap_err()
            .contains("no name"));
    }

    #[test]
    fn venue_form_to_dsl_refuses_a_figure_that_is_not_a_number() {
        let at = |position, rotation| {
            form_of(vec![FormFixture {
                position,
                rotation,
                ..form_fixture("A", "Par", 1)
            }])
        };
        let nan = [f64::NAN, 0.0, 0.0];
        assert!(venue_form_to_dsl("V", &at(Some(nan), None))
            .unwrap_err()
            .contains("position"));
        assert!(venue_form_to_dsl("V", &at(None, Some(nan)))
            .unwrap_err()
            .contains("rotation"));
        let mut form = at(None, None);
        form.focus_points.insert("drummer".to_string(), nan);
        assert!(venue_form_to_dsl("V", &form)
            .unwrap_err()
            .contains("focus point"));
        let mut form = at(None, None);
        form.source = Some(lighting::types::VenueSource {
            mvr: "a.mvr".to_string(),
            origin: [f64::INFINITY, 0.0, 0.0],
        });
        assert!(venue_form_to_dsl("V", &form)
            .unwrap_err()
            .contains("source origin"));
    }

    #[test]
    fn venue_form_to_dsl_refuses_a_blank_mode() {
        for mode in ["", "  "] {
            let form = form_of(vec![FormFixture {
                mode: Some(mode.to_string()),
                ..form_fixture("A", "Brick", 1)
            }]);
            assert!(venue_form_to_dsl("V", &form)
                .unwrap_err()
                .contains("'mode'"));
        }
    }

    fn synthetic_gdtf_bytes() -> Vec<u8> {
        crate::lighting::gdtf::build_zip(&[(
            "description.xml",
            crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
        )])
    }

    // -----------------------------------------------------------------------
    // A GDTF's modes, as the fixture page shows them
    // -----------------------------------------------------------------------

    async fn inspect(bytes: &[u8]) -> serde_json::Value {
        json!(inspect_description(
            &lighting::gdtf::parse_archive(bytes).unwrap()
        ))
    }

    #[tokio::test]
    async fn inspect_gdtf_says_what_each_mode_can_do() {
        let parsed = inspect(&synthetic_gdtf_bytes()).await;
        let modes = parsed["modes"].as_array().unwrap();
        let rgbs = modes.iter().find(|m| m["name"] == "8: RGBS").unwrap();
        // RGB and a strobe, and a dimmer only as a virtual channel.
        assert_eq!(rgbs["capabilities"], json!(["color", "strobe"]), "{rgbs}");
        assert_eq!(rgbs["strobe_range"]["min_hz"], json!(0.4), "{rgbs}");
        assert_eq!(rgbs["strobe_range"]["max_hz"], json!(25.0), "{rgbs}");
        assert_eq!(rgbs["cells"], json!(0));
        assert!(rgbs.get("refused").is_none());
        assert!(rgbs["channels"]
            .as_array()
            .unwrap()
            .contains(&json!([1, "red"])));
        assert!(rgbs.get("warnings").is_none(), "nothing reads them");

        let mover = modes.iter().find(|m| m["name"] == "Mover 16bit").unwrap();
        assert_eq!(mover["capabilities"], json!(["pan_tilt"]), "{mover}");
        assert!(mover.get("strobe_range").is_none());
        assert_eq!(mover["footprint"], json!(5));
    }

    #[tokio::test]
    async fn inspect_gdtf_answers_every_mode_of_a_many_mode_archive() {
        // A big real GDTF has dozens of modes; all of them are distilled (off
        // the async worker) and reported.
        let xml = crate::lighting::gdtf::SYNTHETIC_DESCRIPTION;
        let start = xml.find("<DMXMode Name=\"8: RGBS\"").unwrap();
        let end = start + xml[start..].find("</DMXMode>").unwrap() + "</DMXMode>".len();
        let template = &xml[start..end];
        let many: String = (1..=30)
            .map(|n| template.replace("8: RGBS", &format!("Mode {n}")))
            .collect::<Vec<_>>()
            .join("\n");
        let xml = format!("{}{many}{}", &xml[..start], &xml[end..]);
        let bytes = crate::lighting::gdtf::build_zip(&[("description.xml", xml.as_bytes())]);

        let parsed = inspect(&bytes).await;
        let modes = parsed["modes"].as_array().unwrap();
        for n in 1..=30 {
            let mode = modes
                .iter()
                .find(|m| m["name"] == format!("Mode {n}").as_str())
                .unwrap_or_else(|| panic!("Mode {n} missing from {modes:?}"));
            assert_eq!(mode["capabilities"], json!(["color", "strobe"]), "{mode}");
            assert!(mode.get("refused").is_none(), "{mode}");
        }
        assert!(modes.iter().any(|m| m["name"] == "Mover 16bit"));
        assert_eq!(modes.len(), 31);
    }

    #[tokio::test]
    async fn inspect_gdtf_refuses_a_mode_it_cannot_tell_from_another() {
        // Two modes with one name: an import takes the first, so the second
        // is listed with its reason, not offered.
        let xml = crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.replace("Mover 16bit", "8: RGBS");
        let bytes = crate::lighting::gdtf::build_zip(&[("description.xml", xml.as_bytes())]);
        let parsed = inspect(&bytes).await;
        let modes = parsed["modes"].as_array().unwrap();
        assert_eq!(modes.len(), 2);
        assert!(modes[0].get("refused").is_none());
        assert!(
            modes[1]["refused"]
                .as_str()
                .unwrap()
                .contains("another mode"),
            "{modes:?}"
        );
        assert!(modes[1].get("capabilities").is_none());
    }

    #[test]
    fn a_suggested_type_name_is_safe_in_a_fixture_file() {
        assert_eq!(
            suggested_type_name("Spiider \"Pro\" {v2}"),
            "Spiider Pro v2"
        );
        assert_eq!(suggested_type_name("  A   B "), "A B");
        assert_eq!(suggested_type_name("\"\"{}"), "fixture");
    }
}
