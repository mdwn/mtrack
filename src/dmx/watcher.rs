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

use crate::lighting::effects::Color;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use super::engine::Engine;
use crate::lighting::validation::validate_light_shows;
use crate::util::FileStamp;
use crate::webui::wire::messages::{ShowReload, SocketMessage};

/// Handle to a running file watcher, shuts down when dropped.
pub struct WatcherHandle {
    _watcher: notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>,
}

/// Starts watching the given light show files for changes.
///
/// On file change (debounced 300ms):
/// 1. Re-parses the .light file
/// 2. Builds a new LightingTimeline
/// 3. Swaps it into `engine`'s, rebuilt where the lights are
///    ([`Engine::reload_show`])
/// 4. Sends a reload notification to the player's watch hub
///
/// The engine is held weakly: it owns the handle, and the watcher stops
/// with it. It reloads under `generation`, the engine's watcher generation
/// for the song it watches for: once let go, it reloads nothing more.
pub fn start_watching(
    engine: Weak<Engine>,
    generation: u64,
    file_paths: Vec<PathBuf>,
    fallback_tempo_map: Option<crate::tempo::TempoMap>,
    palette: Arc<HashMap<String, Color>>,
    broadcast_tx: broadcast::Sender<SocketMessage>,
) -> Result<WatcherHandle, Box<dyn std::error::Error>> {
    // Canonicalize paths so they match what the OS reports in events.
    let paths: Vec<PathBuf> = file_paths
        .iter()
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
        .collect();

    // The files those include are watched too: an edit to a shared sequence
    // changes the shows that use it. One included only after the song is
    // loaded is watched from its next load.
    // Includes resolve from the path as given, as the song load did, so a
    // symlinked show includes what its link's folder holds.
    let mut watched = paths.clone();
    for path in &file_paths {
        for included in crate::lighting::parser::included_files(path) {
            if !watched.contains(&included) {
                watched.push(included);
            }
        }
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(Duration::from_millis(300), tx)?;

    // Watch each file path
    for path in &watched {
        if let Some(parent) = path.parent() {
            debouncer
                .watcher()
                .watch(parent, notify::RecursiveMode::NonRecursive)?;
        }
    }

    // Spawn a thread to handle file change events
    std::thread::spawn(move || {
        // Track the last-known stamp per watched file so we can distinguish
        // "the file actually changed" from "something fired a spurious event"
        // (read-only access updating atime, the watcher's own read triggering
        // notify in certain backend configurations, etc.). Pre-seed with the
        // current stamps so the first event we process compares against a
        // real baseline rather than treating every watched file as "new".
        let mut last_stamps: std::collections::HashMap<PathBuf, FileStamp> = watched
            .iter()
            .filter_map(|p| FileStamp::of(p).map(|stamp| (p.clone(), stamp)))
            .collect();

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
                        watched.contains(&event_path)
                    });
                    if !relevant {
                        continue;
                    }

                    // Second guard: stamp-based change detection
                    // ([`FileStamp`]: modified time and length). Reads
                    // (which trigger IN_ACCESS / IN_OPEN / IN_CLOSE_NOWRITE
                    // on some inotify backend configurations) do NOT change
                    // the stamp, so a "did the stamp change?" check breaks
                    // the self-sustaining read → event → reload → read loop
                    // without affecting genuine write-driven reloads.
                    //
                    // Use `fold` rather than `any` so EVERY changed file's
                    // stamp entry is refreshed in this pass. With `any`
                    // short-circuiting on the first hit, a multi-file save
                    // would update only the first file's baseline and
                    // trigger a spurious second reload when the next event
                    // arrives for one of the stale entries.
                    let any_changed = watched.iter().fold(false, |acc, p| {
                        let changed = match FileStamp::of(p) {
                            Some(stamp) => {
                                let changed = last_stamps.get(p) != Some(&stamp);
                                if changed {
                                    last_stamps.insert(p.clone(), stamp);
                                }
                                changed
                            }
                            // Metadata failed — be conservative and reload,
                            // so a transient stat error doesn't silently
                            // skip a real change.
                            None => true,
                        };
                        acc || changed
                    });
                    if !any_changed {
                        // Spurious event — nothing on disk actually changed.
                        // Common cause: read-only access events triggering
                        // notify in this backend configuration.
                        continue;
                    }

                    let Some(engine) = engine.upgrade() else {
                        break;
                    };
                    info!("Light show file changed, reloading...");

                    match reload_timeline(
                        &engine,
                        &file_paths,
                        fallback_tempo_map.as_ref(),
                        &palette,
                        generation,
                    ) {
                        Ok(false) => {
                            info!("Light show watcher let go; its song's show not reloaded");
                            break;
                        }
                        Ok(true) => {
                            info!("Light show reloaded successfully");
                            let _ = broadcast_tx.send(SocketMessage::Reload(ShowReload::Ok));
                        }
                        Err(e) => {
                            warn!("Light show reload failed: {}", e);
                            let _ = broadcast_tx.send(SocketMessage::Reload(ShowReload::Error {
                                error: e.to_string(),
                            }));
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

/// Re-parses light show files, builds a new timeline, and swaps it into
/// `engine`'s, rebuilt where the lights are (see [`Engine::reload_show`]),
/// unless the watcher reloading under `generation` has been let go since:
/// then it returns false, the show as it was.
pub(super) fn reload_timeline(
    engine: &Engine,
    file_paths: &[PathBuf],
    fallback_tempo_map: Option<&crate::tempo::TempoMap>,
    palette: &HashMap<String, Color>,
    generation: u64,
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut all_shows = Vec::new();

    for path in file_paths {
        let content = crate::util::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

        // Parse with the same tempo this timeline will run under. Using the
        // tempo-less entry point here meant a show inheriting the song's tempo
        // failed every live edit, and the timeline silently never swapped.
        let shows =
            crate::lighting::parser::parse_light_file(&content, Some(path), fallback_tempo_map)
                .map_err(|e| format!("Parse error in {}: {}", path.display(), e))?;

        // Validate if lighting config is available
        if let Some(lc) = engine.lighting_config.as_ref() {
            validate_light_shows(&shows, Some(lc))
                .map_err(|e| format!("Validation error in {}: {}", path.display(), e))?;
        }

        for (_, show) in shows {
            all_shows.push(show);
        }
    }

    // No end cap of its own: `reload_show` carries the running timeline's
    // over.
    let timeline = super::engine::lighting_timeline(all_shows, Duration::ZERO, palette)
        .ok_or("No shows found after re-parse")?;
    let tempo_map = timeline.tempo_map_or(fallback_tempo_map);
    Ok(engine.reload_show(timeline, tempo_map, generation))
}

#[cfg(test)]
mod test {
    use super::*;
    use std::sync::Arc;

    /// An engine for one universe with `lighting` (from `base`), its song
    /// time at `song_time`.
    fn test_engine(
        lighting: Option<&crate::config::Lighting>,
        base: Option<&std::path::Path>,
        song_time: Duration,
    ) -> Arc<Engine> {
        let engine = Engine::new(
            &crate::config::Dmx::new(
                None,
                None,
                Some(9010),
                vec![crate::config::Universe::new(1, "universe1".to_string())],
                None,
            ),
            lighting,
            base,
            crate::dmx::ola_client::OlaClientFactory::create_mock_client(),
        )
        .unwrap();
        engine.update_song_time(song_time);
        Arc::new(engine)
    }

    #[test]
    fn reload_timeline_with_valid_file_at_zero() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", duration: 5s, dimmer: 100%
}"#,
        )
        .unwrap();

        let engine = test_engine(None, None, Duration::ZERO);

        let result = reload_timeline(&engine, &[dsl_path], None, &HashMap::new(), 0);
        assert!(result.is_ok());
        assert!(engine.current_song_timeline.lock().is_some());
    }

    #[test]
    fn a_reload_keeps_the_colours_the_song_started_with() {
        // A live edit of the show re-reads its file, but the palette the
        // song started with (its override and its entry's) is the one the
        // new timeline settles to.
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            "palette { primary: \"#C01020\" }\nshow \"t\" {\n    @00:00.000\n    front_wash: static color: palette.primary, duration: 5s\n}",
        )
        .unwrap();
        let engine = test_engine(None, None, Duration::ZERO);
        let red = |engine: &Engine| {
            let timeline = engine.current_song_timeline.lock();
            let (_, effects) = timeline.as_ref().unwrap().cue_effects().next().unwrap();
            match &effects[0].effect_type {
                crate::lighting::effects::EffectType::Static { parameters, .. } => {
                    (parameters["red"] * 255.0).round() as u8
                }
                other => panic!("{other:?}"),
            }
        };
        reload_timeline(
            &engine,
            std::slice::from_ref(&dsl_path),
            None,
            &HashMap::new(),
            0,
        )
        .unwrap();
        assert_eq!(red(&engine), 0xC0);
        let palette = HashMap::from([("primary".to_string(), Color::new(0x20, 0x40, 0xFF))]);
        reload_timeline(&engine, &[dsl_path], None, &palette, 0).unwrap();
        assert_eq!(red(&engine), 0x20);
    }

    #[test]
    fn a_reload_leaves_what_midi_dmx_holds() {
        // A live edit's reload cleared the MIDI DMX store along with the
        // show's effects, and nothing put it back mid-song (#583).
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            "show \"test\" {\n    @00:00.000\n    front_wash: static color: \"blue\", duration: 5s\n}",
        )
        .unwrap();
        let store = Arc::new(parking_lot::RwLock::new(
            crate::dmx::midi_dmx_store::MidiDmxStore::new(),
        ));
        {
            let mut store = store.write();
            store.register_slot(1, 1, "par", "dimmer");
            store.register_universe(1);
        }
        store.read().write(1, 1, 200, false);
        let engine = test_engine(None, None, Duration::from_secs(3));
        engine
            .effect_engine
            .lock()
            .set_midi_dmx_store(store.clone());

        reload_timeline(&engine, &[dsl_path], None, &HashMap::new(), 0).unwrap();
        assert!(
            store.read().has_active_slots(),
            "the reload cleared MIDI DMX"
        );
    }

    #[test]
    fn reload_timeline_with_valid_file_at_nonzero() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", duration: 5s, dimmer: 100%
    @00:05.000
    front_wash: static color: "red", duration: 5s, dimmer: 50%
}"#,
        )
        .unwrap();

        let engine = test_engine(None, None, Duration::from_secs(3));

        let result = reload_timeline(&engine, &[dsl_path], None, &HashMap::new(), 0);
        assert!(result.is_ok());
        assert!(engine.current_song_timeline.lock().is_some());
    }

    #[test]
    fn reload_timeline_with_missing_file() {
        let engine = test_engine(None, None, Duration::ZERO);

        let result = reload_timeline(
            &engine,
            &[PathBuf::from("/nonexistent/show.light")],
            None,
            &HashMap::new(),
            0,
        );
        assert!(result.is_err());
    }

    #[test]
    fn reload_timeline_with_invalid_dsl() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("bad.light");
        std::fs::write(&dsl_path, "this is not valid DSL {").unwrap();

        let engine = test_engine(None, None, Duration::ZERO);

        let result = reload_timeline(&engine, &[dsl_path], None, &HashMap::new(), 0);
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
    front_wash: static color: "blue", duration: 5s, dimmer: 100%
}"#,
        )
        .unwrap();

        // Lighting config with "front_wash" defined so validation passes
        let lighting_config = crate::config::Lighting::new(
            None,
            Some(std::collections::HashMap::from([(
                "front_wash".to_string(),
                crate::config::lighting::LogicalGroup::new("front_wash".to_string(), vec![]),
            )])),
            None,
        );

        let engine = test_engine(Some(&lighting_config), None, Duration::ZERO);

        let result = reload_timeline(&engine, &[dsl_path], None, &HashMap::new(), 0);
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
    unknown_fixture: static color: "blue", duration: 5s, dimmer: 100%
}"#,
        )
        .unwrap();

        // Lighting config WITHOUT "unknown_fixture" — validation will fail
        let lighting_config = crate::config::Lighting::new(
            None,
            Some(std::collections::HashMap::from([(
                "front_wash".to_string(),
                crate::config::lighting::LogicalGroup::new("front_wash".to_string(), vec![]),
            )])),
            None,
        );

        let engine = test_engine(Some(&lighting_config), None, Duration::ZERO);

        let result = reload_timeline(&engine, &[dsl_path], None, &HashMap::new(), 0);
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
    front_wash: static color: "blue", duration: 5s, dimmer: 100%
}"#,
        )
        .unwrap();

        // The group named, so the show validates against the configuration
        // the lighting system loads.
        let lighting = crate::config::Lighting::new(
            None,
            Some(std::collections::HashMap::from([(
                "front_wash".to_string(),
                crate::config::lighting::LogicalGroup::new("front_wash".to_string(), vec![]),
            )])),
            None,
        );
        let engine = test_engine(
            Some(&lighting),
            Some(tmp_dir.path()),
            Duration::from_secs(1),
        );
        assert!(engine.lighting_system.is_some());

        let result = reload_timeline(&engine, &[dsl_path], None, &HashMap::new(), 0);
        assert!(result.is_ok());
    }

    #[test]
    fn reload_timeline_empty_shows_file() {
        // A DSL file that has no show blocks should give "No shows found" error
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("empty.light");
        // Write a valid DSL file but with empty content (no show blocks)
        std::fs::write(&dsl_path, "// just a comment\n").unwrap();

        let engine = test_engine(None, None, Duration::ZERO);

        let result = reload_timeline(&engine, &[dsl_path], None, &HashMap::new(), 0);
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
    front_wash: static color: "blue", duration: 5s, dimmer: 100%
    @00:01.000
    clear()
    @00:02.000
    front_wash: static color: "red", duration: 5s, dimmer: 50%
}"#,
        )
        .unwrap();

        // Set song_time > 0 to exercise the layer_commands/stop_sequences path
        let engine = test_engine(None, None, Duration::from_secs(3));

        let result = reload_timeline(&engine, &[dsl_path], None, &HashMap::new(), 0);
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
    front_wash: static color: "white", duration: 5s, dimmer: 100%
    @00:00.500
    front_wash: static color: "black", duration: 5s, dimmer: 0%
}

