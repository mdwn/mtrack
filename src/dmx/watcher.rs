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

use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use parking_lot::Mutex;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::lighting::system::LightingSystem;
use crate::lighting::timeline::LightingTimeline;
use crate::lighting::validation::validate_light_shows;
use crate::lighting::EffectEngine;

/// Handle to a running file watcher, shuts down when dropped.
pub struct WatcherHandle {
    _watcher: notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>,
}

/// Starts watching the given light show files for changes.
///
/// On file change (debounced 300ms):
/// 1. Re-parses the .light file
/// 2. Builds a new LightingTimeline
/// 3. Swaps it into current_song_timeline
/// 4. Reconstructs state at current_song_time
/// 5. Sends a reload notification over WebSocket
pub fn start_watching(
    file_paths: Vec<PathBuf>,
    effect_engine: Arc<Mutex<EffectEngine>>,
    current_song_timeline: Arc<Mutex<Option<LightingTimeline>>>,
    current_song_time: Arc<Mutex<Duration>>,
    lighting_system: Option<Arc<Mutex<LightingSystem>>>,
    lighting_config: Option<crate::config::Lighting>,
    broadcast_tx: broadcast::Sender<String>,
) -> Result<WatcherHandle, Box<dyn std::error::Error>> {
    // Canonicalize paths so they match what the OS reports in events.
    let paths: Vec<PathBuf> = file_paths
        .iter()
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
        .collect();

    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(Duration::from_millis(300), tx)?;

    // Watch each file path
    for path in &paths {
        if let Some(parent) = path.parent() {
            debouncer
                .watcher()
                .watch(parent, notify::RecursiveMode::NonRecursive)?;
        }
    }

    // Spawn a thread to handle file change events
    let effect_engine = effect_engine.clone();
    let current_song_timeline = current_song_timeline.clone();
    let current_song_time = current_song_time.clone();

    std::thread::spawn(move || {
        for events in rx {
            match events {
                Ok(events) => {
                    // Check if any of our watched files changed
                    let relevant = events.iter().any(|event| {
                        if event.kind != DebouncedEventKind::Any {
                            return false;
                        }
                        let event_path = event
                            .path
                            .canonicalize()
                            .unwrap_or_else(|_| event.path.clone());
                        paths.contains(&event_path)
                    });
                    if !relevant {
                        continue;
                    }

                    info!("Light show file changed, reloading...");

                    match reload_timeline(
                        &paths,
                        &effect_engine,
                        &current_song_timeline,
                        &current_song_time,
                        lighting_system.as_ref(),
                        lighting_config.as_ref(),
                    ) {
                        Ok(()) => {
                            info!("Light show reloaded successfully");
                            let msg = json!({
                                "type": "reload",
                                "status": "ok",
                            });
                            let _ = broadcast_tx.send(msg.to_string());
                        }
                        Err(e) => {
                            warn!("Light show reload failed: {}", e);
                            let msg = json!({
                                "type": "reload",
                                "status": "error",
                                "error": e.to_string(),
                            });
                            let _ = broadcast_tx.send(msg.to_string());
                        }
                    }
                }
                Err(e) => {
                    error!("File watcher error: {:?}", e);
                }
            }
        }
    });

    Ok(WatcherHandle {
        _watcher: debouncer,
    })
}

