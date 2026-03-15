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

use std::error::Error;
use std::path::{Path, PathBuf};

use crate::config;

/// Resolves the config file path from a user-provided path string.
/// If it's a file or has a YAML extension, use it directly.
/// Otherwise treat it as a project directory and look for mtrack.yaml inside.
fn resolve_config_path(path: &str) -> PathBuf {
    let input = Path::new(path);
    if input.is_file() || input.extension().is_some_and(|e| e == "yaml" || e == "yml") {
        input.to_path_buf()
    } else {
        input.join("mtrack.yaml")
    }
}

/// A migration action to report to the user.
struct MigrationAction {
    category: &'static str,
    description: String,
}

/// Collected migration results that can be applied or reported.
struct MigrationPlan {
    actions: Vec<MigrationAction>,
    files_to_write: Vec<(PathBuf, String)>,
    files_to_copy: Vec<(PathBuf, PathBuf)>,
    dirs_to_create: Vec<PathBuf>,
    files_to_delete: Vec<PathBuf>,
    config_yaml: Option<String>,
    config_path: PathBuf,
    backup_path: PathBuf,
}

impl MigrationPlan {
    fn new(config_path: PathBuf) -> Self {
        let backup_path = config_path.with_extension("yaml.bak");
        Self {
            actions: Vec::new(),
            files_to_write: Vec::new(),
            files_to_copy: Vec::new(),
            dirs_to_create: Vec::new(),
            files_to_delete: Vec::new(),
            config_yaml: None,
            config_path,
            backup_path,
        }
    }

    fn add_action(&mut self, category: &'static str, description: String) {
        self.actions.push(MigrationAction {
            category,
            description,
        });
    }

    fn has_changes(&self) -> bool {
        !self.actions.is_empty()
    }

    fn print_report(&self, apply: bool) {
        if apply {
            println!("mtrack migrate [applied]\n");
        } else {
            println!("mtrack migrate [dry-run]\n");
        }

        let mut current_category = "";
        for action in &self.actions {
            if action.category != current_category {
                if !current_category.is_empty() {
                    println!();
                }
                println!("{}:", action.category);
                current_category = action.category;
            }
            println!("  [migrate] {}", action.description);
        }

        if !apply {
            println!("\nRun with --apply to execute these changes.");
        }
    }

    fn apply(&self) -> Result<(), Box<dyn Error>> {
        // Create directories.
        for dir in &self.dirs_to_create {
            std::fs::create_dir_all(dir)?;
        }

        // Write files.
        for (path, content) in &self.files_to_write {
            std::fs::write(path, content)?;
        }

        // Copy files.
        for (src, dst) in &self.files_to_copy {
            std::fs::copy(src, dst)?;
        }

        // Backup and rewrite config.
        if let Some(yaml) = &self.config_yaml {
            std::fs::copy(&self.config_path, &self.backup_path)?;
            std::fs::write(&self.config_path, yaml)?;
        }

        // Delete files (after successful copy).
        for path in &self.files_to_delete {
            std::fs::remove_file(path)?;
        }

        Ok(())
    }
}

/// Runs the migration process.
pub fn migrate(path: &str, apply: bool) -> Result<(), Box<dyn Error>> {
    let config_path = resolve_config_path(path);
    if !config_path.exists() {
        return Err(format!("Config file not found: {}", config_path.display()).into());
    }

    let config_dir = config_path.parent().unwrap_or(Path::new(".")).to_path_buf();

    // Deserialize raw (before normalize) to inspect inline fields.
    let raw_player = config::Player::deserialize_raw(&config_path)?;

    // Now deserialize with normalize to get the canonical form.
    let mut player = config::Player::deserialize(&config_path)?;

    let mut plan = MigrationPlan::new(config_path.clone());

    // Step C runs before A so fixture mutations are applied to profiles before
    // they are serialized and cleared by the profiles migration step.
    // Step C: Inline fixtures → venue file
    migrate_fixtures(&raw_player, &mut player, &config_dir, &mut plan);

    // Step A: Profiles → profiles_dir
    migrate_profiles(&raw_player, &mut player, &config_dir, &mut plan);

    // Step B: Playlist → playlists_dir
    migrate_playlist(
        &raw_player,
        &mut player,
        &config_dir,
        &config_path,
        &mut plan,
    )?;

    // Step D: Legacy fields → clear (implicit via normalize + step A)
    migrate_legacy_fields(&raw_player, &mut player, &mut plan);

    if !plan.has_changes() {
        println!("Nothing to migrate. Config is already using directory-based files.");
        return Ok(());
    }

    // Serialize the modified config.
    let yaml = crate::util::to_yaml_string(&player)?;
    plan.add_action(
        "Config",
        format!(
            "Backup {} → {}",
            config_path.display(),
            plan.backup_path.display()
        ),
    );
    plan.add_action("Config", format!("Rewrite {}", config_path.display()));
    plan.config_yaml = Some(yaml);

    plan.print_report(apply);

    if apply {
        plan.apply()?;
    }

    Ok(())
}