show "test" {
    @00:00.000
    sequence "flash"
    @00:01.000
    stop sequence "flash"
    @00:02.000
    front_wash: static color: "red", duration: 5s, dimmer: 50%
}"#,
        )
        .unwrap();

        // Set song_time past the stop sequence cue
        let engine = test_engine(None, None, Duration::from_secs(3));

        let result = reload_timeline(&engine, &[dsl_path], None, &HashMap::new(), 0);
        assert!(result.is_ok());
    }

    #[test]
    fn resolve_effect_groups_without_system() {
        let effect = crate::lighting::EffectInstance::new(
            "test".to_string(),
            crate::lighting::effects::EffectType::Static {
                parameters: std::collections::HashMap::new(),
                duration: Duration::ZERO,
            },
            vec!["group1".to_string()],
            None,
            None,
            None,
        );

        let resolved = test_engine(None, None, Duration::ZERO).resolve_effect_groups(effect);
        assert_eq!(resolved.target_fixtures, vec!["group1".to_string()]);
    }

    #[test]
    fn resolve_effect_groups_with_system() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let engine = test_engine(
            Some(&crate::config::Lighting::new(None, None, None)),
            Some(tmp_dir.path()),
            Duration::ZERO,
        );

        let effect = crate::lighting::EffectInstance::new(
            "test".to_string(),
            crate::lighting::effects::EffectType::Static {
                parameters: std::collections::HashMap::new(),
                duration: Duration::ZERO,
            },
            vec!["group1".to_string()],
            None,
            None,
            None,
        );

        let _resolved = engine.resolve_effect_groups(effect);
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
    front_wash: static color: "blue", duration: 5s, dimmer: 100%
}"#,
        )
        .unwrap();
        std::fs::write(
            &dsl_path2,
            r#"show "show2" {
    @00:01.000
    rear_wash: static color: "red", duration: 5s, dimmer: 50%
}"#,
        )
        .unwrap();

        let engine = test_engine(None, None, Duration::ZERO);

        let result = reload_timeline(&engine, &[dsl_path1, dsl_path2], None, &HashMap::new(), 0);
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
    front_wash: static color: "blue", duration: 5s, dimmer: 100%
}"#,
        )
        .unwrap();

        let engine = test_engine(None, None, Duration::ZERO);
        let (tx, mut rx) = broadcast::channel(16);

        let _handle = start_watching(
            Arc::downgrade(&engine),
            0,
            vec![dsl_path.clone()],
            None,
            Default::default(),
            tx,
        )
        .unwrap();

        // Modify the file to trigger a reload event
        std::thread::sleep(Duration::from_millis(500));
        std::fs::write(
            &dsl_path,
            r#"show "test" {
    @00:00.000
    front_wash: static color: "red", duration: 5s, dimmer: 50%
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
                        matches!(msg, SocketMessage::Reload(_)),
                        "Expected reload message, got: {:?}",
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
            engine.current_song_timeline.lock().is_some(),
            "Timeline should be set after reload"
        );
    }

    #[test]
    fn editing_an_included_file_reloads_the_show() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let common_dir = tmp_dir.path().join("common");
        std::fs::create_dir(&common_dir).unwrap();
        let common_path = common_dir.join("looks.light");
        let looks = |colour: &str| {
            format!(
                "sequence \"wash\" {{\n    @00:00.000\n    front_wash: static color: \"{colour}\", duration: 5s\n}}\n"
            )
        };
        std::fs::write(&common_path, looks("blue")).unwrap();
        let song_dir = tmp_dir.path().join("song");
        std::fs::create_dir(&song_dir).unwrap();
        let dsl_path = song_dir.join("show.light");
        std::fs::write(
            &dsl_path,
            "include \"../common/looks.light\"\n\nshow \"test\" {\n    @00:00.000\n    sequence \"wash\"\n}\n",
        )
        .unwrap();

        let engine = test_engine(None, None, Duration::ZERO);
        let (tx, mut rx) = broadcast::channel(16);
        let _handle = start_watching(
            Arc::downgrade(&engine),
            0,
            vec![dsl_path.clone()],
            None,
            Default::default(),
            tx,
        )
        .unwrap();

        // Only the included file changes; the show file is untouched.
        std::thread::sleep(Duration::from_millis(500));
        std::fs::write(&common_path, looks("red")).unwrap();

        let start = std::time::Instant::now();
        let mut status = None;
        while start.elapsed() < Duration::from_secs(5) {
            match rx.try_recv() {
                Ok(msg) => {
                    status = Some(msg);
                    break;
                }
                Err(broadcast::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => panic!("Unexpected error: {:?}", e),
            }
        }
        let status = status.expect("an edit to an included file reloads the show");
        assert_eq!(
            status,
            SocketMessage::Reload(ShowReload::Ok),
            "reload failed"
        );
        assert!(engine.current_song_timeline.lock().is_some());
    }

    #[test]
    fn start_watching_reload_error_path() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", duration: 5s, dimmer: 100%
}"#,
        )
        .unwrap();

        let engine = test_engine(None, None, Duration::ZERO);
        let (tx, mut rx) = broadcast::channel(16);

        let _handle = start_watching(
            Arc::downgrade(&engine),
            0,
            vec![dsl_path.clone()],
            None,
            Default::default(),
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
                        matches!(msg, SocketMessage::Reload(ShowReload::Error { .. })),
                        "Expected error reload message, got: {:?}",
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
    fn a_watcher_let_go_reloads_nothing() {
        // Dropping a watcher does not drain what it has taken in: an edit
        // still in its debounce as the song changed reloaded the last
        // song's show into the next one.
        let tmp_dir = tempfile::tempdir().unwrap();
        let last = tmp_dir.path().join("last.light");
        std::fs::write(
            &last,
            "show \"last\" {\n    @00:00.000\n    front_wash: static color: \"red\", duration: 5s\n    @00:02.000\n    front_wash: static color: \"green\", duration: 5s\n}",
        )
        .unwrap();
        let next = tmp_dir.path().join("next.light");
        std::fs::write(
            &next,
            "show \"next\" {\n    @00:00.000\n    front_wash: static color: \"blue\", duration: 5s\n}",
        )
        .unwrap();
        let engine = test_engine(None, None, Duration::from_secs(1));
        let old = engine.retire_watcher();
        // The next song starts, with a watcher of its own.
        let new = engine.retire_watcher();
        assert!(reload_timeline(
            &engine,
            std::slice::from_ref(&next),
            None,
            &HashMap::new(),
            new
        )
        .unwrap());
        let cues = engine.get_timeline_cues();
        // The last song's watcher's edit lands after.
        assert!(!reload_timeline(&engine, &[last], None, &HashMap::new(), old).unwrap());
        assert_eq!(engine.get_timeline_cues(), cues);
        // The next song's own still reloads.
        assert!(reload_timeline(&engine, &[next], None, &HashMap::new(), new).unwrap());
    }

    #[test]
    fn start_watching_returns_handle() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dsl_path = tmp_dir.path().join("show.light");
        std::fs::write(
            &dsl_path,
            r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", duration: 5s, dimmer: 100%
}"#,
        )
        .unwrap();

        let engine = test_engine(None, None, Duration::ZERO);
        let (tx, _rx) = broadcast::channel(16);

        let result = start_watching(
            Arc::downgrade(&engine),
            0,
            vec![dsl_path],
            None,
            Default::default(),
            tx,
        );
        assert!(result.is_ok());

        // WatcherHandle drops here, stopping the watcher
    }
}
