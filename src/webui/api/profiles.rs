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

//! The profile files of the hardware profiles directory, read, listed, saved
//! and deleted: what `ConfigService`'s profile file RPCs call. Plain
//! functions over a directory and the player (no web types); a refusal is a
//! [`ProfileFileError`], which the RPC maps to a status.

use std::path::Path;
use std::sync::Arc;

use tracing::warn;

use super::super::config_io;
use super::super::safe_path::{SafePath, SafePathError, VerifiedRoot};
use super::lighting_api::content_version;
use crate::config::Profile;
use crate::player::{ConfigChange, Player};
use crate::webui::wire::config::{ProfileFile, ProfileFileInfo};
use config::Config;

/// Why a profile file call was refused or failed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProfileFileError {
    /// The name or the profile is not valid; the message says what is wrong.
    Invalid(String),
    /// What was named is not there.
    NotFound(String),
    /// A path that leaves the profiles directory.
    Forbidden(String),
    /// The file is not at the version the save read it at. `current` is the
    /// version it is at; none when there is no such file.
    Changed {
        message: String,
        current: Option<String>,
    },
    /// A create reached a file already there (or one made since the lookup).
    Exists(String),
    /// A new name that differs from a file there only in case.
    CaseClash(String),
    /// mtrack's own fault, or the filesystem's.
    Internal(String),
}

impl std::fmt::Display for ProfileFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Changed { message, .. }
            | Self::Invalid(message)
            | Self::NotFound(message)
            | Self::Forbidden(message)
            | Self::Exists(message)
            | Self::CaseClash(message)
            | Self::Internal(message) => write!(f, "{message}"),
        }
    }
}

