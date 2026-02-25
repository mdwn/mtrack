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
    let paths = file_paths.clone();

    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(Duration::from_millis(300), tx)?;

    // Watch each file path
    for path in &file_paths {
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
                        event.kind == DebouncedEventKind::Any && paths.contains(&event.path)
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
        let mut engine = effect_engine.lock();
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