/// Re-parses light show files, builds a new timeline, and swaps it in.
fn reload_timeline(
    file_paths: &[PathBuf],
    effect_engine: &Arc<Mutex<EffectEngine>>,
    current_song_timeline: &Arc<Mutex<Option<LightingTimeline>>>,
    current_song_time: &Arc<Mutex<Duration>>,
    lighting_system: Option<&Arc<Mutex<LightingSystem>>>,
    lighting_config: Option<&crate::config::Lighting>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut all_shows = Vec::new();

    for path in file_paths {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

        let shows = crate::lighting::parser::parse_light_shows(&content)
            .map_err(|e| format!("Parse error in {}: {}", path.display(), e))?;

        // Validate if lighting config is available
        if let Some(lc) = lighting_config {
            validate_light_shows(&shows, Some(lc))
                .map_err(|e| format!("Validation error in {}: {}", path.display(), e))?;
        }

        for (_, show) in shows {
            all_shows.push(show);
        }
    }

    if all_shows.is_empty() {
        return Err("No shows found after re-parse".into());
    }

    let mut new_timeline = LightingTimeline::new(all_shows);

    // Get current song time
    let song_time = { *current_song_time.lock() };

    // Calculate the timeline update before acquiring the engine lock.
    let timeline_update = if song_time > Duration::ZERO {
        new_timeline.start_at(song_time)
    } else {
        new_timeline.start();
        crate::lighting::timeline::TimelineUpdate::default()
    };

    // Pre-resolve group names while the engine lock is NOT held
    // to avoid deadlocking with the lighting system lock.
    let resolved_effects_with_elapsed: Vec<_> = {
        let mut sorted: Vec<_> = timeline_update.effects_with_elapsed.values().collect();
        sorted.sort_by_key(|(effect, _)| effect.cue_time.unwrap_or(Duration::ZERO));
        sorted
            .into_iter()
            .map(|(effect, elapsed)| {
                (
                    resolve_effect_groups(lighting_system, effect.clone()),
                    *elapsed,
                )
            })
            .collect()
    };
    let mut resolved_effects: Vec<_> = timeline_update.effects;
    resolved_effects.sort_by_key(|e| if e.id.starts_with("seq_") { 0 } else { 1 });
    let resolved_effects: Vec<_> = resolved_effects
        .into_iter()
        .map(|e| resolve_effect_groups(lighting_system, e))
        .collect();

    // Extract tempo map before moving new_timeline into the lock.
    let tempo_map = new_timeline.tempo_map().cloned();

    // Swap the timeline first, so the main effects loop won't generate
    // updates from the old timeline while the engine has new state.
    {
        let mut current = current_song_timeline.lock();
        *current = Some(new_timeline);
    }

    // Atomically stop + apply the new state under a single lock to prevent
    // the sampler loop from seeing an intermediate empty state (flash).
    {
        info!("Watcher acquiring effect_engine lock for reload...");
        let mut engine = effect_engine.lock();
        info!("Watcher acquired effect_engine lock.");
        engine.set_tempo_map(tempo_map);
        engine.stop_all_effects();

        if song_time > Duration::ZERO {
            for cmd in &timeline_update.layer_commands {
                engine.apply_layer_command(cmd);
            }
            for seq_name in &timeline_update.stop_sequences {
                engine.stop_sequence(seq_name);
            }
            for (effect, elapsed_time) in resolved_effects_with_elapsed {
                if let Err(e) = engine.start_effect_with_elapsed(effect, elapsed_time) {
                    error!("Failed to start lighting effect with elapsed time: {}", e);
                }
            }
        }
        for effect in resolved_effects {
            if let Err(e) = engine.start_effect(effect) {
                error!("Failed to start lighting effect: {}", e);
            }
        }
        info!("Watcher releasing effect_engine lock.");
    }

    Ok(())
}