impl From<SafePathError> for ProfileFileError {
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

/// A profile file saved: the version it is at now, and what became of the
/// hardware reload.
pub(crate) struct ProfileSaved {
    pub(crate) version: String,
    pub(crate) reload: ConfigChange,
}

/// Validates a profile filename for use in file paths.
/// Serialises every read-check-write of a profile file, here and in the
/// config store's writes of the profiles it loaded from a directory.
pub(crate) static PROFILE_WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A YAML document as JSON, for merging. `None` when it does not parse.
fn yaml_to_json(text: &str) -> Option<serde_json::Value> {
    use yaml_rust2::{Yaml, YamlLoader};
    fn convert(y: &Yaml) -> serde_json::Value {
        match y {
            Yaml::Null | Yaml::BadValue | Yaml::Alias(_) => serde_json::Value::Null,
            Yaml::Boolean(b) => (*b).into(),
            Yaml::Integer(i) => (*i).into(),
            Yaml::Real(r) => r
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map_or_else(|| r.clone().into(), serde_json::Value::Number),
            Yaml::String(s) => s.clone().into(),
            Yaml::Array(a) => a.iter().map(convert).collect(),
            Yaml::Hash(h) => h
                .iter()
                .filter_map(|(k, v)| {
                    let key = match k {
                        Yaml::String(s) => s.clone(),
                        Yaml::Integer(i) => i.to_string(),
                        Yaml::Real(r) => r.clone(),
                        Yaml::Boolean(b) => b.to_string(),
                        _ => return None,
                    };
                    Some((key, convert(v)))
                })
                .collect(),
        }
    }
    YamlLoader::load_from_str(text).ok()?.first().map(convert)
}

/// Copies into `new` the keys of the existing profile file that the typed
/// profile does not know (a key this version does not model, or one added by
/// hand), so a save from the UI does not silently drop them. Keys are
/// compared case-insensitively, as the profile loader reads them. Only
/// mappings are merged; list items are rewritten as the UI sent them.
/// Comments are not carried: the YAML crate does not keep them.
fn carry_unknown_keys(old_bytes: &[u8], new: &mut serde_json::Value) {
    let Some(text) = std::str::from_utf8(old_bytes).ok() else {
        return;
    };
    let Some(raw) = yaml_to_json(text) else {
        return;
    };
    let typed = Config::builder()
        .add_source(config::File::from_str(text, config::FileFormat::Yaml))
        .build()
        .and_then(|c| c.try_deserialize::<Profile>())
        .ok()
        .and_then(|p| serde_json::to_value(&p).ok());
    let Some(typed) = typed else { return };
    carry(&raw, &typed, new);
}

/// A file a test makes at its path between a save's lookup and its write,
/// as another writer racing the save would.
#[cfg(test)]
pub(crate) static RACE: std::sync::Mutex<Option<(std::path::PathBuf, &'static str)>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
fn race_before_write(path: &std::path::Path) {
    let mut race = RACE.lock().unwrap();
    if race.as_ref().is_some_and(|(at, _)| at == path) {
        let (at, text) = race.take().unwrap();
        std::fs::write(at, text).unwrap();
    }
}

/// The profile currently in `filename`, if there is one and it parses.
async fn existing_profile(profiles_dir: &std::path::Path, filename: &str) -> Option<Profile> {
    let path = profile_path(profiles_dir, filename)
        .ok()
        .filter(|file| file.exists())?
        .path()
        .to_path_buf();
    tokio::task::spawn_blocking(move || {
        Config::builder()
            .add_source(crate::util::yaml_source(&path).ok()?)
            .build()
            .and_then(|c| c.try_deserialize::<Profile>())
            .ok()
    })
    .await
    .ok()
    .flatten()
}

fn carry(raw: &serde_json::Value, typed: &serde_json::Value, new: &mut serde_json::Value) {
    let (Some(raw), Some(typed), Some(new)) =
        (raw.as_object(), typed.as_object(), new.as_object_mut())
    else {
        return;
    };
    for (key, value) in raw {
        let lower = key.to_lowercase();
        match typed.iter().find(|(k, _)| k.to_lowercase() == lower) {
            None => {
                if !new.keys().any(|k| k.to_lowercase() == lower) {
                    new.insert(key.clone(), value.clone());
                }
            }
            Some((typed_key, typed_value)) => {
                if let Some(slot) = new.get_mut(typed_key) {
                    carry(value, typed_value, slot);
                }
            }
        }
    }
}

/// The file in `profiles_dir` that `filename` addresses
/// (`config::profile_file`: a full file name, `stage.yml`, is that file; a
/// stem, `stage`, its `.yaml`, else its `.yml`, else a new `.yaml`), checked
/// to be inside the directory: every read, save and delete here goes to it.
/// A new file named as one there but for case is refused.
fn profile_path(
    profiles_dir: &Path,
    filename: &str,
) -> Result<crate::util::NamedFile, ProfileFileError> {
    let file = crate::config::profile_file(profiles_dir, filename)
        .map_err(|e| ProfileFileError::CaseClash(e.to_string()))?;
    let root = VerifiedRoot::new(profiles_dir)?;
    let joined = match file.path().file_name() {
        Some(file_name) => root.as_path().join(file_name),
        None => file.path().to_path_buf(),
    };
    if file.exists() {
        let safe = SafePath::resolve(&joined, &root)?;
        Ok(file.with_path(safe.as_path().to_path_buf()))
    } else {
        Ok(file.with_path(joined))
    }
}

/// A name that is safe to use in a path.
fn validate_profile_filename(name: &str) -> Result<(), ProfileFileError> {
    SafePath::validate_name(name)
        .map_err(|_| ProfileFileError::Invalid("Invalid profile name".to_string()))
}

/// Runs a blocking filesystem operation off the async runtime.
async fn blocking_io<F, T, E>(label: &str, f: F) -> Result<T, ProfileFileError>
where
    F: FnOnce() -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(ProfileFileError::Internal(format!(
            "Failed to {label}: {e}"
        ))),
        Err(e) => Err(ProfileFileError::Internal(format!(
            "{label} task failed: {e}"
        ))),
    }
}

