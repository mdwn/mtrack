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

use super::audio::Audio;
use super::controller::Controller;
use super::dmx::Dmx;
use super::error::ConfigError;
use super::midi::Midi;
use super::player::Player;
use super::profile::Profile;
use crate::util::to_yaml_string;
use crate::webui::config_io::atomic_write;

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
    /// Note: blocking I/O (atomic_write) is performed under the write lock.
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

        // Apply the mutation.
        mutate_fn(&mut guard)?;

        // Serialize and persist.
        let new_yaml =
            to_yaml_string(&*guard).map_err(|e| ConfigError::StoreSerialization(e.to_string()))?;
        let new_checksum = compute_checksum(&new_yaml);

        atomic_write(&self.path, &new_yaml).map_err(ConfigError::StoreIo)?;

        // Broadcast change (ignore error if no receivers).
        let _ = self.change_tx.send(());

        Ok(ConfigSnapshot {
            config: guard.clone(),
            yaml: new_yaml,
            checksum: new_checksum,
        })
    }

    /// Updates the audio configuration.
    pub async fn update_audio(
        &self,
        audio: Option<Audio>,
        checksum: &str,
    ) -> Result<ConfigSnapshot, ConfigError> {
        self.mutate(checksum, |config| {
            config.set_audio(audio);
        })
        .await
    }

    /// Updates the MIDI configuration.
    pub async fn update_midi(
        &self,
        midi: Option<Midi>,
        checksum: &str,
    ) -> Result<ConfigSnapshot, ConfigError> {
        self.mutate(checksum, |config| {
            config.set_midi(midi);
        })
        .await
    }

    /// Updates the DMX configuration.
    pub async fn update_dmx(
        &self,
        dmx: Option<Dmx>,
        checksum: &str,
    ) -> Result<ConfigSnapshot, ConfigError> {
        self.mutate(checksum, |config| {
            config.set_dmx(dmx);
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

        if let Some(dir) = guard.resolved_profiles_dir(&self.path) {
            persist_gains_to_profile_file(&dir, hostname, &gains)?;
        } else {
            let new_yaml = to_yaml_string(&*guard)
                .map_err(|e| ConfigError::StoreSerialization(e.to_string()))?;
            atomic_write(&self.path, &new_yaml).map_err(ConfigError::StoreIo)?;
        }

        let _ = self.change_tx.send(());
        Ok(())
    }

    /// Sets the active playlist name without requiring a checksum.
    /// This is called internally when switching playlists, not from the
    /// config editor UI, so optimistic concurrency isn't needed.
    pub async fn set_active_playlist(&self, name: String) -> Result<(), ConfigError> {
        let mut guard = self.inner.write().await;
        guard.set_active_playlist(name);
        let new_yaml =
            to_yaml_string(&*guard).map_err(|e| ConfigError::StoreSerialization(e.to_string()))?;
        atomic_write(&self.path, &new_yaml).map_err(ConfigError::StoreIo)?;
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
        atomic_write(path, &yaml).map_err(ConfigError::StoreIo)?;
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

        // Verify via serialized YAML (setters modify top-level fields).
        let (yaml, _) = store.read_yaml().await.unwrap();
        assert!(yaml.contains("new-audio-device"));

        // Clear audio.
        let snap = store.update_audio(None, &snap.checksum).await.unwrap();
        let (yaml, _) = store.read_yaml().await.unwrap();
        // After clearing, the top-level audio key should be gone,
        // but the profile's audio should still be there.
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
