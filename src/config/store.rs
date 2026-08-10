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

//! Mutable configuration store backed by a YAML file on disk.
//!
//! Wraps a `config::Player` in a `tokio::sync::RwLock`, supports optimistic
//! concurrency via a whole-config checksum, and persists every mutation
//! atomically to the original YAML file.

use std::path::PathBuf;

use tokio::sync::{broadcast, RwLock};
use tracing::warn;

use super::audio::Audio;
use super::controller::Controller;
use super::dmx::Dmx;
use super::error::ConfigError;
use super::midi::Midi;
use super::player::Player;
use super::profile::Profile;
use crate::util::to_yaml_string;
use crate::webui::config_io::staged_write;

/// A snapshot of the current configuration with its checksum.
///
/// The `yaml` field holds the exact serialized YAML string whose SHA-256
/// produces `checksum`. Consumers must use this string directly rather
/// than re-serializing `config`, because `HashMap` iteration order is
/// non-deterministic and a second serialization may produce different
/// key ordering — breaking the checksum invariant.
pub struct ConfigSnapshot {
    pub config: Player,
    pub yaml: String,
    pub checksum: String,
}

impl std::fmt::Debug for ConfigSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigSnapshot")
            .field("checksum", &self.checksum)
            .finish()
    }
}

/// Mutable configuration store.
///
/// The store wraps the full `config::Player` in a `RwLock`. Every mutation
/// validates an expected checksum (optimistic concurrency), updates the
/// in-memory config, persists to disk atomically, and broadcasts a change
/// signal.
pub struct ConfigStore {
    inner: RwLock<Player>,
    path: PathBuf,
    change_tx: broadcast::Sender<()>,
}

/// A change to one hardware subsystem, applied wherever that subsystem is
/// actually configured.
///
/// The two destinations serialize differently -- a profile holds an
/// `AudioConfig` (device settings plus this machine's routing and gains), the
/// legacy top level holds a bare `Audio` -- so the update is carried as a value
/// and applied by the destination rather than by the caller.
enum SubsystemUpdate {
    Audio(Option<Audio>),
    Midi(Option<Midi>),
    Dmx(Option<Dmx>),
}

impl SubsystemUpdate {
    /// Applies to a hardware profile, the source of truth once `profiles:` is
    /// present.
    fn apply_to_profile(self, profile: &mut Profile) {
        match self {
            SubsystemUpdate::Audio(audio) => profile.set_audio(audio),
            SubsystemUpdate::Midi(midi) => profile.set_midi(midi),
            SubsystemUpdate::Dmx(dmx) => profile.set_dmx(dmx),
        }
    }

    /// Applies to the legacy top-level blocks, which are read only when the
    /// config declares no profiles.
    fn apply_to_legacy(self, config: &mut Player) {
        match self {
            SubsystemUpdate::Audio(audio) => config.set_audio(audio),
            SubsystemUpdate::Midi(midi) => config.set_midi(midi),
            SubsystemUpdate::Dmx(dmx) => config.set_dmx(dmx),
        }
    }
}

/// Each profile serialized on its own, in declaration order.
///
/// Used both to detect which profiles an edit touched and as the exact bytes
/// written back to their files, so the comparison and the write can never
/// disagree about what changed.
fn serialized_profiles(config: &Player) -> Result<Vec<String>, ConfigError> {
    config
        .profile_list()
        .unwrap_or_default()
        .iter()
        .map(|p| to_yaml_string(p).map_err(|e| ConfigError::StoreSerialization(e.to_string())))
        .collect()
}

/// Computes a deterministic hex-encoded SHA-256 hash of a YAML string.
fn compute_checksum(yaml: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(yaml.as_bytes());
    format!("{:x}", hash)
}

impl ConfigStore {
    /// Creates a new ConfigStore wrapping an already-loaded config.
    pub fn new(config: Player, path: PathBuf) -> Self {
        let (change_tx, _) = broadcast::channel(16);
        Self {
            inner: RwLock::new(config),
            path,
            change_tx,
        }
    }

    /// Returns a snapshot of the current config with its checksum.
    #[allow(dead_code)]
    pub async fn read(&self) -> Result<ConfigSnapshot, ConfigError> {
        let guard = self.inner.read().await;
        let yaml =
            to_yaml_string(&*guard).map_err(|e| ConfigError::StoreSerialization(e.to_string()))?;
        let checksum = compute_checksum(&yaml);
        Ok(ConfigSnapshot {
            config: guard.clone(),
            yaml,
            checksum,
        })
    }

    /// Returns a clone of the current config.
    pub async fn read_config(&self) -> Player {
        self.inner.read().await.clone()
    }

    /// Returns the path to the on-disk config file.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Returns the serialized YAML and checksum without cloning the config.
    pub async fn read_yaml(&self) -> Result<(String, String), ConfigError> {
        let guard = self.inner.read().await;
        let yaml =
            to_yaml_string(&*guard).map_err(|e| ConfigError::StoreSerialization(e.to_string()))?;
        let checksum = compute_checksum(&yaml);
        Ok((yaml, checksum))
    }

