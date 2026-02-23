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

    // Set tempo map on effect engine
    {
        let mut engine = effect_engine.lock();
        engine.set_tempo_map(new_timeline.tempo_map().cloned());
        engine.stop_all_effects();
    }

    // Start the new timeline at the current position
    let timeline_update = if song_time > Duration::ZERO {
        new_timeline.start_at(song_time)
    } else {
        new_timeline.start();
        crate::lighting::timeline::TimelineUpdate::default()
    };

    // Swap in the new timeline
    {
        let mut current = current_song_timeline.lock();
        *current = Some(new_timeline);
    }

    // Apply historical state from the timeline update
    if song_time > Duration::ZERO {
        apply_timeline_update_to_engine(effect_engine, lighting_system, timeline_update)?;
    }

    Ok(())
}

/// Applies a timeline update to the effect engine (mirrors DmxEngine::apply_timeline_update).
fn apply_timeline_update_to_engine(
    effect_engine: &Arc<Mutex<EffectEngine>>,
    lighting_system: Option<&Arc<Mutex<LightingSystem>>>,
    timeline_update: crate::lighting::timeline::TimelineUpdate,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::lighting::parser::LayerCommandType;

    // Process layer commands
    if !timeline_update.layer_commands.is_empty() {
        let mut engine = effect_engine.lock();
        for cmd in &timeline_update.layer_commands {
            match cmd.command_type {
                LayerCommandType::Clear => {
                    if let Some(layer) = cmd.layer {
                        engine.clear_layer(layer);
                    } else {
                        engine.clear_all_layers();
                    }
                }
                LayerCommandType::Release => {
                    if let Some(layer) = cmd.layer {
                        if let Some(fade_time) = cmd.fade_time {
                            engine.release_layer_with_time(layer, Some(fade_time));
                        } else {
                            engine.release_layer(layer);
                        }
                    }
                }
                LayerCommandType::Freeze => {
                    if let Some(layer) = cmd.layer {
                        engine.freeze_layer(layer);
                    }
                }
                LayerCommandType::Unfreeze => {
                    if let Some(layer) = cmd.layer {
                        engine.unfreeze_layer(layer);
                    }
                }
                LayerCommandType::Master => {
                    if let Some(layer) = cmd.layer {
                        if let Some(intensity) = cmd.intensity {
                            engine.set_layer_intensity_master(layer, intensity);
                        }
                        if let Some(speed) = cmd.speed {
                            engine.set_layer_speed_master(layer, speed);
                        }
                    }
                }
            }
        }
    }

    // Process stop sequences
    if !timeline_update.stop_sequences.is_empty() {
        let mut engine = effect_engine.lock();
        for sequence_name in &timeline_update.stop_sequences {
            engine.stop_sequence(sequence_name);
        }
    }

    // Start effects with elapsed time
    let mut effects_sorted: Vec<_> = timeline_update.effects_with_elapsed.values().collect();
    effects_sorted.sort_by_key(|(effect, _)| effect.cue_time.unwrap_or(Duration::ZERO));

    for (effect, elapsed_time) in effects_sorted {
        let resolved = resolve_effect_groups(lighting_system, effect.clone());
        let mut engine = effect_engine.lock();
        if let Err(e) = engine.start_effect_with_elapsed(resolved, *elapsed_time) {
            error!("Failed to start lighting effect with elapsed time: {}", e);
        }
    }

    // Start regular effects
    let mut effects = timeline_update.effects;
    effects.sort_by_key(|e| if e.id.starts_with("seq_") { 0 } else { 1 });
    for effect in effects {
        let resolved = resolve_effect_groups(lighting_system, effect);
        let mut engine = effect_engine.lock();
        if let Err(e) = engine.start_effect(resolved) {
            error!("Failed to start lighting effect: {}", e);
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