/// Resolves group names in an effect's target_fixtures to actual fixture names.
fn resolve_effect_groups(
    lighting_system: Option<&Arc<Mutex<LightingSystem>>>,
    mut effect: crate::lighting::EffectInstance,
) -> crate::lighting::EffectInstance {
    if let Some(ls) = lighting_system {
        let mut system = ls.lock();
        let mut resolved_fixtures = Vec::new();
        for group_name in &effect.target_fixtures {
            let fixtures = system.resolve_logical_group_graceful(group_name);
            resolved_fixtures.extend(fixtures);
        }
        effect.target_fixtures = resolved_fixtures;
    }
    effect
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn reload_timeline_with_valid_file_at_zero() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
}"#,
        )
        .unwrap();

        let effect_engine = Arc::new(Mutex::new(EffectEngine::new()));
        let timeline = Arc::new(Mutex::new(None));
        let song_time = Arc::new(Mutex::new(Duration::ZERO));

        let result = reload_timeline(
            &[dsl_path],
            &effect_engine,
            &timeline,
            &song_time,
            None,
            None,
        );
        assert!(result.is_ok());
        assert!(timeline.lock().is_some());
    }

    #[test]
    fn reload_timeline_with_valid_file_at_nonzero() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
    @00:05.000
    front_wash: static color: "red", dimmer: 50%
}"#,
        )
        .unwrap();

        let effect_engine = Arc::new(Mutex::new(EffectEngine::new()));
        let timeline = Arc::new(Mutex::new(None));
        let song_time = Arc::new(Mutex::new(Duration::from_secs(3)));

        let result = reload_timeline(
            &[dsl_path],
            &effect_engine,
            &timeline,
            &song_time,
            None,
            None,
        );
        assert!(result.is_ok());
        assert!(timeline.lock().is_some());
    }

    #[test]
    fn reload_timeline_with_missing_file() {
        let effect_engine = Arc::new(Mutex::new(EffectEngine::new()));
        let timeline = Arc::new(Mutex::new(None));
        let song_time = Arc::new(Mutex::new(Duration::ZERO));

        let result = reload_timeline(
            &[PathBuf::from("/nonexistent/show.light")],
            &effect_engine,
            &timeline,
            &song_time,
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn reload_timeline_with_invalid_dsl() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("bad.light");
        std::fs::write(&dsl_path, "this is not valid DSL {").unwrap();

        let effect_engine = Arc::new(Mutex::new(EffectEngine::new()));
        let timeline = Arc::new(Mutex::new(None));
        let song_time = Arc::new(Mutex::new(Duration::ZERO));

        let result = reload_timeline(
            &[dsl_path],
            &effect_engine,
            &timeline,
            &song_time,
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn reload_timeline_with_validation() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
}"#,
        )
        .unwrap();

        // Lighting config with "front_wash" defined so validation passes
        let lighting_config = crate::config::Lighting::new(
            None,
            Some({
                let mut fixtures = std::collections::HashMap::new();
                fixtures.insert("front_wash".to_string(), "Generic_Dimmer @ 1:1".to_string());
                fixtures
            }),
            None,
            None,
        );

        let effect_engine = Arc::new(Mutex::new(EffectEngine::new()));
        let timeline = Arc::new(Mutex::new(None));
        let song_time = Arc::new(Mutex::new(Duration::ZERO));

        let result = reload_timeline(
            &[dsl_path],
            &effect_engine,
            &timeline,
            &song_time,
            None,
            Some(&lighting_config),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn reload_timeline_validation_failure() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            r#"show "test" {
    @00:00.000
    unknown_fixture: static color: "blue", dimmer: 100%
}"#,
        )
        .unwrap();

        // Lighting config WITHOUT "unknown_fixture" — validation will fail
        let lighting_config = crate::config::Lighting::new(
            None,
            Some({
                let mut fixtures = std::collections::HashMap::new();
                fixtures.insert("front_wash".to_string(), "Generic_Dimmer @ 1:1".to_string());
                fixtures
            }),
            None,
            None,
        );

        let effect_engine = Arc::new(Mutex::new(EffectEngine::new()));
        let timeline = Arc::new(Mutex::new(None));
        let song_time = Arc::new(Mutex::new(Duration::ZERO));

        let result = reload_timeline(
            &[dsl_path],
            &effect_engine,
            &timeline,
            &song_time,
            None,
            Some(&lighting_config),
        );
        assert!(result.is_err());
    }

    #[test]
    fn reload_timeline_with_lighting_system() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
}"#,
        )
        .unwrap();

        let mut ls = LightingSystem::new();
        let _ = ls.load(
            &crate::config::Lighting::new(None, None, None, None),
            tmp_dir.path(),
        );
        let lighting_system = Arc::new(Mutex::new(ls));

        let effect_engine = Arc::new(Mutex::new(EffectEngine::new()));
        let timeline = Arc::new(Mutex::new(None));
        let song_time = Arc::new(Mutex::new(Duration::from_secs(1)));

        let result = reload_timeline(
            &[dsl_path],
            &effect_engine,
            &timeline,
            &song_time,
            Some(&lighting_system),
            None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn reload_timeline_empty_shows_file() {
        // A DSL file that has no show blocks should give "No shows found" error
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("empty.light");
        // Write a valid DSL file but with empty content (no show blocks)
        std::fs::write(&dsl_path, "// just a comment\n").unwrap();

        let effect_engine = Arc::new(Mutex::new(EffectEngine::new()));
        let timeline = Arc::new(Mutex::new(None));
        let song_time = Arc::new(Mutex::new(Duration::ZERO));

        let result = reload_timeline(
            &[dsl_path],
            &effect_engine,
            &timeline,
            &song_time,
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No shows found"));
    }

    #[test]
    fn reload_timeline_at_nonzero_with_layer_commands() {
        // Use a DSL file with a "clear()" command to cover the layer_commands path
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
    @00:01.000
    clear()
    @00:02.000
    front_wash: static color: "red", dimmer: 50%
}"#,
        )
        .unwrap();

        let effect_engine = Arc::new(Mutex::new(EffectEngine::new()));
        let timeline = Arc::new(Mutex::new(None));
        // Set song_time > 0 to exercise the layer_commands/stop_sequences path
        let song_time = Arc::new(Mutex::new(Duration::from_secs(3)));

        let result = reload_timeline(
            &[dsl_path],
            &effect_engine,
            &timeline,
            &song_time,
            None,
            None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn reload_timeline_at_nonzero_with_stop_sequence() {
        // Use a DSL file with a stop sequence command
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            r#"
sequence "flash" {
    @00:00.000
    front_wash: static color: "white", dimmer: 100%
    @00:00.500
    front_wash: static color: "black", dimmer: 0%
}

show "test" {
    @00:00.000
    sequence "flash"
    @00:01.000
    stop sequence "flash"
    @00:02.000
    front_wash: static color: "red", dimmer: 50%
}"#,
        )
        .unwrap();

        let effect_engine = Arc::new(Mutex::new(EffectEngine::new()));
        let timeline = Arc::new(Mutex::new(None));
        // Set song_time past the stop sequence cue
        let song_time = Arc::new(Mutex::new(Duration::from_secs(3)));

        let result = reload_timeline(
            &[dsl_path],
            &effect_engine,
            &timeline,
            &song_time,
            None,
            None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn resolve_effect_groups_without_system() {
        let effect = crate::lighting::EffectInstance::new(
            "test".to_string(),
            crate::lighting::effects::EffectType::Static {
                parameters: std::collections::HashMap::new(),
                duration: None,
            },
            vec!["group1".to_string()],
            None,
            None,
            None,
        );

        let resolved = resolve_effect_groups(None, effect);
        assert_eq!(resolved.target_fixtures, vec!["group1".to_string()]);
    }

    #[test]
    fn resolve_effect_groups_with_system() {
        let ls = LightingSystem::new();
        let lighting_system = Arc::new(Mutex::new(ls));

        let effect = crate::lighting::EffectInstance::new(
            "test".to_string(),
            crate::lighting::effects::EffectType::Static {
                parameters: std::collections::HashMap::new(),
                duration: None,
            },
            vec!["group1".to_string()],
            None,
            None,
            None,
        );

        let _resolved = resolve_effect_groups(Some(&lighting_system), effect);
    }

    #[test]
    fn reload_multiple_files() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path1 = tmp_dir.path().join("show1.light");
        let dsl_path2 = tmp_dir.path().join("show2.light");
        std::fs::write(
            &dsl_path1,
            r#"show "show1" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
}"#,
        )
        .unwrap();
        std::fs::write(
            &dsl_path2,
            r#"show "show2" {
    @00:01.000
    rear_wash: static color: "red", dimmer: 50%
}"#,
        )
        .unwrap();

        let effect_engine = Arc::new(Mutex::new(EffectEngine::new()));
        let timeline = Arc::new(Mutex::new(None));
        let song_time = Arc::new(Mutex::new(Duration::ZERO));

        let result = reload_timeline(
            &[dsl_path1, dsl_path2],
            &effect_engine,
            &timeline,
            &song_time,
            None,
            None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn start_watching_and_trigger_reload() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
}"#,
        )
        .unwrap();

        let effect_engine = Arc::new(Mutex::new(EffectEngine::new()));
        let timeline = Arc::new(Mutex::new(None));
        let song_time = Arc::new(Mutex::new(Duration::ZERO));
        let (tx, mut rx) = broadcast::channel(16);

        let _handle = start_watching(
            vec![dsl_path.clone()],
            effect_engine,
            timeline.clone(),
            song_time,
            None,
            None,
            tx,
        )
        .unwrap();

        // Modify the file to trigger a reload event
        std::thread::sleep(Duration::from_millis(500));
        std::fs::write(
            &dsl_path,
            r#"show "test" {
    @00:00.000
    front_wash: static color: "red", dimmer: 50%
}"#,
        )
        .unwrap();

        // Wait for the debounced event (300ms debounce + processing time)
        // Check for a broadcast message within a reasonable timeout
        let start = std::time::Instant::now();
        let mut received = false;
        while start.elapsed() < Duration::from_secs(5) {
            match rx.try_recv() {
                Ok(msg) => {
                    assert!(
                        msg.contains("reload"),
                        "Expected reload message, got: {}",
                        msg
                    );
                    received = true;
                    break;
                }
                Err(broadcast::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => panic!("Unexpected error: {:?}", e),
            }
        }
        assert!(received, "Should have received a reload broadcast message");

        // Timeline should have been updated
        assert!(
            timeline.lock().is_some(),
            "Timeline should be set after reload"
        );
    }

    #[test]
    fn start_watching_reload_error_path() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
}"#,
        )
        .unwrap();

        let effect_engine = Arc::new(Mutex::new(EffectEngine::new()));
        let timeline = Arc::new(Mutex::new(None));
        let song_time = Arc::new(Mutex::new(Duration::ZERO));
        let (tx, mut rx) = broadcast::channel(16);

        let _handle = start_watching(
            vec![dsl_path.clone()],
            effect_engine,
            timeline,
            song_time,
            None,
            None,
            tx,
        )
        .unwrap();

        // Wait for watcher to be established
        std::thread::sleep(Duration::from_millis(500));

        // Corrupt the file to trigger a reload error
        std::fs::write(&dsl_path, "invalid DSL content {{{").unwrap();

        // Wait for the error broadcast
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            match rx.try_recv() {
                Ok(msg) => {
                    assert!(
                        msg.contains("error"),
                        "Expected error reload message, got: {}",
                        msg
                    );
                    break;
                }
                Err(broadcast::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => panic!("Unexpected error: {:?}", e),
            }
        }
    }

    #[test]
    fn start_watching_returns_handle() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
}"#,
        )
        .unwrap();

        let effect_engine = Arc::new(Mutex::new(EffectEngine::new()));
        let timeline = Arc::new(Mutex::new(None));
        let song_time = Arc::new(Mutex::new(Duration::ZERO));
        let (tx, _rx) = broadcast::channel(16);

        let result = start_watching(
            vec![dsl_path],
            effect_engine,
            timeline,
            song_time,
            None,
            None,
            tx,
        );
        assert!(result.is_ok());

        // WatcherHandle drops here, stopping the watcher
    }
}