    /// Subscribes to change notifications.
    #[allow(dead_code)]
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.change_tx.subscribe()
    }

    /// Applies a fallible mutation to the config. Like `mutate`, but the closure
    /// can return an error to abort the mutation.
    async fn try_mutate<F>(
        &self,
        expected_checksum: &str,
        mutate_fn: F,
    ) -> Result<ConfigSnapshot, ConfigError>
    where
        F: FnOnce(&mut Player) -> Result<(), ConfigError>,
    {
        self.mutate_inner(expected_checksum, |config| {
            mutate_fn(config)?;
            Ok(())
        })
        .await
    }

    /// Applies an infallible mutation to the config. The expected checksum is
    /// validated before mutation, and the config is persisted and broadcast after.
    async fn mutate<F>(
        &self,
        expected_checksum: &str,
        mutate_fn: F,
    ) -> Result<ConfigSnapshot, ConfigError>
    where
        F: FnOnce(&mut Player),
    {
        self.mutate_inner(expected_checksum, |config| {
            mutate_fn(config);
            Ok(())
        })
        .await
    }

    /// Core mutation implementation. Validates checksum, applies closure,
    /// persists to disk, and broadcasts.
    ///
    /// Note: blocking I/O (write_file) is performed under the write lock.
    /// This is acceptable because config mutations are rare, user-initiated
    /// operations — not on any hot path.
    async fn mutate_inner<F>(
        &self,
        expected_checksum: &str,
        mutate_fn: F,
    ) -> Result<ConfigSnapshot, ConfigError>
    where
        F: FnOnce(&mut Player) -> Result<(), ConfigError>,
    {
        let mut guard = self.inner.write().await;

        // Compute current checksum and validate.
        let current_yaml =
            to_yaml_string(&*guard).map_err(|e| ConfigError::StoreSerialization(e.to_string()))?;
        let current_checksum = compute_checksum(&current_yaml);

        if current_checksum != expected_checksum {
            return Err(ConfigError::StaleChecksum {
                expected: expected_checksum.to_string(),
                actual: current_checksum,
            });
        }

        // Snapshot the profiles before mutating, so persistence can tell which
        // profile files (if any) actually need rewriting.
        let profiles_before = serialized_profiles(&guard)?;

        // The whole config, kept so a mutation that cannot be written is undone
        // in memory too. Persistence rejects some edits (adding a profile to a
        // `profiles_dir` layout, for one), and a rejected edit that survived in
        // memory would be served to every reader and silently lost on the next
        // restart -- the same class of bug this change exists to fix.
        let rollback = guard.clone();

        // Apply the mutation.
        if let Err(e) = mutate_fn(&mut guard) {
            *guard = rollback;
            return Err(e);
        }

        // The checksum covers the whole in-memory config, including profiles
        // loaded from a directory. That is deliberate and independent of how
        // the config is split across files: it is what `read` hashes, so
        // optimistic concurrency stays consistent.
        let new_yaml =
            to_yaml_string(&*guard).map_err(|e| ConfigError::StoreSerialization(e.to_string()))?;
        let new_checksum = compute_checksum(&new_yaml);

        if let Err(e) = self.persist(&guard, &profiles_before) {
            *guard = rollback;
            return Err(e);
        }

        // Broadcast change (ignore error if no receivers).
        let _ = self.change_tx.send(());

        Ok(ConfigSnapshot {
            config: guard.clone(),
            yaml: new_yaml,
            checksum: new_checksum,
        })
    }

    /// Writes the config to disk, respecting how it is split across files.
    ///
    /// With `profiles_dir`, the in-memory `profiles` list is a load-time copy
    /// of the files in that directory, and those files -- not the main config
    /// -- are what the loader reads. Serializing the whole `Player` back to
    /// `mtrack.yaml` therefore inlined a duplicate of every profile into the
    /// one file the layout exists to avoid, while the edit itself still had no
    /// effect. So the main config is written without the inline copy, and each
    /// profile that actually changed is written to the file it came from.
    ///
    /// Only changed profiles are rewritten. Rewriting all of them would drop
    /// the comments from files that had nothing to do with the edit.
    fn persist(&self, config: &Player, profiles_before: &[String]) -> Result<(), ConfigError> {
        let Some(dir) = self.directory_profiles(config) else {
            let yaml = to_yaml_string(config)
                .map_err(|e| ConfigError::StoreSerialization(e.to_string()))?;
            return staged_write(&self.path, &yaml).map_err(ConfigError::StoreIo);
        };

        let profiles_after = serialized_profiles(config)?;
        if profiles_after.len() != profiles_before.len() {
            // Position in the list is what identifies a profile's file, so
            // adding or removing one has no unambiguous mapping. Refused rather
            // than guessed: the alternative is writing a profile into the file
            // that belongs to a different machine.
            return Err(ConfigError::Validation(format!(
                "cannot add or remove profiles while they are loaded from '{}'; edit the profile \
                 files in that directory instead",
                dir.display()
            )));
        }

        // The indices come from load time; this listing is from now. A file
        // added, removed or renamed in between shifts every index after it --
        // `00-new.yaml` sorts first and displaces the whole list -- so a
        // mismatched count means the mapping cannot be trusted and the write
        // would land in some other machine's profile.
        let files = super::player::list_profile_files(&dir)?;
        if files.len() != profiles_after.len() {
            return Err(ConfigError::Validation(format!(
                "'{}' now holds {} profile file(s) but {} were loaded; the directory changed \
                 underneath the running player, so profiles can no longer be matched to their \
                 files. Restart to pick up the new layout.",
                dir.display(),
                files.len(),
                profiles_after.len()
            )));
        }

        // The main config first. It is usually unchanged (a subsystem edit
        // lives entirely in a profile file), and doing it first means a failure
        // here leaves every profile file untouched rather than half-applied.
        // A failure *between* profile files can still split the write; that
        // needs a real two-phase commit, which this store does not have.
        let mut main = config.clone();
        if main.profile_list().is_some() {
            self.warn_if_dropping_inline_profiles();
        }
        *main.profiles_mut() = None;
        let yaml =
            to_yaml_string(&main).map_err(|e| ConfigError::StoreSerialization(e.to_string()))?;
        if std::fs::read_to_string(&self.path).ok().as_deref() != Some(yaml.as_str()) {
            staged_write(&self.path, &yaml).map_err(ConfigError::StoreIo)?;
        }

        for (index, (before, after)) in profiles_before.iter().zip(&profiles_after).enumerate() {
            if before == after {
                continue;
            }
            staged_write(&files[index], after).map_err(ConfigError::StoreIo)?;
        }

        Ok(())
    }

    /// Warns when the main config still carries an inline `profiles:` block
    /// that is about to be dropped.
    ///
    /// With `profiles_dir` populated the loader replaces inline profiles with
    /// the directory's and warns that it ignored them, so by the time the store
    /// holds the config the inline block exists only in the file. Writing the
    /// config back therefore deletes it, and it cannot be reconstructed. That is
    /// the right content to write -- keeping a stale duplicate would be worse --
    /// but it removes something the operator wrote, so it is not done silently.
    fn warn_if_dropping_inline_profiles(&self) {
        let Ok(current) = std::fs::read_to_string(&self.path) else {
            return;
        };
        if current.lines().any(|l| l.trim_start() == "profiles:") {
            warn!(
                path = %self.path.display(),
                "removing the inline 'profiles:' block from the config: it was already ignored in \
                 favour of 'profiles_dir', and is not preserved across a config write"
            );
        }
    }

    /// The profiles directory, but only when it is actually supplying the
    /// profiles.
    ///
    /// `load_profiles_dir` keeps inline profiles as a fallback when the
    /// directory holds no YAML files, so a configured-but-empty directory must
    /// still persist inline -- otherwise the profiles would be stripped from
    /// the main config and nothing would be left to load.
    fn directory_profiles(&self, config: &Player) -> Option<PathBuf> {
        let dir = config.resolved_profiles_dir(&self.path)?;
        match super::player::list_profile_files(&dir) {
            Ok(files) if !files.is_empty() => Some(dir),
            _ => None,
        }
    }

    /// Updates the audio configuration.
    pub async fn update_audio(
        &self,
        audio: Option<Audio>,
        checksum: &str,
    ) -> Result<ConfigSnapshot, ConfigError> {
        self.update_subsystem(SubsystemUpdate::Audio(audio), checksum)
            .await
    }

    /// Updates the MIDI configuration.
    pub async fn update_midi(
        &self,
        midi: Option<Midi>,
        checksum: &str,
    ) -> Result<ConfigSnapshot, ConfigError> {
        self.update_subsystem(SubsystemUpdate::Midi(midi), checksum)
            .await
    }

    /// Updates the DMX configuration.
    pub async fn update_dmx(
        &self,
        dmx: Option<Dmx>,
        checksum: &str,
    ) -> Result<ConfigSnapshot, ConfigError> {
        self.update_subsystem(SubsystemUpdate::Dmx(dmx), checksum)
            .await
    }

    /// Applies a subsystem update to wherever the loader will actually read it.
    ///
    /// `Player::normalize` makes `profiles` the source of truth and *discards*
    /// the top-level `audio`/`midi`/`dmx` blocks the moment `profiles:` is
    /// present, warning as it goes. Writing to the top level therefore produced
    /// a value that the served config showed, the checksum covered, and the
    /// next restart threw away -- silently, unless someone was reading logs.
    /// So the update goes to the active profile whenever there is one, and to
    /// the legacy top-level block only on a config that has no profiles at all.
    async fn update_subsystem(
        &self,
        update: SubsystemUpdate,
        checksum: &str,
    ) -> Result<ConfigSnapshot, ConfigError> {
        let hostname = super::hostname::resolve_hostname();
        self.try_mutate(checksum, |config| {
            if config.profile_list().is_some() {
                let profile = config.active_profile_mut(&hostname).ok_or_else(|| {
                    ConfigError::Validation(format!(
                        "no profile matches hostname '{hostname}', so there is nowhere to apply \
                         this update"
                    ))
                })?;
                update.apply_to_profile(profile);
            } else {
                update.apply_to_legacy(config);
            }
            Ok(())
        })
        .await
    }

    /// Updates the controllers configuration.
    pub async fn update_controllers(
        &self,
        controllers: Vec<Controller>,
        checksum: &str,
    ) -> Result<ConfigSnapshot, ConfigError> {
        self.mutate(checksum, |config| {
            config.set_controllers(controllers);
        })
        .await
    }

    /// Updates the inline sample definitions.
    pub async fn update_samples(
        &self,
        samples: std::collections::HashMap<String, super::samples::SampleDefinition>,
        max_sample_voices: Option<u32>,
        checksum: &str,
    ) -> Result<ConfigSnapshot, ConfigError> {
        self.mutate(checksum, |config| {
            config.set_samples(samples);
            config.set_max_sample_voices(max_sample_voices);
        })
        .await
    }

    /// Adds a profile.
    pub async fn add_profile(
        &self,
        profile: Profile,
        checksum: &str,
    ) -> Result<ConfigSnapshot, ConfigError> {
        self.mutate(checksum, |config| {
            let profiles = config.profiles_mut();
            match profiles {
                Some(list) => list.push(profile),
                None => *profiles = Some(vec![profile]),
            }
        })
        .await
    }

    /// Updates a profile at the given index.
    pub async fn update_profile(
        &self,
        index: usize,
        profile: Profile,
        checksum: &str,
    ) -> Result<ConfigSnapshot, ConfigError> {
        self.try_mutate(checksum, |config| {
            let list = config
                .profiles_mut()
                .as_mut()
                .ok_or(ConfigError::InvalidProfileIndex { index, len: 0 })?;
            if index >= list.len() {
                return Err(ConfigError::InvalidProfileIndex {
                    index,
                    len: list.len(),
                });
            }
            list[index] = profile;
            Ok(())
        })
        .await
    }

    /// Removes a profile at the given index.
    pub async fn remove_profile(
        &self,
        index: usize,
        checksum: &str,
    ) -> Result<ConfigSnapshot, ConfigError> {
        self.try_mutate(checksum, |config| {
            let list = config
                .profiles_mut()
                .as_mut()
                .ok_or(ConfigError::InvalidProfileIndex { index, len: 0 })?;
            if index >= list.len() {
                return Err(ConfigError::InvalidProfileIndex {
                    index,
                    len: list.len(),
                });
            }
            list.remove(index);
            Ok(())
        })
        .await
    }

    /// Sets the per-track gains on the active profile (first profile whose
    /// hostname matches or that has no hostname constraint) and persists them.
    /// Called internally when gains change at runtime, not from the config
    /// editor UI, so no checksum is required (same model as
    /// `set_active_playlist`): the mutation runs under the store's write
    /// lock, and a config-editor save racing it gets a StaleChecksum
    /// rejection plus a `config_changed` broadcast to re-pull.
    ///
    /// When profiles are loaded from `profiles_dir`, the owning profile file
    /// is rewritten (inline profiles in the main config are ignored at load
    /// time in that layout). Rewriting a profile file drops YAML comments.
    pub async fn set_track_gains(
        &self,
        hostname: &str,
        gains: indexmap::IndexMap<String, f32>,
    ) -> Result<(), ConfigError> {
        let mut guard = self.inner.write().await;

        // Update the in-memory active profile so subsequent reads are current.
        let profile = guard.active_profile_mut(hostname).ok_or_else(|| {
            ConfigError::Validation(format!("no profile matches hostname '{}'", hostname))
        })?;
        let audio = profile.audio_config_mut().ok_or_else(|| {
            ConfigError::Validation("active profile has no audio config".to_string())
        })?;
        audio.set_track_gains(gains.clone());

        // `directory_profiles` rather than `resolved_profiles_dir`: a
        // configured-but-empty directory falls back to inline profiles at load
        // time, and looking for an owning file in it would fail with "no
        // profile file matches" on a config that loaded perfectly well.
        if let Some(dir) = self.directory_profiles(&guard) {
            persist_gains_to_profile_file(&dir, hostname, &gains)?;
        } else {
            let new_yaml = to_yaml_string(&*guard)
                .map_err(|e| ConfigError::StoreSerialization(e.to_string()))?;
            staged_write(&self.path, &new_yaml).map_err(ConfigError::StoreIo)?;
        }

        let _ = self.change_tx.send(());
        Ok(())
    }

    /// Sets the active playlist name without requiring a checksum.
    /// This is called internally when switching playlists, not from the
    /// config editor UI, so optimistic concurrency isn't needed.
    ///
    /// Goes through [`ConfigStore::persist`] like every other write. Writing
    /// the whole `Player` directly would inline a copy of every directory-loaded
    /// profile into the main config -- the same corruption this store was fixed
    /// to stop, on a path an operator hits every time they change playlist.
    pub async fn set_active_playlist(&self, name: String) -> Result<(), ConfigError> {
        let mut guard = self.inner.write().await;
        // The profiles are untouched here, so this is only ever "nothing
        // changed" and no profile file is rewritten.
        let profiles_before = serialized_profiles(&guard)?;
        guard.set_active_playlist(name);
        self.persist(&guard, &profiles_before)?;
        let _ = self.change_tx.send(());
        Ok(())
    }
}