/// The answer to a write of `path` (`config_io::write`): a create beaten to
/// the file by one already there is `Exists`; another failure is the
/// filesystem's.
fn written(result: std::io::Result<()>, path: &Path) -> Result<(), ProfileFileError> {
    match result {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(ProfileFileError::Exists(format!(
                "a profile file `{}` already exists",
                crate::util::filename_display(path)
            )))
        }
        Err(e) => Err(ProfileFileError::Internal(
            crate::util::write_failure_message(crate::util::WriteTarget::File(path), &e),
        )),
    }
}

/// Makes the profiles directory before anything canonicalizes it, only when
/// it is inside `project`: a `profiles_dir:` pointing elsewhere is the
/// operator's to set up. Canonicalizing fails on a directory that is not
/// there, so a creation placed after it could never run.
async fn ensure_dir(dir: &Path, project: &Path) -> Result<std::path::PathBuf, ProfileFileError> {
    let (dir, project) = (dir.to_path_buf(), project.to_path_buf());
    match tokio::task::spawn_blocking(move || crate::util::create_dir_within(&dir, &project)).await
    {
        Ok(Ok(created)) => Ok(created.into_path_buf()),
        Ok(Err(e @ crate::util::CreateWithinError::Io { .. })) => {
            Err(ProfileFileError::Internal(e.to_string()))
        }
        Ok(Err(e)) => Err(ProfileFileError::Invalid(e.to_string())),
        Err(e) => Err(ProfileFileError::Internal(format!(
            "Failed to create directory: {e}"
        ))),
    }
}

/// The profile files of `profiles_dir` the config loads, with the sections
/// each has, sorted by stem then file name. A file that does not parse is
/// listed, with why.
pub(crate) async fn list(profiles_dir: &Path) -> Result<Vec<ProfileFileInfo>, ProfileFileError> {
    let profiles_dir = profiles_dir.to_path_buf();
    // codeql[rust/path-injection] profiles_dir comes from server config, not user input.
    blocking_io("read profiles dir", move || {
        // The profile files the config loads, as it lists them.
        let mut items: Vec<ProfileFileInfo> = Vec::new();
        for path in crate::config::list_profile_files(&profiles_dir)? {
            // A name the API cannot take back as written is listed, but
            // not as one to act on: a lossy name would reach another file.
            let name = path.file_name().unwrap_or_default();
            let file = name.to_string_lossy().into_owned();
            let addressable = name
                .to_str()
                .is_some_and(|name| SafePath::validate_name(name).is_ok());
            let filename = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            // Parse the profile. One that does not parse is still a file
            // the lookups reach, so it is listed, with why.
            let parsed = crate::util::yaml_source(&path)
                .and_then(|source| Config::builder().add_source(source).build())
                .and_then(|c| c.try_deserialize::<Profile>());
            let (profile, error) = match parsed {
                Ok(profile) => (Some(profile), None),
                Err(e) => (None, Some(e.to_string())),
            };
            let profile = profile.as_ref();

            items.push(ProfileFileInfo {
                file,
                addressable,
                error,
                hostname: profile.and_then(|p| p.hostname()).map(str::to_string),
                has_audio: profile.is_some_and(|p| p.audio_config().is_some()),
                has_midi: profile.is_some_and(|p| p.midi().is_some()),
                has_dmx: profile.is_some_and(|p| p.dmx().is_some()),
                has_trigger: profile.is_some_and(|p| p.trigger().is_some()),
                has_controllers: profile.is_some_and(|p| !p.controllers().is_empty()),
                filename,
            });
        }
        items.sort_by(|a, b| (&a.filename, &a.file).cmp(&(&b.filename, &b.file)));
        Ok::<_, crate::config::ConfigError>(items)
    })
    .await
}