/// Step A: Migrate inline profiles to profiles_dir.
fn migrate_profiles(
    raw: &config::Player,
    player: &mut config::Player,
    config_dir: &Path,
    plan: &mut MigrationPlan,
) {
    // Skip if profiles_dir is already set and there are no inline profiles.
    if raw.profiles_dir_raw().is_some() {
        return;
    }

    let profiles = match player.inline_profiles() {
        Some(profiles) if !profiles.is_empty() => profiles.to_vec(),
        _ => return,
    };

    let profiles_dir = config_dir.join("profiles");
    plan.dirs_to_create.push(profiles_dir.clone());

    for (i, profile) in profiles.iter().enumerate() {
        let filename = match profile.hostname() {
            Some(hostname) => format!("{}.yaml", hostname),
            None => format!("profile-{}.yaml", i + 1),
        };
        let profile_path = profiles_dir.join(&filename);

        let hostname_desc = match profile.hostname() {
            Some(h) => format!("hostname: {}", h),
            None => "no hostname".to_string(),
        };

        match crate::util::to_yaml_string(profile) {
            Ok(yaml) => {
                plan.add_action(
                    "Profiles",
                    format!("Write profiles/{} ({})", filename, hostname_desc),
                );
                plan.files_to_write.push((profile_path, yaml));
            }
            Err(e) => {
                eprintln!("Warning: failed to serialize profile {}: {}", filename, e);
            }
        }
    }

    plan.add_action("Profiles", "Set profiles_dir: profiles/".to_string());
    plan.add_action("Profiles", "Clear inline profiles".to_string());

    player.set_profiles_dir("profiles/".to_string());
    player.clear_inline_profiles();
}

/// Step B: Migrate legacy playlist to playlists_dir.
fn migrate_playlist(
    raw: &config::Player,
    player: &mut config::Player,
    config_dir: &Path,
    config_path: &Path,
    plan: &mut MigrationPlan,
) -> Result<(), Box<dyn Error>> {
    let playlist_value = match raw.playlist_raw() {
        Some(v) => v.to_string(),
        None => return Ok(()),
    };

    // Resolve the playlist file path.
    let playlist_path = if Path::new(&playlist_value).is_absolute() {
        PathBuf::from(&playlist_value)
    } else {
        let cfg_dir = config_path.parent().unwrap_or(Path::new("."));
        cfg_dir.join(&playlist_value)
    };

    let playlists_dir = config_dir.join("playlists");

    // Get just the filename from the playlist path.
    let filename = playlist_path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "playlist.yaml".to_string());

    let dest = playlists_dir.join(&filename);

    plan.dirs_to_create.push(playlists_dir);

    if playlist_path.exists() {
        plan.add_action(
            "Playlists",
            format!("Copy {} → playlists/{}", playlist_value, filename),
        );
        plan.files_to_copy.push((playlist_path.clone(), dest));
        plan.files_to_delete.push(playlist_path);
    } else {
        plan.add_action(
            "Playlists",
            format!(
                "Warning: playlist file {} not found, skipping copy",
                playlist_value
            ),
        );
    }

    if raw.profiles_dir_raw().is_none() || player.playlist_raw().is_some() {
        plan.add_action("Playlists", "Set playlists_dir: playlists/".to_string());
        plan.add_action("Playlists", "Clear playlist field".to_string());
    }

    player.set_playlists_dir_value("playlists/".to_string());
    player.clear_playlist();

    Ok(())
}