/// Persists track gains into the profile file inside `dir` that owns the
/// active profile: files are visited in the same sorted order used at load
/// time, and the first profile matching the hostname rule wins.
fn persist_gains_to_profile_file(
    dir: &std::path::Path,
    hostname: &str,
    gains: &indexmap::IndexMap<String, f32>,
) -> Result<(), ConfigError> {
    let yaml_paths = super::player::list_profile_files(dir)?;

    // A parse failure aborts (matching load behavior) rather than skipping:
    // skipping an unparseable file could write the gains into the wrong
    // profile when the broken file is the active profile's.
    for path in &yaml_paths {
        let mut profile = config::Config::builder()
            .add_source(config::File::from(path.as_path()))
            .build()
            .and_then(|c| c.try_deserialize::<Profile>())
            .map_err(|source| ConfigError::ProfileParse {
                path: path.clone(),
                source,
            })?;

        let matches = match profile.hostname() {
            Some(h) => h == hostname,
            None => true,
        };
        if !matches {
            continue;
        }

        let audio = profile.audio_config_mut().ok_or_else(|| {
            ConfigError::Validation("active profile has no audio config".to_string())
        })?;
        audio.set_track_gains(gains.clone());

        let yaml =
            to_yaml_string(&profile).map_err(|e| ConfigError::StoreSerialization(e.to_string()))?;
        staged_write(path, &yaml).map_err(ConfigError::StoreIo)?;
        return Ok(());
    }

    Err(ConfigError::Validation(format!(
        "no profile file in {} matches hostname '{}'",
        dir.display(),
        hostname
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_player(yaml: &str) -> Player {
        let mut temp = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        temp.write_all(yaml.as_bytes()).unwrap();
        Player::deserialize(temp.path()).unwrap()
    }

    fn basic_yaml() -> &'static str {
        r#"
songs: songs
profiles:
  - audio:
      device: mock-device
      track_mappings:
        click: [1]
"#
    }

    /// A `Midi` with `beat_clock` and `persist_tempo` set, which `Midi::new`
    /// cannot express.
    fn midi_with_persist(device: &str) -> Midi {
        config::Config::builder()
            .add_source(config::File::from_str(
                &format!("device: {device}\nbeat_clock: true\npersist_tempo: true\n"),
                config::FileFormat::Yaml,
            ))
            .build()
            .and_then(|c| c.try_deserialize())
            .unwrap()
    }

    /// A subsystem update must reach the profile that owns the setting, not the
    /// top-level block the loader discards once `profiles:` is present.
    #[tokio::test]
    async fn update_midi_inline_profiles_reaches_the_active_profile() {
        let yaml = "songs: songs\nprofiles:\n  - audio:\n      device: a\n      track_mappings:\n        click: [1]\n    midi:\n      device: old-midi\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, yaml).unwrap();
        let player = Player::deserialize(&path).unwrap();

        let store = ConfigStore::new(player, path.clone());
        let snap = store.read().await.unwrap();
        store
            .update_midi(Some(midi_with_persist("new-midi")), &snap.checksum)
            .await
            .unwrap();

        // The value has to survive a full reload, which is what the player does
        // on restart.
        let mut reloaded = Player::deserialize(&path).unwrap();
        let profile = reloaded
            .active_profile_mut(&super::super::hostname::resolve_hostname())
            .expect("a hostname-less profile matches every host");
        let midi = profile.midi().expect("the update must land on the profile");
        assert_eq!(midi.device(), "new-midi");
        assert!(midi.persist_tempo(), "persist_tempo was discarded");
    }

    /// The same, on a `profiles_dir` layout: the owning file is rewritten and
    /// the main config is not turned into an inline copy of the directory.
    #[tokio::test]
    async fn update_midi_profiles_dir_writes_the_owning_profile_file() {
        let dir = tempfile::tempdir().unwrap();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();

        let main_yaml = "songs: songs\nprofiles_dir: profiles\n";
        let main_path = dir.path().join("config.yaml");
        std::fs::write(&main_path, main_yaml).unwrap();
        std::fs::write(
            profiles_dir.join("01-only.yaml"),
            "kind: hardware_profile\naudio:\n  device: a\n  track_mappings:\n    click: [1]\nmidi:\n  device: old-midi\n",
        )
        .unwrap();

        let player = Player::deserialize(&main_path).unwrap();
        let store = ConfigStore::new(player, main_path.clone());
        let snap = store.read().await.unwrap();
        store
            .update_midi(Some(midi_with_persist("new-midi")), &snap.checksum)
            .await
            .unwrap();

        let profile_after = std::fs::read_to_string(profiles_dir.join("01-only.yaml")).unwrap();
        assert!(
            profile_after.contains("persist_tempo: true"),
            "the profile file that owns this setting was not updated: {profile_after}"
        );

        // The directory is the source of truth, so the main config must not be
        // rewritten with the profiles inlined -- that collapses a multi-file
        // layout into one file and silently changes which profiles load.
        let main_after = std::fs::read_to_string(&main_path).unwrap();
        assert!(
            main_after.contains("profiles_dir:"),
            "profiles_dir was dropped from the main config: {main_after}"
        );
        assert!(
            !main_after.contains("old-midi") && !main_after.contains("new-midi"),
            "the profiles were inlined into the main config: {main_after}"
        );

        let mut reloaded = Player::deserialize(&main_path).unwrap();
        let profile = reloaded
            .active_profile_mut(&super::super::hostname::resolve_hostname())
            .unwrap();
        assert!(profile.midi().unwrap().persist_tempo());
    }

    /// Position in the profile list is what identifies a profile's file, so
    /// adding one through this API has no unambiguous destination. It must be
    /// refused outright rather than half-applied.
    #[tokio::test]
    async fn add_profile_is_refused_on_a_profiles_dir_layout() {
        let dir = tempfile::tempdir().unwrap();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        let main_yaml = "songs: songs\nprofiles_dir: profiles\n";
        let main_path = dir.path().join("config.yaml");
        std::fs::write(&main_path, main_yaml).unwrap();
        std::fs::write(
            profiles_dir.join("01-only.yaml"),
            "kind: hardware_profile\naudio:\n  device: a\n  track_mappings:\n    click: [1]\n",
        )
        .unwrap();

        let player = Player::deserialize(&main_path).unwrap();
        let store = ConfigStore::new(player, main_path.clone());
        let snap = store.read().await.unwrap();

        let added = Profile::new(Some("new-host".to_string()), None, None, None);
        let err = store.add_profile(added, &snap.checksum).await.unwrap_err();
        assert!(
            matches!(err, ConfigError::Validation(_)),
            "expected a validation error, got {err:?}"
        );

        // Refused means refused: the in-memory config must not keep the profile
        // that could not be written, or readers would see an edit that vanishes
        // on restart.
        let after = store.read_config().await;
        assert_eq!(
            after.profile_list().map(|p| p.len()),
            Some(1),
            "the rejected profile was left in memory"
        );
        let main_after = std::fs::read_to_string(&main_path).unwrap();
        assert_eq!(main_after, main_yaml, "the main config was rewritten");
    }

    /// Builds a `profiles_dir` project with one profile file, returning the
    /// temp dir, the main config path, and the profiles directory.
    fn profiles_dir_project(
        profile_yaml: &str,
    ) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        let main_path = dir.path().join("config.yaml");
        std::fs::write(&main_path, "songs: songs\nprofiles_dir: profiles\n").unwrap();
        std::fs::write(profiles_dir.join("01-only.yaml"), profile_yaml).unwrap();
        (dir, main_path, profiles_dir)
    }

    /// Switching playlist happens constantly during a show and goes through a
    /// checksum-free path of its own, so it has to respect the split layout
    /// like every other write.
    #[tokio::test]
    async fn set_active_playlist_does_not_inline_directory_profiles() {
        let (_dir, main_path, _profiles_dir) = profiles_dir_project(
            "kind: hardware_profile\naudio:\n  device: a\n  track_mappings:\n    click: [1]\nmidi:\n  device: some-midi\n",
        );
        let player = Player::deserialize(&main_path).unwrap();
        let store = ConfigStore::new(player, main_path.clone());

        store
            .set_active_playlist("alternate".to_string())
            .await
            .unwrap();

        let main_after = std::fs::read_to_string(&main_path).unwrap();
        assert!(
            main_after.contains("active_playlist: alternate"),
            "the playlist change was not written: {main_after}"
        );
        assert!(
            main_after.contains("profiles_dir:"),
            "profiles_dir was dropped: {main_after}"
        );
        assert!(
            !main_after.contains("some-midi"),
            "the directory's profiles were inlined into the main config: {main_after}"
        );
    }

    /// Profile indices are fixed at load time but the file list is read at
    /// write time. A file appearing since load shifts every index after it, so
    /// the write must be refused rather than landing in the wrong file.
    #[tokio::test]
    async fn a_profile_file_added_since_load_blocks_the_write() {
        let (_dir, main_path, profiles_dir) = profiles_dir_project(
            "kind: hardware_profile\naudio:\n  device: a\n  track_mappings:\n    click: [1]\nmidi:\n  device: old-midi\n",
        );
        let player = Player::deserialize(&main_path).unwrap();
        let store = ConfigStore::new(player, main_path.clone());
        let snap = store.read().await.unwrap();

        // Sorts before the loaded one, so it would take index 0.
        let intruder = profiles_dir.join("00-new.yaml");
        std::fs::write(
            &intruder,
            "kind: hardware_profile\nhostname: someone-else\naudio:\n  device: z\n",
        )
        .unwrap();

        let err = store
            .update_midi(Some(midi_with_persist("new-midi")), &snap.checksum)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ConfigError::Validation(_)),
            "expected a validation error, got {err:?}"
        );

        // The other machine's profile must be exactly as it was written.
        let intruder_after = std::fs::read_to_string(&intruder).unwrap();
        assert!(
            !intruder_after.contains("new-midi"),
            "the edit landed in another machine's profile file: {intruder_after}"
        );
    }

    #[tokio::test]
    async fn checksum_stable_for_same_content() {
        let player = make_player(basic_yaml());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, basic_yaml()).unwrap();

        let store = ConfigStore::new(player, path);
        let snap1 = store.read().await.unwrap();
        let snap2 = store.read().await.unwrap();
        assert_eq!(snap1.checksum, snap2.checksum);
    }

    #[tokio::test]
    async fn checksum_changes_when_content_changes() {
        let player = make_player(basic_yaml());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, basic_yaml()).unwrap();

        let store = ConfigStore::new(player, path);
        let snap1 = store.read().await.unwrap();

        let _snap2 = store
            .update_midi(Some(Midi::new("new-midi", None)), &snap1.checksum)
            .await
            .unwrap();
        let snap3 = store.read().await.unwrap();
        assert_ne!(snap1.checksum, snap3.checksum);
    }

    #[tokio::test]
    async fn update_with_correct_checksum_succeeds_and_persists() {
        let player = make_player(basic_yaml());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, basic_yaml()).unwrap();

        let store = ConfigStore::new(player, path.clone());
        let snap = store.read().await.unwrap();

        let new_snap = store
            .update_midi(Some(Midi::new("updated-midi", None)), &snap.checksum)
            .await
            .unwrap();

        // Verify in-memory state.
        let read_snap = store.read().await.unwrap();
        assert_eq!(read_snap.checksum, new_snap.checksum);

        // Verify persisted to disk.
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains("updated-midi"));
    }

    #[tokio::test]
    async fn update_with_stale_checksum_returns_error() {
        let player = make_player(basic_yaml());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, basic_yaml()).unwrap();

        let store = ConfigStore::new(player, path);
        let result = store
            .update_midi(Some(Midi::new("new-midi", None)), "stale-checksum")
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::StaleChecksum {
                expected,
                actual: _,
            } => {
                assert_eq!(expected, "stale-checksum");
            }
            other => panic!("expected StaleChecksum, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn subscribers_notified_on_mutation() {
        let player = make_player(basic_yaml());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, basic_yaml()).unwrap();

        let store = ConfigStore::new(player, path);
        let mut rx = store.subscribe();

        let snap = store.read().await.unwrap();
        store
            .update_midi(Some(Midi::new("midi-device", None)), &snap.checksum)
            .await
            .unwrap();

        // Should receive the notification.
        let result = rx.try_recv();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn concurrent_reads_dont_block() {
        let player = make_player(basic_yaml());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, basic_yaml()).unwrap();

        let store = std::sync::Arc::new(ConfigStore::new(player, path));

        let store1 = store.clone();
        let store2 = store.clone();

        let (r1, r2) = tokio::join!(
            tokio::spawn(async move { store1.read().await.unwrap().checksum }),
            tokio::spawn(async move { store2.read().await.unwrap().checksum }),
        );
        assert_eq!(r1.unwrap(), r2.unwrap());
    }

    #[tokio::test]
    async fn add_and_remove_profile() {
        let player = make_player(basic_yaml());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, basic_yaml()).unwrap();

        let store = ConfigStore::new(player, path);
        let snap = store.read().await.unwrap();

        // Add a profile.
        let new_profile = Profile::new(Some("new-host".to_string()), None, None, None);
        let snap = store
            .add_profile(new_profile, &snap.checksum)
            .await
            .unwrap();
        assert_eq!(snap.config.all_profiles().len(), 2);

        // Remove the added profile.
        let snap = store.remove_profile(1, &snap.checksum).await.unwrap();
        assert_eq!(snap.config.all_profiles().len(), 1);
    }

    #[tokio::test]
    async fn update_profile_at_index() {
        let player = make_player(basic_yaml());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, basic_yaml()).unwrap();

        let store = ConfigStore::new(player, path);
        let snap = store.read().await.unwrap();

        let updated = Profile::new(Some("updated-host".to_string()), None, None, None);
        let snap = store
            .update_profile(0, updated, &snap.checksum)
            .await
            .unwrap();
        assert_eq!(
            snap.config.all_profiles()[0].hostname(),
            Some("updated-host")
        );
    }

    #[tokio::test]
    async fn update_profile_out_of_bounds_returns_error() {
        let player = make_player(basic_yaml());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, basic_yaml()).unwrap();

        let store = ConfigStore::new(player, path);
        let snap = store.read().await.unwrap();

        let profile = Profile::new(Some("host".to_string()), None, None, None);
        let result = store.update_profile(99, profile, &snap.checksum).await;
        match result.unwrap_err() {
            ConfigError::InvalidProfileIndex { index, len } => {
                assert_eq!(index, 99);
                assert_eq!(len, 1);
            }
            other => panic!("expected InvalidProfileIndex, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn remove_profile_out_of_bounds_returns_error() {
        let player = make_player(basic_yaml());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, basic_yaml()).unwrap();

        let store = ConfigStore::new(player, path);
        let snap = store.read().await.unwrap();

        let result = store.remove_profile(5, &snap.checksum).await;
        match result.unwrap_err() {
            ConfigError::InvalidProfileIndex { index, len } => {
                assert_eq!(index, 5);
                assert_eq!(len, 1);
            }
            other => panic!("expected InvalidProfileIndex, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn update_audio_stores_new_audio() {
        let player = make_player(basic_yaml());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, basic_yaml()).unwrap();

        let store = ConfigStore::new(player, path);
        let snap = store.read().await.unwrap();

        let snap = store
            .update_audio(
                Some(super::super::audio::Audio::new("new-audio-device")),
                &snap.checksum,
            )
            .await
            .unwrap();

        // This config declares profiles, so the update belongs to the active
        // profile -- the top-level block would be discarded on the next load.
        let (yaml, _) = store.read_yaml().await.unwrap();
        assert!(yaml.contains("new-audio-device"));

        // An audio update carries device settings only, so the profile's track
        // mappings must survive it. Losing them turns a working rig silent.
        let profile_mappings = store
            .read_config()
            .await
            .active_profile_mut(&super::super::hostname::resolve_hostname())
            .and_then(|p| p.audio_config().map(|a| a.track_mappings().clone()))
            .unwrap();
        assert!(
            profile_mappings.contains_key("click"),
            "update_audio wiped the profile's track mappings: {profile_mappings:?}"
        );

        // Clear audio.
        let snap = store.update_audio(None, &snap.checksum).await.unwrap();
        let (yaml, _) = store.read_yaml().await.unwrap();
        assert!(!yaml.contains("new-audio-device"));
        drop(snap);
    }

    #[tokio::test]
    async fn update_dmx_stores_new_dmx() {
        let player = make_player(basic_yaml());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, basic_yaml()).unwrap();

        let store = ConfigStore::new(player, path);
        let snap = store.read().await.unwrap();

        let dmx = super::super::dmx::Dmx::new(
            None,
            None,
            Some(9090),
            vec![super::super::dmx::Universe::new(1, "test".to_string())],
            None,
        );
        let snap = store.update_dmx(Some(dmx), &snap.checksum).await.unwrap();

        let (yaml, _) = store.read_yaml().await.unwrap();
        assert!(yaml.contains("9090"));

        // Clear DMX.
        let snap = store.update_dmx(None, &snap.checksum).await.unwrap();
        let (yaml, _) = store.read_yaml().await.unwrap();
        assert!(!yaml.contains("9090"));
        drop(snap);
    }

    #[tokio::test]
    async fn update_controllers_stores_and_clears() {
        let player = make_player(basic_yaml());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, basic_yaml()).unwrap();

        let store = ConfigStore::new(player, path);
        let snap = store.read().await.unwrap();

        let controllers = vec![Controller::Grpc(
            super::super::controller::GrpcController::new(5000),
        )];
        let snap = store
            .update_controllers(controllers, &snap.checksum)
            .await
            .unwrap();

        let (yaml, _) = store.read_yaml().await.unwrap();
        assert!(yaml.contains("5000"));

        // Empty vec clears controllers (maps to None).
        let snap = store
            .update_controllers(vec![], &snap.checksum)
            .await
            .unwrap();
        let (yaml, _) = store.read_yaml().await.unwrap();
        assert!(!yaml.contains("5000"));
        drop(snap);
    }

    #[tokio::test]
    async fn disk_persistence_round_trip() {
        let player = make_player(basic_yaml());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, basic_yaml()).unwrap();

        let store = ConfigStore::new(player, path.clone());
        let snap = store.read().await.unwrap();

        // Mutate via store — add a profile with a distinctive hostname.
        let profile = Profile::new(Some("round-trip-host".to_string()), None, None, None);
        store.add_profile(profile, &snap.checksum).await.unwrap();

        // Deserialize from disk independently.
        let reloaded = Player::deserialize(&path).unwrap();
        let hostnames: Vec<_> = reloaded
            .all_profiles()
            .iter()
            .filter_map(|p| p.hostname())
            .collect();
        assert!(
            hostnames.contains(&"round-trip-host"),
            "expected round-trip-host in {:?}",
            hostnames
        );
    }

    #[tokio::test]
    async fn set_track_gains_inline_profile_persists() {
        let yaml = r#"
songs: songs
profiles:
  - hostname: gain-host
    audio:
      device: mock-device
      track_mappings:
        click: [1]
        keys: [2, 3]
"#;
        let player = make_player(yaml);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, yaml).unwrap();

        let store = ConfigStore::new(player, path.clone());
        let gains = indexmap::IndexMap::from([("click".to_string(), -6.0f32)]);
        store.set_track_gains("gain-host", gains).await.unwrap();

        // In-memory config reflects the change.
        let mut config = store.read_config().await;
        let profile = config.active_profile_mut("gain-host").unwrap();
        let audio = profile.audio_config().unwrap();
        assert_eq!(audio.track_gains()["click"], -6.0);

        // On-disk config reflects the change after an independent reload.
        let mut reloaded = Player::deserialize(&path).unwrap();
        let profile = reloaded.active_profile_mut("gain-host").unwrap();
        assert_eq!(profile.audio_config().unwrap().track_gains()["click"], -6.0);
    }

    #[tokio::test]
    async fn set_track_gains_profiles_dir_writes_profile_file() {
        let dir = tempfile::tempdir().unwrap();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();

        let main_yaml = "songs: songs\nprofiles_dir: profiles\n";
        let main_path = dir.path().join("config.yaml");
        std::fs::write(&main_path, main_yaml).unwrap();

        // Two profile files: the first doesn't match the hostname, the second does.
        std::fs::write(
            profiles_dir.join("01-other.yaml"),
            "kind: hardware_profile\nhostname: other-host\naudio:\n  device: a\n  track_mappings:\n    cue: [1]\n",
        )
        .unwrap();
        std::fs::write(
            profiles_dir.join("02-target.yaml"),
            "kind: hardware_profile\nhostname: gain-host\naudio:\n  device: b\n  track_mappings:\n    click: [1]\n",
        )
        .unwrap();

        let player = Player::deserialize(&main_path).unwrap();
        let store = ConfigStore::new(player, main_path.clone());
        let gains = indexmap::IndexMap::from([("click".to_string(), 3.0f32)]);
        store.set_track_gains("gain-host", gains).await.unwrap();

        // The owning profile file was updated; the other one untouched.
        let target = std::fs::read_to_string(profiles_dir.join("02-target.yaml")).unwrap();
        assert!(target.contains("track_gains"), "got: {target}");
        assert!(target.contains("click: 3"), "got: {target}");
        let other = std::fs::read_to_string(profiles_dir.join("01-other.yaml")).unwrap();
        assert!(!other.contains("track_gains"));

        // The main config file was not rewritten with inlined profiles.
        let main_after = std::fs::read_to_string(&main_path).unwrap();
        assert_eq!(main_after, main_yaml);

        // A full reload through profiles_dir sees the gains.
        let mut reloaded = Player::deserialize(&main_path).unwrap();
        let profile = reloaded.active_profile_mut("gain-host").unwrap();
        assert_eq!(profile.audio_config().unwrap().track_gains()["click"], 3.0);
    }

    #[tokio::test]
    async fn set_track_gains_unknown_hostname_errors() {
        // The profile is hostname-constrained, so a different hostname matches
        // no profile and the call must fail.
        let yaml = "songs: songs\nprofiles:\n  - hostname: only-host\n    audio:\n      device: a\n      track_mappings:\n        click: [1]\n";
        let player = make_player(yaml);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, yaml).unwrap();
        let store = ConfigStore::new(player, path);

        let gains = indexmap::IndexMap::from([("click".to_string(), -6.0f32)]);
        assert!(store.set_track_gains("nope", gains).await.is_err());
    }

    #[test]
    fn sha256_checksum_deterministic() {
        let yaml = "songs: songs\nprofiles:\n  - audio:\n      device: test\n";
        let c1 = compute_checksum(yaml);
        let c2 = compute_checksum(yaml);
        assert_eq!(c1, c2);
        // SHA-256 produces a 64-char hex string.
        assert_eq!(c1.len(), 64);
        assert!(c1.chars().all(|c| c.is_ascii_hexdigit()));

        // Different content produces different checksum.
        let c3 = compute_checksum("different content");
        assert_ne!(c1, c3);
    }
}