/// Reads the profile file `filename` addresses, as a profile and as
/// written, with its version.
pub(crate) async fn read(
    profiles_dir: &Path,
    filename: &str,
) -> Result<ProfileFile, ProfileFileError> {
    validate_profile_filename(filename)?;
    let file = profile_path(profiles_dir, filename)?;
    if !file.exists() {
        return Err(ProfileFileError::NotFound(format!(
            "Profile '{filename}' not found"
        )));
    }

    // codeql[rust/path-injection] the path is validated via profile_path.
    let fp = file.path().to_path_buf();
    let (raw, profile) = blocking_io("read profile", move || {
        let raw = crate::util::read_to_string(&fp)
            .map_err(|e| format!("Failed to read profile: {}", e))?;
        // Parsed from the text read, not read again: the two could differ
        // by a save between them.
        let profile: Profile = Config::builder()
            .add_source(config::File::from_str(&raw, config::FileFormat::Yaml))
            .build()
            .and_then(|c| c.try_deserialize())
            .map_err(|e| format!("Failed to parse profile: {}", e))?;
        Ok::<_, String>((raw, profile))
    })
    .await?;

    let profile = serde_json::to_value(&profile)
        .map_err(|e| ProfileFileError::Internal(format!("Failed to serialize profile: {e}")))?;
    Ok(ProfileFile {
        profile,
        version: content_version(raw.as_bytes()),
        yaml: raw,
    })
}

/// What a save of a profile file asks.
pub(crate) struct ProfileSave<'a> {
    /// The profiles directory the config sets.
    pub(crate) profiles_dir: &'a Path,
    /// The project directory: the one place a missing profiles directory is
    /// made.
    pub(crate) project: &'a Path,
    pub(crate) filename: &'a str,
    /// The profile, as the editor sends it (sections it leaves out keep the
    /// file's; `null` removes one).
    pub(crate) body: serde_json::Value,
    /// The version the caller read the file at; none saves whatever is there.
    pub(crate) expected_version: Option<String>,
    /// Only create: a name that reaches a file already there is refused.
    pub(crate) create: bool,
}

/// Creates or updates a profile file, writes the change into the store's
/// copy of the config, and applies it to the hardware.
pub(crate) async fn save(
    player: &Arc<Player>,
    save: ProfileSave<'_>,
) -> Result<ProfileSaved, ProfileFileError> {
    let ProfileSave {
        profiles_dir,
        project,
        filename,
        body,
        expected_version,
        create,
    } = save;
    validate_profile_filename(filename)?;

    // The store's mutations persist its own copy of the profile files: held
    // from the read below to the reload after the write, so none lands in
    // between and writes the copy from before this save over it.
    let store = player.config_store();
    let disk_write = match &store {
        Some(store) => Some(store.lock_for_disk_write().await),
        None => None,
    };

    // Sections the body leaves out keep the values in the file it replaces;
    // `null` removes one (#523). The existing file is read the way the
    // profile read does; a missing or unreadable one has nothing to keep.
    let existing = existing_profile(profiles_dir, filename).await;
    let profile: Profile = Profile::from_update(existing.as_ref(), body)
        .map_err(|e| ProfileFileError::Invalid(format!("Invalid profile: {}", e)))?;

    if let Err(errors) = profile.validate() {
        return Err(ProfileFileError::Invalid(errors.join("; ")));
    }

    let new_json = serde_json::to_value(&profile)
        .map_err(|e| ProfileFileError::Internal(format!("Failed to serialize profile: {}", e)))?;

    // Before `profile_path`, which canonicalizes the directory and fails if
    // it is missing -- a creation after that call can never run.
    let profiles_dir = ensure_dir(profiles_dir, project).await?;

    // The file the profile already lives in, as the read finds it, so a
    // save never leaves two files for one profile.
    // codeql[rust/path-injection] filename is validated; path is verified via profile_path.
    let file = profile_path(&profiles_dir, filename)?;
    SafePath::validate_name_to_write(filename, file.exists())
        .map_err(|_| ProfileFileError::Invalid("Invalid profile name".to_string()))?;
    // A create is only ever a create: a name that reaches a file already
    // there (by a stem, `stage.yaml` reaching `stage.yaml.yaml`) is refused,
    // not merged.
    if create && file.exists() {
        let exists = std::io::ErrorKind::AlreadyExists.into();
        written(Err(exists), file.path())?;
    }
    let new = !file.exists();
    let file_path = file.path().to_path_buf();

    // Write the file off the async runtime. The version check and the write
    // happen under one lock so two saves cannot both pass the check. A new
    // file is created as new (`config_io::staged_create`), so one made since
    // the lookup is refused by the filesystem, never written over.
    let fp = file_path;
    let outcome = blocking_io("write profile", move || {
        let _guard = PROFILE_WRITES.lock().unwrap_or_else(|e| e.into_inner());
        let old = if new {
            None
        } else {
            crate::util::read_file(&fp).ok()
        };
        if let Some(expected) = &expected_version {
            let current = old.as_deref().map(content_version);
            if current.as_deref() != Some(expected.as_str()) {
                return Ok::<_, String>(Err(current));
            }
        }
        let mut merged = new_json;
        if let Some(old) = &old {
            carry_unknown_keys(old, &mut merged);
        }
        let yaml = crate::util::to_yaml_string(&merged).map_err(|e| e.to_string())?;
        #[cfg(test)]
        race_before_write(&fp);
        let written = config_io::write(&fp, &yaml, new);
        Ok(Ok((written, content_version(yaml.as_bytes()))))
    })
    .await?;
    let version = match outcome {
        Ok((result, version)) => {
            written(result, file.path())?;
            version
        }
        Err(current) => {
            return Err(ProfileFileError::Changed {
                message: format!(
                    "profile \"{filename}\" changed since you loaded it (another tab or \
                     page); it has been reloaded, so reapply your change"
                ),
                current,
            });
        }
    };

    reload_after_write(disk_write, "written").await?;
    let reload = player.apply_config_change().await;
    Ok(ProfileSaved { version, reload })
}