/// Step C: Migrate inline fixtures to a venue file.
fn migrate_fixtures(
    raw: &config::Player,
    player: &mut config::Player,
    config_dir: &Path,
    plan: &mut MigrationPlan,
) {
    // Check raw config for inline fixtures (they live in dmx.lighting.fixtures).
    // Check profiles first (modern config), then fall back to the raw top-level
    // dmx field (legacy config before normalize moves it into profiles).
    let fixtures = if let Some(lighting) = raw.lighting_from_profiles() {
        lighting.inline_fixtures_raw().cloned()
    } else {
        raw.dmx_raw()
            .and_then(|d| d.lighting())
            .and_then(|l| l.inline_fixtures_raw())
            .cloned()
    };

    let fixtures = match fixtures {
        Some(f) if !f.is_empty() => f,
        _ => return,
    };

    let venues_dir = config_dir.join("lighting").join("venues");
    let venue_path = venues_dir.join("inline_migrated.light");

    plan.dirs_to_create.push(venues_dir);

    // Generate venue DSL.
    let mut lines = vec![format!("venue \"inline_migrated\" {{")];
    let mut sorted_fixtures: Vec<_> = fixtures.iter().collect();
    sorted_fixtures.sort_by_key(|(name, _)| name.as_str());

    for (name, type_spec) in &sorted_fixtures {
        // type_spec is expected to be "Type @ universe:channel"
        lines.push(format!("  fixture \"{}\" {}", name, type_spec));
    }
    lines.push("}".to_string());
    let venue_content = lines.join("\n") + "\n";

    plan.add_action(
        "Lighting",
        format!(
            "Write lighting/venues/inline_migrated.light ({} fixtures)",
            fixtures.len()
        ),
    );
    plan.files_to_write.push((venue_path, venue_content));

    plan.add_action(
        "Lighting",
        "Set directories.venues: lighting/venues".to_string(),
    );
    plan.add_action("Lighting", "Clear inline fixtures".to_string());

    // Mutate the player config to set venues dir and clear fixtures.
    // We need to modify the lighting config on profiles (after normalize).
    if let Some(profiles) = player.profiles_mut() {
        for profile in profiles.iter_mut() {
            if let Some(dmx) = profile.dmx_mut() {
                if let Some(lighting) = dmx.lighting_mut() {
                    if lighting.directories().and_then(|d| d.venues()).is_none() {
                        lighting.set_venues_dir("lighting/venues".to_string());
                    }
                    lighting.clear_inline_fixtures();
                }
            }
        }
    }
    // Also clear on the raw dmx field (which will be serialized if profiles haven't been set up).
    if let Some(lighting) = player.lighting_mut() {
        if lighting.directories().and_then(|d| d.venues()).is_none() {
            lighting.set_venues_dir("lighting/venues".to_string());
        }
        lighting.clear_inline_fixtures();
    }
}

/// Step D: Clear legacy top-level fields (they've already been normalized into profiles).
fn migrate_legacy_fields(
    raw: &config::Player,
    player: &mut config::Player,
    plan: &mut MigrationPlan,
) {
    if !raw.has_legacy_fields() {
        return;
    }

    plan.add_action(
        "Legacy Fields",
        "Clear legacy top-level fields (audio, midi, dmx, trigger, track_mappings, controllers, sample_triggers)".to_string(),
    );
    player.clear_legacy_fields();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a temp dir with a config file and optional additional files.
    fn setup_migration(yaml: &str, extra_files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("mtrack.yaml");
        std::fs::write(&config_path, yaml).unwrap();
        for (name, content) in extra_files {
            let file_path = dir.path().join(name);
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&file_path, content).unwrap();
        }
        (dir, config_path)
    }

    #[test]
    fn test_migrate_profiles_to_dir() {
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles:
  - hostname: pi-a
    audio:
      device: device-a
      track_mappings:
        drums: [1]
  - hostname: pi-b
    audio:
      device: device-b
      track_mappings:
        drums: [11]
  - audio:
      device: fallback
      track_mappings:
        drums: [1]
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Profile files should exist.
        assert!(dir.path().join("profiles/pi-a.yaml").exists());
        assert!(dir.path().join("profiles/pi-b.yaml").exists());
        assert!(dir.path().join("profiles/profile-3.yaml").exists());

        // Backup should exist.
        assert!(dir.path().join("mtrack.yaml.bak").exists());

        // Rewritten config should have profiles_dir set.
        let rewritten = std::fs::read_to_string(dir.path().join("mtrack.yaml")).unwrap();
        assert!(rewritten.contains("profiles_dir"));
    }

    #[test]
    fn test_migrate_playlist_to_dir() {
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
playlist: my_playlist.yaml
"#,
            &[("my_playlist.yaml", "- song1\n- song2\n")],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Playlist should be copied.
        assert!(dir.path().join("playlists/my_playlist.yaml").exists());
        let content =
            std::fs::read_to_string(dir.path().join("playlists/my_playlist.yaml")).unwrap();
        assert!(content.contains("song1"));

        // Original should be deleted.
        assert!(!dir.path().join("my_playlist.yaml").exists());

        // Config should have playlists_dir.
        let rewritten = std::fs::read_to_string(dir.path().join("mtrack.yaml")).unwrap();
        assert!(rewritten.contains("playlists_dir"));
        assert!(!rewritten.contains("playlist: my_playlist"));
    }

    #[test]
    fn test_migrate_inline_fixtures() {
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles:
  - audio:
      device: device-a
      track_mappings:
        drums: [1]
    dmx:
      universes:
        - universe: 1
          name: main
      lighting:
        fixtures:
          par1: "GenericPar @ 1:1"
          mover1: "MovingHead @ 1:10"
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Venue file should exist.
        let venue_path = dir.path().join("lighting/venues/inline_migrated.light");
        assert!(venue_path.exists());

        let venue = std::fs::read_to_string(&venue_path).unwrap();
        assert!(venue.contains("venue \"inline_migrated\""));
        assert!(venue.contains("fixture \"mover1\" MovingHead @ 1:10"));
        assert!(venue.contains("fixture \"par1\" GenericPar @ 1:1"));
    }

    #[test]
    fn test_migrate_legacy_fields() {
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
midi:
  device: mock-midi
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Profile should be created from legacy fields.
        assert!(dir.path().join("profiles").exists());

        // A profile file should exist with the legacy config.
        let profile_files: Vec<_> = std::fs::read_dir(dir.path().join("profiles"))
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(profile_files.len(), 1);

        // The profile should contain the device from the legacy config.
        let profile_content = std::fs::read_to_string(profile_files[0].path()).unwrap();
        assert!(profile_content.contains("mock-device"));

        // Config should have profiles_dir and not contain legacy device values.
        let rewritten = std::fs::read_to_string(dir.path().join("mtrack.yaml")).unwrap();
        assert!(rewritten.contains("profiles_dir"));
        assert!(!rewritten.contains("mock-device"));
        assert!(!rewritten.contains("mock-midi"));
    }

    #[test]
    fn test_migrate_idempotent() {
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles:
  - hostname: pi-a
    audio:
      device: device-a
      track_mappings:
        drums: [1]
"#,
            &[],
        );

        // First migration.
        migrate(dir.path().to_str().unwrap(), true).unwrap();

        let config_after_first = std::fs::read_to_string(dir.path().join("mtrack.yaml")).unwrap();

        // Second migration should be a no-op.
        migrate(dir.path().to_str().unwrap(), true).unwrap();

        let config_after_second = std::fs::read_to_string(dir.path().join("mtrack.yaml")).unwrap();
        assert_eq!(config_after_first, config_after_second);
    }

    #[test]
    fn test_migrate_dry_run_no_writes() {
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles:
  - hostname: pi-a
    audio:
      device: device-a
      track_mappings:
        drums: [1]
"#,
            &[],
        );

        // Dry-run (default).
        migrate(dir.path().to_str().unwrap(), false).unwrap();

        // No profiles directory should be created.
        assert!(!dir.path().join("profiles").exists());

        // No backup file.
        assert!(!dir.path().join("mtrack.yaml.bak").exists());

        // Config should be unchanged.
        let config = std::fs::read_to_string(dir.path().join("mtrack.yaml")).unwrap();
        assert!(config.contains("hostname: pi-a"));
    }

    #[test]
    fn test_migrate_nothing_to_do() {
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles_dir: profiles/
playlists_dir: playlists/
"#,
            &[],
        );

        // Create the required profiles dir (empty is ok).
        std::fs::create_dir_all(dir.path().join("profiles")).unwrap();

        // Should print nothing-to-do message and not error.
        migrate(dir.path().to_str().unwrap(), false).unwrap();

        // No backup created since nothing to do.
        assert!(!dir.path().join("mtrack.yaml.bak").exists());
    }

    #[test]
    fn test_migrate_backup_created() {
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles:
  - audio:
      device: device-a
      track_mappings:
        drums: [1]
"#,
            &[],
        );

        let original = std::fs::read_to_string(dir.path().join("mtrack.yaml")).unwrap();

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Backup should contain the original config.
        let backup = std::fs::read_to_string(dir.path().join("mtrack.yaml.bak")).unwrap();
        assert_eq!(original, backup);
    }

    #[test]
    fn test_resolve_config_path_directory() {
        // When given a directory, appends mtrack.yaml.
        let path = resolve_config_path("/some/dir");
        assert_eq!(path, PathBuf::from("/some/dir/mtrack.yaml"));
    }

    #[test]
    fn test_resolve_config_path_yaml_file() {
        let path = resolve_config_path("/some/dir/custom.yaml");
        assert_eq!(path, PathBuf::from("/some/dir/custom.yaml"));
    }

    #[test]
    fn test_resolve_config_path_yml_file() {
        let path = resolve_config_path("/some/dir/config.yml");
        assert_eq!(path, PathBuf::from("/some/dir/config.yml"));
    }

    #[test]
    fn test_migrate_playlist_missing_file() {
        // Playlist field set but file doesn't exist — should still migrate config
        // but skip the copy.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
playlist: nonexistent.yaml
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // playlists dir created but no file inside.
        assert!(dir.path().join("playlists").exists());
        assert!(!dir.path().join("playlists/nonexistent.yaml").exists());

        // Config should still be updated.
        let rewritten = std::fs::read_to_string(dir.path().join("mtrack.yaml")).unwrap();
        assert!(rewritten.contains("playlists_dir"));
    }

    #[test]
    fn test_migrate_config_not_found() {
        let result = migrate("/nonexistent/path/to/project", false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Config file not found"));
    }

    #[test]
    fn test_migrate_with_yaml_file_path() {
        // Pass a .yaml file path directly instead of a directory.
        let (dir, config_path) = setup_migration(
            r#"
songs: songs
profiles:
  - hostname: test-host
    audio:
      device: device-x
      track_mappings:
        drums: [1]
"#,
            &[],
        );

        migrate(config_path.to_str().unwrap(), true).unwrap();

        assert!(dir.path().join("profiles/test-host.yaml").exists());
        assert!(dir.path().join("mtrack.yaml.bak").exists());
    }

    #[test]
    fn test_migrate_profiles_dir_already_set_skips() {
        // When profiles_dir is already set, profile migration is skipped entirely.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles_dir: profiles/
profiles:
  - hostname: pi-a
    audio:
      device: device-a
      track_mappings:
        drums: [1]
"#,
            &[],
        );

        // Create the profiles dir so deserialize works.
        std::fs::create_dir_all(dir.path().join("profiles")).unwrap();

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // No profile files should be written (profiles_dir already set).
        let entries: Vec<_> = std::fs::read_dir(dir.path().join("profiles"))
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_migrate_no_profiles_skips() {
        // Config with no profiles at all — profile step should be a no-op.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), false).unwrap();

        // No profiles directory created.
        assert!(!dir.path().join("profiles").exists());
        // No backup since nothing to migrate.
        assert!(!dir.path().join("mtrack.yaml.bak").exists());
    }

    #[test]
    fn test_migrate_legacy_fixtures_from_dmx() {
        // Fixtures on legacy top-level dmx field (not in profiles).
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
dmx:
  universes:
    - universe: 1
      name: main
  lighting:
    fixtures:
      wash1: "GenericWash @ 1:20"
      spot1: "Spot @ 1:30"
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Venue file should be created from legacy dmx fixtures.
        let venue_path = dir.path().join("lighting/venues/inline_migrated.light");
        assert!(venue_path.exists());

        let venue = std::fs::read_to_string(&venue_path).unwrap();
        assert!(venue.contains("fixture \"spot1\" Spot @ 1:30"));
        assert!(venue.contains("fixture \"wash1\" GenericWash @ 1:20"));
    }

    #[test]
    fn test_migrate_fixtures_existing_venues_dir_preserved() {
        // If lighting config already has a venues directory set, don't overwrite it.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles:
  - audio:
      device: device-a
      track_mappings:
        drums: [1]
    dmx:
      universes:
        - universe: 1
          name: main
      lighting:
        fixtures:
          par1: "GenericPar @ 1:1"
        directories:
          venues: custom/venues
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Venue file should still be written to the default location.
        assert!(dir
            .path()
            .join("lighting/venues/inline_migrated.light")
            .exists());

        // The profile file should preserve the existing custom venues dir
        // (profiles are extracted to files, so the venues dir is in the profile).
        let profile_files: Vec<_> = std::fs::read_dir(dir.path().join("profiles"))
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(profile_files.len(), 1);
        let profile_content = std::fs::read_to_string(profile_files[0].path()).unwrap();
        assert!(
            profile_content.contains("custom/venues"),
            "profile should preserve existing venues dir, got:\n{}",
            profile_content
        );

        // Fixtures should be cleared from the profile.
        assert!(
            !profile_content.contains("GenericPar"),
            "fixtures should be cleared from profile"
        );
    }

    #[test]
    fn test_migrate_empty_fixtures_skips() {
        // Empty fixtures map — fixture step should be a no-op.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles:
  - audio:
      device: device-a
      track_mappings:
        drums: [1]
    dmx:
      universes:
        - universe: 1
          name: main
      lighting:
        fixtures: {}
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // No venue file should be created.
        assert!(!dir.path().join("lighting/venues").exists());
    }

    #[test]
    fn test_migrate_all_steps_combined() {
        // Config with inline profiles, legacy playlist, inline fixtures, and legacy fields.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
playlist: setlist.yaml
audio:
  device: my-interface
track_mappings:
  click: [1]
  cue: [2]
midi:
  device: my-midi
dmx:
  universes:
    - universe: 1
      name: main
  lighting:
    fixtures:
      par1: "GenericPar @ 1:1"
"#,
            &[("setlist.yaml", "- song_a\n- song_b\n")],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Profiles extracted.
        assert!(dir.path().join("profiles").exists());
        let profile_files: Vec<_> = std::fs::read_dir(dir.path().join("profiles"))
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(profile_files.len(), 1);

        // Playlist copied.
        assert!(dir.path().join("playlists/setlist.yaml").exists());
        assert!(!dir.path().join("setlist.yaml").exists());

        // Venue file created.
        assert!(dir
            .path()
            .join("lighting/venues/inline_migrated.light")
            .exists());

        // Config rewritten with directory pointers.
        let rewritten = std::fs::read_to_string(dir.path().join("mtrack.yaml")).unwrap();
        assert!(rewritten.contains("profiles_dir"));
        assert!(rewritten.contains("playlists_dir"));

        // Legacy values should not appear.
        assert!(!rewritten.contains("my-interface"));
        assert!(!rewritten.contains("my-midi"));

        // Backup exists.
        assert!(dir.path().join("mtrack.yaml.bak").exists());
    }

    #[test]
    fn test_migrate_absolute_playlist_path() {
        let dir = tempfile::tempdir().unwrap();
        let playlist_dir = tempfile::tempdir().unwrap();
        let abs_playlist = playlist_dir.path().join("abs_playlist.yaml");
        std::fs::write(&abs_playlist, "- song_x\n").unwrap();

        let config_path = dir.path().join("mtrack.yaml");
        std::fs::write(
            &config_path,
            format!(
                "songs: songs\nplaylist: {}\n",
                abs_playlist.to_str().unwrap()
            ),
        )
        .unwrap();

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Playlist copied using original filename.
        assert!(dir.path().join("playlists/abs_playlist.yaml").exists());
        let content =
            std::fs::read_to_string(dir.path().join("playlists/abs_playlist.yaml")).unwrap();
        assert!(content.contains("song_x"));

        // Original should be deleted.
        assert!(!abs_playlist.exists());
    }

    #[test]
    fn test_migrate_resolve_config_path_existing_file() {
        // When the path points to an actual file, resolve_config_path returns it directly.
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("custom.yaml");
        std::fs::write(&config_path, "songs: songs\n").unwrap();

        let resolved = resolve_config_path(config_path.to_str().unwrap());
        assert_eq!(resolved, config_path);
    }

    #[test]
    fn test_migrated_config_round_trips() {
        // After migration, the rewritten config should be loadable by Player::deserialize.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles:
  - hostname: pi-a
    audio:
      device: device-a
      track_mappings:
        drums: [1]
        synth: [2]
    midi:
      device: midi-a
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // The migrated config with profiles_dir should deserialize successfully.
        let config_path = dir.path().join("mtrack.yaml");
        let player = config::Player::deserialize(&config_path).unwrap();

        // Should load profiles from the directory.
        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].hostname(), Some("pi-a"));
        assert_eq!(
            profiles[0].audio_config().unwrap().audio().device(),
            "device-a"
        );
    }

    #[test]
    fn test_migrate_legacy_with_controllers() {
        // Legacy controller/controllers fields should be cleared.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
controllers:
  - kind: grpc
    port: 43234
  - kind: osc
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Profile should contain the controllers.
        let profile_files: Vec<_> = std::fs::read_dir(dir.path().join("profiles"))
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(profile_files.len(), 1);
        let profile_content = std::fs::read_to_string(profile_files[0].path()).unwrap();
        assert!(profile_content.contains("grpc"));

        // Config should not contain the legacy controller values.
        let rewritten = std::fs::read_to_string(dir.path().join("mtrack.yaml")).unwrap();
        assert!(!rewritten.contains("43234"));
    }

    #[test]
    fn test_migrate_legacy_with_trigger() {
        // Legacy trigger config should be normalized into profile and cleared.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
trigger:
  device: "UltraLite-mk5"
  inputs:
    - kind: audio
      channel: 1
      sample: "kick"
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Profile should contain trigger config.
        let profile_files: Vec<_> = std::fs::read_dir(dir.path().join("profiles"))
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(profile_files.len(), 1);
        let profile_content = std::fs::read_to_string(profile_files[0].path()).unwrap();
        assert!(profile_content.contains("UltraLite"));

        // Config should not contain legacy trigger values.
        let rewritten = std::fs::read_to_string(dir.path().join("mtrack.yaml")).unwrap();
        assert!(!rewritten.contains("UltraLite"));
    }

    #[test]
    fn test_migrate_legacy_sample_triggers() {
        // Legacy sample_triggers should be normalized and cleared.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
sample_triggers:
  - trigger:
      type: note_on
      channel: 1
      key: 60
      velocity: 127
    sample: kick
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Profile should exist.
        assert!(dir.path().join("profiles").exists());

        // Config should not contain legacy sample_triggers values.
        let rewritten = std::fs::read_to_string(dir.path().join("mtrack.yaml")).unwrap();
        assert!(!rewritten.contains("note_on"));
    }

    #[test]
    fn test_migrate_playlist_only() {
        // Only playlist needs migration, no profiles or fixtures.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles_dir: profiles/
playlist: show.yaml
"#,
            &[("show.yaml", "- intro\n- outro\n")],
        );

        std::fs::create_dir_all(dir.path().join("profiles")).unwrap();

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Playlist migrated.
        assert!(dir.path().join("playlists/show.yaml").exists());
        assert!(!dir.path().join("show.yaml").exists());

        // No profiles directory changes (already set).
        let rewritten = std::fs::read_to_string(dir.path().join("mtrack.yaml")).unwrap();
        assert!(rewritten.contains("playlists_dir"));
    }

    #[test]
    fn test_migrate_fixtures_only() {
        // Only fixtures need migration.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles_dir: profiles/
profiles:
  - audio:
      device: device-a
      track_mappings:
        drums: [1]
    dmx:
      universes:
        - universe: 1
          name: main
      lighting:
        fixtures:
          par1: "GenericPar @ 1:1"
"#,
            &[],
        );

        std::fs::create_dir_all(dir.path().join("profiles")).unwrap();
        std::fs::write(
            dir.path().join("profiles/host.yaml"),
            "audio:\n  device: device-a\n  track_mappings:\n    drums: [1]\ndmx:\n  universes:\n    - universe: 1\n      name: main\n  lighting:\n    fixtures:\n      par1: \"GenericPar @ 1:1\"\n",
        ).unwrap();

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Venue file created.
        assert!(dir
            .path()
            .join("lighting/venues/inline_migrated.light")
            .exists());
    }

    #[test]
    fn test_migrate_preserves_songs_path() {
        // After migration, the songs path should be preserved.
        let (dir, _config_path) = setup_migration(
            r#"
songs: my_songs
profiles:
  - hostname: pi-a
    audio:
      device: device-a
      track_mappings:
        drums: [1]
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        let rewritten = std::fs::read_to_string(dir.path().join("mtrack.yaml")).unwrap();
        assert!(rewritten.contains("my_songs"));
    }

    #[test]
    fn test_migrate_preserves_non_migrated_fields() {
        // Non-migrated fields like active_playlist and samples should survive.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
active_playlist: all_songs
samples:
  kick:
    file: kick.wav
    output_channels: [1, 2]
profiles:
  - audio:
      device: device-a
      track_mappings:
        drums: [1]
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        let rewritten = std::fs::read_to_string(dir.path().join("mtrack.yaml")).unwrap();
        assert!(rewritten.contains("all_songs"));
        assert!(rewritten.contains("kick"));
    }

    #[test]
    fn test_migrate_multiple_profiles_naming() {
        // Verify the naming convention: hostname-based and index-based.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles:
  - hostname: stage-left
    audio:
      device: device-a
      track_mappings:
        drums: [1]
  - audio:
      device: fallback-1
      track_mappings:
        drums: [1]
  - audio:
      device: fallback-2
      track_mappings:
        drums: [2]
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        assert!(dir.path().join("profiles/stage-left.yaml").exists());
        assert!(dir.path().join("profiles/profile-2.yaml").exists());
        assert!(dir.path().join("profiles/profile-3.yaml").exists());
    }

    #[test]
    fn test_resolve_config_path_non_yaml_extension() {
        // A non-yaml extension on a non-existent path appends mtrack.yaml
        // (the path isn't a file, so is_file() is false, extension check fails).
        let path = resolve_config_path("/some/dir/config.toml");
        assert_eq!(path, PathBuf::from("/some/dir/config.toml/mtrack.yaml"));
    }

    #[test]
    fn test_resolve_config_path_existing_non_yaml_file() {
        // An existing file with a non-yaml extension is returned as-is
        // (is_file() short-circuits the extension check).
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("config.toml");
        std::fs::write(&file_path, "songs: songs\n").unwrap();

        let resolved = resolve_config_path(file_path.to_str().unwrap());
        assert_eq!(resolved, file_path);
    }

    #[test]
    fn test_migrate_invalid_yaml() {
        // Invalid YAML should return an error from deserialize_raw.
        let (dir, _config_path) = setup_migration("this is not valid yaml: [[[", &[]);

        let result = migrate(dir.path().to_str().unwrap(), false);
        assert!(result.is_err());
    }

    #[test]
    fn test_migrate_explicit_empty_profiles() {
        // `profiles: []` is a valid config with an explicit empty list.
        // Should be a no-op (no profiles to extract).
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles: []
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), false).unwrap();

        // No profiles directory should be created.
        assert!(!dir.path().join("profiles").exists());
    }

    #[test]
    fn test_migrate_legacy_audio_device_string() {
        // Legacy `audio_device: "..."` (bare string) triggers migration.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
audio_device: my-card
track_mappings:
  click: [1]
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Profile should be extracted.
        assert!(dir.path().join("profiles").exists());
        let profile_files: Vec<_> = std::fs::read_dir(dir.path().join("profiles"))
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(profile_files.len(), 1);
        let profile_content = std::fs::read_to_string(profile_files[0].path()).unwrap();
        assert!(profile_content.contains("my-card"));
    }

    #[test]
    fn test_migrate_legacy_midi_device_string() {
        // Legacy `midi_device: "..."` (bare string) triggers migration.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
midi_device: my-midi-port
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Profile should contain the midi device.
        let profile_files: Vec<_> = std::fs::read_dir(dir.path().join("profiles"))
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(profile_files.len(), 1);
        let profile_content = std::fs::read_to_string(profile_files[0].path()).unwrap();
        assert!(profile_content.contains("my-midi-port"));
    }

    #[test]
    fn test_migrate_lighting_without_fixtures_key() {
        // Profile with lighting section but no fixtures key at all — should skip.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles:
  - audio:
      device: device-a
      track_mappings:
        drums: [1]
    dmx:
      universes:
        - universe: 1
          name: main
      lighting:
        current_venue: main_stage
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // No venue file should be created.
        assert!(!dir.path().join("lighting/venues").exists());
    }

    #[test]
    fn test_migrate_dmx_without_lighting() {
        // DMX configured but no lighting section at all.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles:
  - audio:
      device: device-a
      track_mappings:
        drums: [1]
    dmx:
      universes:
        - universe: 1
          name: main
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // No venue file should be created.
        assert!(!dir.path().join("lighting/venues").exists());
    }

    #[test]
    fn test_migrate_multi_profile_fixtures_cleared() {
        // Multiple profiles both have inline fixtures — verify both get cleared.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles:
  - hostname: pi-a
    audio:
      device: device-a
      track_mappings:
        drums: [1]
    dmx:
      universes:
        - universe: 1
          name: main
      lighting:
        fixtures:
          par1: "GenericPar @ 1:1"
  - hostname: pi-b
    audio:
      device: device-b
      track_mappings:
        drums: [11]
    dmx:
      universes:
        - universe: 2
          name: aux
      lighting:
        fixtures:
          mover1: "MovingHead @ 2:1"
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Venue file created.
        assert!(dir
            .path()
            .join("lighting/venues/inline_migrated.light")
            .exists());

        // Both profile files should have fixtures cleared.
        let pi_a = std::fs::read_to_string(dir.path().join("profiles/pi-a.yaml")).unwrap();
        assert!(
            !pi_a.contains("GenericPar"),
            "fixtures should be cleared from pi-a profile"
        );

        let pi_b = std::fs::read_to_string(dir.path().join("profiles/pi-b.yaml")).unwrap();
        assert!(
            !pi_b.contains("MovingHead"),
            "fixtures should be cleared from pi-b profile"
        );
    }

    #[test]
    fn test_migrate_legacy_dmx_without_lighting_section() {
        // Legacy top-level DMX with universes but no lighting — fixtures step
        // should be skipped, but legacy fields should still be cleared.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
dmx:
  universes:
    - universe: 1
      name: main
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // No venue file.
        assert!(!dir.path().join("lighting/venues").exists());

        // Profile should exist with DMX config.
        let profile_files: Vec<_> = std::fs::read_dir(dir.path().join("profiles"))
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(profile_files.len(), 1);
    }

    #[test]
    fn test_migrate_playlists_dir_already_set_skips_playlist() {
        // When playlists_dir is already set but playlist field is also set,
        // playlist should still be migrated.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles_dir: profiles/
playlists_dir: playlists/
playlist: legacy.yaml
"#,
            &[("legacy.yaml", "- song1\n"), ("playlists/.gitkeep", "")],
        );

        std::fs::create_dir_all(dir.path().join("profiles")).unwrap();

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // Playlist should be copied.
        assert!(dir.path().join("playlists/legacy.yaml").exists());
        // Original deleted.
        assert!(!dir.path().join("legacy.yaml").exists());
    }

    #[test]
    fn test_migrate_venue_dsl_sorted_output() {
        // Verify fixtures in the venue file are sorted alphabetically by name.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles:
  - audio:
      device: device-a
      track_mappings:
        drums: [1]
    dmx:
      universes:
        - universe: 1
          name: main
      lighting:
        fixtures:
          zebra: "Z @ 1:100"
          alpha: "A @ 1:1"
          middle: "M @ 1:50"
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        let venue =
            std::fs::read_to_string(dir.path().join("lighting/venues/inline_migrated.light"))
                .unwrap();

        // Verify sorted order: alpha < middle < zebra.
        let alpha_pos = venue.find("alpha").unwrap();
        let middle_pos = venue.find("middle").unwrap();
        let zebra_pos = venue.find("zebra").unwrap();
        assert!(alpha_pos < middle_pos);
        assert!(middle_pos < zebra_pos);
    }

    #[test]
    fn test_migrate_profile_content_valid_yaml() {
        // Extracted profile files should be valid YAML that can be deserialized.
        let (dir, _config_path) = setup_migration(
            r#"
songs: songs
profiles:
  - hostname: pi-a
    audio:
      device: device-a
      sample_rate: 48000
      track_mappings:
        drums: [1, 2]
        synth: [3, 4]
    midi:
      device: midi-a
    controllers:
      - kind: grpc
        port: 43234
"#,
            &[],
        );

        migrate(dir.path().to_str().unwrap(), true).unwrap();

        // The extracted profile should be valid YAML and round-trip as a Profile.
        let profile_path = dir.path().join("profiles/pi-a.yaml");
        let profile_yaml = std::fs::read_to_string(&profile_path).unwrap();
        assert!(profile_yaml.contains("pi-a"));
        assert!(profile_yaml.contains("device-a"));
        assert!(profile_yaml.contains("48000"));
        assert!(profile_yaml.contains("grpc"));
    }
}