/// Deletes the profile file `filename` addresses, drops it from the store's
/// copy of the config and applies the change to the hardware.
pub(crate) async fn delete(
    player: &Arc<Player>,
    profiles_dir: &Path,
    filename: &str,
) -> Result<ConfigChange, ProfileFileError> {
    validate_profile_filename(filename)?;

    // Held from the delete to the reload after it, as a save holds it: a
    // store mutation between the two would still hold this profile, and be
    // refused for files that no longer line up with what it loaded.
    let store = player.config_store();
    let disk_write = match &store {
        Some(store) => Some(store.lock_for_disk_write().await),
        None => None,
    };

    // codeql[rust/path-injection] filename is validated; path is verified via profile_path.
    let target = profile_path(profiles_dir, filename)?;
    if !target.exists() {
        return Err(ProfileFileError::NotFound(format!(
            "Profile '{filename}' not found"
        )));
    }
    blocking_io("delete profile", move || {
        std::fs::remove_file(target.path())
    })
    .await?;

    reload_after_write(disk_write, "deleted").await?;
    Ok(player.apply_config_change().await)
}

/// The store's copy is what the hardware reload re-initialises from, and
/// writing the file did not touch it. Without this the save is acknowledged
/// and then ignored until restart. A failure here is not cosmetic: the
/// reload would re-initialise from the boot-time copy. `Player::deserialize`
/// validates the entire config, so one unrelated bad file in the directory is
/// enough to cause it -- the caller has to be told rather than left with a
/// success.
async fn reload_after_write(
    disk_write: Option<crate::config::store::DiskWrite<'_>>,
    what: &str,
) -> Result<(), ProfileFileError> {
    let Some(disk_write) = disk_write else {
        return Ok(());
    };
    disk_write.reload().await.map_err(|e| {
        warn!("Config reload after profile write failed: {}", e);
        ProfileFileError::Internal(format!(
            "the profile was {what} but the running config could not be \
             reloaded, so it will not take effect until restart: {e}"
        ))
    })
}
