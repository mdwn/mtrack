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

//! The `json!` each payload was built with before it had a type, kept as
//! the oracle that the type serialises to the same JSON, over
//! representative values. A second copy of every payload's shape: once
//! the change is verified this file and its `mod` line go, and the short
//! tests in `tests.rs` stay.

mod messages {
    use std::collections::HashMap;

    use serde_json::json;

    use crate::config::{FadeCurve, FadeStatus, Section};
    use crate::dmx::engine::AmbientMode;
    use crate::playlist::AmbientSource;
    use crate::webui::state::{build_metadata_json, build_state_json, to_json};
    use crate::webui::wire::messages as wire;

    fn value(msg: &wire::SocketMessage) -> serde_json::Value {
        serde_json::from_str(&to_json(msg)).unwrap()
    }

    fn fade() -> FadeStatus {
        FadeStatus {
            length: "1measure".to_string(),
            curve: FadeCurve::EqualPower,
            exclude: Some(vec!["click".to_string()]),
            lights: None,
        }
    }

    fn old_fade(f: &FadeStatus) -> serde_json::Value {
        json!({
            "length": f.length,
            "curve": match f.curve {
                FadeCurve::EqualPower => "equal_power",
                FadeCurve::Linear => "linear",
            },
            "exclude": f.exclude,
            "lights": f.lights,
        })
    }

    #[test]
    fn playback() {
        let grid = crate::audio::beat_grid::BeatGrid {
            beats: vec![0.0, 0.5, 1.0, 1.5, 2.0],
            measure_starts: vec![0, 4],
        };
        let section = Section {
            name: "verse".to_string(),
            start_measure: 1,
            end_measure: 2,
            start_beat: Some(2.5),
            end_beat: None,
            color: None,
        };
        for (with_options, loop_state, old_loop_state) in [
            (true, wire::LoopState::Idle, json!({"state": "idle"})),
            (
                false,
                wire::LoopState::LoopArmed {
                    section_name: "verse".to_string(),
                },
                json!({"state": "loop_armed", "section_name": "verse"}),
            ),
            (
                true,
                wire::LoopState::BreakRequested {
                    section_name: "verse".to_string(),
                },
                json!({"state": "break_requested", "section_name": "verse"}),
            ),
        ] {
            let accent_levels = with_options.then(|| vec![3, 1, 1, 1, 3]);
            let msg = wire::SocketMessage::Playback(Box::new(wire::Playback {
                is_playing: with_options,
                elapsed_ms: 1234,
                song_name: "Song".to_string(),
                song_duration_ms: 60_000,
                playlist_name: "show".to_string(),
                playlist_position: 2,
                playlist_off_list: !with_options,
                playlist_songs: vec!["A".to_string(), "Song".to_string()],
                tracks: vec![wire::TrackInfo {
                    name: "kick".to_string(),
                    output_channels: vec![1, 2],
                    gain_db: -3.0,
                    muted: true,
                }],
                available_playlists: vec!["show".to_string()],
                persisted_playlist_name: "show".to_string(),
                locked: false,
                hardware_reload_started: 7,
                hardware_reload_refused: vec![wire::RefusedReload {
                    generation: 6,
                    error: "busy".to_string(),
                }],
                process_id: "18446744073709551615".to_string(),
                beat_grid: with_options.then(|| wire::PlaybackGrid {
                    grid: grid.clone(),
                    accent_levels: accent_levels.clone(),
                }),
                available_sections: vec![section.clone()],
                active_section: with_options.then(|| wire::ActiveSection {
                    name: "verse".to_string(),
                    start_ms: 2000,
                    end_ms: 4000,
                }),
                looping: with_options,
                reactive_loop_state: loop_state,
                pending_start_ms: with_options.then_some(500),
                pilot_hints: vec![wire::ResolvedPilotHint {
                    label: "go".to_string(),
                    at_ms: 1,
                    start_ms: 0,
                    end_ms: 1,
                    has_audio: false,
                }],
                tempo: with_options.then_some(wire::PlaybackTempo {
                    bpm: 120.0,
                    time_signature: (7, 8),
                }),
                auto_play: with_options.then(|| wire::AutoPlay {
                    song: "Next".to_string(),
                    starts_in_ms: 900,
                }),
                ending: with_options.then(|| wire::Ending {
                    song: "Song".to_string(),
                    remaining_ms: 300,
                }),
                ambient: with_options.then(|| wire::AmbientState {
                    mode: AmbientMode::On,
                    source: Some(AmbientSource::Entry),
                    show: None,
                    state: "unavailable".to_string(),
                }),
                next: with_options.then(wire::NextUp::default),
            }));
            let old_grid = with_options.then(|| {
                let mut g = json!({
                    "beats": grid.beats,
                    "measure_starts": grid.measure_starts,
                });
                g["accent_levels"] = json!(accent_levels);
                g
            });
            // The section as `SongSummary` carried it: its unset beats
            // and colour left out, as the song's YAML leaves them.
            let old = json!({
                "type": "playback",
                "is_playing": with_options,
                "elapsed_ms": 1234,
                "song_name": "Song",
                "song_duration_ms": 60_000,
                "playlist_name": "show",
                "playlist_position": 2,
                "playlist_off_list": !with_options,
                "playlist_songs": ["A", "Song"],
                "tracks": [{"name": "kick", "output_channels": [1, 2], "gain_db": -3.0, "muted": true}],
                "available_playlists": ["show"],
                "persisted_playlist_name": "show",
                "locked": false,
                "hardware_reload_started": 7,
                "hardware_reload_refused": [json!({"generation": 6, "error": "busy"})],
                "process_id": u64::MAX.to_string(),
                "beat_grid": old_grid,
                "available_sections": [json!(section)],
                "active_section": with_options.then(|| json!({
                    "name": "verse",
                    "start_ms": 2000,
                    "end_ms": 4000,
                })),
                "looping": with_options,
                "reactive_loop_state": old_loop_state,
                "pending_start_ms": with_options.then_some(500),
                "pilot_hints": [json!({
                    "label": "go",
                    "at_ms": 1,
                    "start_ms": 0,
                    "end_ms": 1,
                    "has_audio": false,
                })],
                "tempo": with_options.then(|| json!({
                    "bpm": 120.0,
                    "time_signature": [7, 8],
                })),
                "auto_play": with_options.then(|| json!({
                    "song": "Next",
                    "starts_in_ms": 900,
                })),
                "ending": with_options.then(|| json!({
                    "song": "Song",
                    "remaining_ms": 300,
                })),
                "ambient": if with_options {
                    json!({
                        "mode": AmbientMode::On.as_str(),
                        "source": Some(AmbientSource::Entry).map(|s| s.as_str()),
                        "show": None::<String>,
                        "state": "unavailable",
                    })
                } else {
                    serde_json::Value::Null
                },
                "next": if with_options { json!({"song": null}) } else { serde_json::Value::Null },
            });
            assert_eq!(value(&msg), old);
        }
    }

    #[test]
    fn grid_without_a_metronome_has_no_accent_levels() {
        let grid = wire::PlaybackGrid {
            grid: crate::audio::beat_grid::BeatGrid {
                beats: vec![0.0, 1.0],
                measure_starts: vec![0],
            },
            accent_levels: None,
        };
        assert_eq!(
            json!(grid),
            json!({"beats": [0.0, 1.0], "measure_starts": [0]})
        );
    }

    #[test]
    fn next_up() {
        let f = fade();
        let cases = [
            (wire::NextUp::default(), json!({"song": null})),
            (
                wire::NextUp {
                    ends_loop: Some(true),
                    fade_out: Some(Some(f.clone())),
                    ..Default::default()
                },
                json!({"song": null, "ends_loop": true, "fade_out": old_fade(&f)}),
            ),
            (
                wire::NextUp {
                    ends_loop: Some(true),
                    fade_out: Some(None),
                    ..Default::default()
                },
                json!({"song": null, "ends_loop": true, "fade_out": null}),
            ),
            (
                wire::NextUp {
                    song: Some("B".to_string()),
                    breaks_loop: Some(true),
                    fade_out: Some(Some(f.clone())),
                    fade_in: Some(None),
                    auto_play_ms: Some(None),
                    ..Default::default()
                },
                json!({
                    "song": "B",
                    "breaks_loop": true,
                    "fade_out": old_fade(&f),
                    "fade_in": null,
                    "auto_play_ms": null,
                }),
            ),
            (
                wire::NextUp {
                    song: Some("B".to_string()),
                    breaks_loop: Some(false),
                    fade_out: Some(None),
                    fade_in: Some(None),
                    auto_play_ms: Some(Some(2000)),
                    ..Default::default()
                },
                json!({
                    "song": "B",
                    "breaks_loop": false,
                    "fade_out": null,
                    "fade_in": null,
                    "auto_play_ms": 2000,
                }),
            ),
        ];
        for (next, old) in cases {
            assert_eq!(json!(next), old);
        }
        let linear = FadeStatus {
            curve: FadeCurve::Linear,
            exclude: None,
            lights: Some("0s".to_string()),
            ..fade()
        };
        assert_eq!(json!(linear), old_fade(&linear));
    }

    #[test]
    fn state() {
        let fixtures = vec![crate::state::FixtureSnapshot {
            name: "bar".to_string(),
            channels: HashMap::from([("red".to_string(), 255), ("dimmer".to_string(), 9)]),
            cells: std::collections::BTreeMap::from([(
                "c1".to_string(),
                HashMap::from([("red".to_string(), 1)]),
            )]),
            strobe_hz: Some(4.0),
        }];
        let poses = vec![crate::state::PoseSnapshot {
            name: "mover".to_string(),
            pan: 10.0,
            tilt: -5.0,
            aim: [0.0, 0.0, -1.0],
            floor: Some([1.0, 2.0]),
        }];
        let snapshot = crate::state::StateSnapshot {
            fixtures: fixtures.clone(),
            active_effects: vec!["fx".to_string()],
            poses: poses.clone(),
            under_test: vec!["bar".to_string()],
        };
        let old = json!({
            "type": "state",
            "fixtures": {"bar": {"red": 255, "dimmer": 9}},
            "active_effects": ["fx"],
            "poses": {"mover": {
                "pan": 10.0,
                "tilt": -5.0,
                "aim": [0.0, 0.0, -1.0],
                "floor": [1.0, 2.0],
            }},
            "cells": {"bar": {"c1": {"red": 1}}},
            "strobe_rates": {"bar": 4.0},
            "under_test": ["bar"],
        });
        let got: serde_json::Value = serde_json::from_str(&build_state_json(&snapshot)).unwrap();
        assert_eq!(got, old);
        let mut pose = poses[0].clone();
        pose.floor = None;
        assert_eq!(
            json!(pose),
            json!({"pan": 10.0, "tilt": -5.0, "aim": [0.0, 0.0, -1.0], "floor": null})
        );
    }

    #[test]
    fn metadata() {
        let fixture = wire::FixtureMetadata {
            tags: vec!["front".to_string()],
            fixture_type: "par".to_string(),
            position: Some([1.0, 2.0, 3.0]),
            rotation: None,
            beam_angle: Some(30.0),
            rig: None,
            capabilities: vec!["color".to_string()],
            cells: vec![wire::CellPlacement {
                name: "c1".to_string(),
                offset: [0.1, 0.0, 0.0],
            }],
        };
        let venue = wire::VenueMetadata {
            name: "club".to_string(),
            dir: None,
            focus_points: std::collections::BTreeMap::from([("dsc".to_string(), [0.0, 1.0, 0.0])]),
            scenery: Some("club/scene.glb".to_string()),
            scenery_error: None,
        };
        let msg = wire::SocketMessage::Metadata(wire::StageMetadata {
            fixtures: std::collections::BTreeMap::from([("p1".to_string(), fixture)]),
            venue: Some(venue),
        });
        let old = json!({
            "type": "metadata",
            "fixtures": {"p1": {
                "tags": ["front"],
                "type": "par",
                "position": [1.0, 2.0, 3.0],
                "rotation": null,
                "beam_angle": 30.0,
                "rig": null,
                "capabilities": ["color"],
                "cells": [json!({"name": "c1", "offset": [0.1, 0.0, 0.0]})],
            }},
            "venue": {
                "name": "club",
                "dir": null,
                "focus_points": {"dsc": [0.0, 1.0, 0.0]},
                "scenery": "club/scene.glb",
                "scenery_error": null,
            },
        });
        assert_eq!(value(&msg), old);
        let none: serde_json::Value = serde_json::from_str(&build_metadata_json(None)).unwrap();
        assert_eq!(
            none,
            json!({"type": "metadata", "fixtures": {}, "venue": serde_json::Value::Null})
        );
    }

    #[test]
    fn logs_waveform_config_and_reload() {
        let cases = [
            (
                wire::SocketMessage::Logs {
                    lines: vec![wire::LogLine {
                        level: "INFO".to_string(),
                        target: "mtrack".to_string(),
                        message: "hi".to_string(),
                    }],
                },
                json!({"type": "logs", "lines": [json!({
                    "level": "INFO",
                    "target": "mtrack",
                    "message": "hi",
                })]}),
            ),
            (
                wire::SocketMessage::Waveform(wire::Waveform {
                    song_name: "Song".to_string(),
                    tracks: vec![wire::TrackPeaks {
                        name: "kick".to_string(),
                        peaks: vec![0.5, 1.0],
                    }],
                }),
                json!({"type": "waveform", "song_name": "Song", "tracks": [json!({
                    "name": "kick",
                    "peaks": [0.5f32, 1.0f32],
                })]}),
            ),
            (
                wire::SocketMessage::ConfigChanged,
                json!({ "type": "config_changed" }),
            ),
            (
                wire::SocketMessage::Reload(wire::ShowReload::Ok),
                json!({"type": "reload", "status": "ok"}),
            ),
            (
                wire::SocketMessage::Reload(wire::ShowReload::Error {
                    error: "bad".to_string(),
                }),
                json!({"type": "reload", "status": "error", "error": "bad"}),
            ),
        ];
        for (msg, old) in cases {
            assert_eq!(value(&msg), old);
        }
    }
}

mod songs {
    #[test]
    fn wire_payloads_match_the_old_json() {
        use crate::audio::guessed_tempo::{GuessedTempo, GuessedTempoChange};
        use crate::webui::wire::songs::*;
        use serde_json::json;

        let grid = crate::audio::beat_grid::BeatGrid {
            beats: vec![0.0, 0.5],
            measure_starts: vec![0],
        };
        for (grid, crossfade) in [(Some(grid), Some(250)), (None, None)] {
            let info = SongInfo {
                name: "Song".to_string(),
                duration_ms: 1000,
                duration_display: "0:01".to_string(),
                num_channels: 2,
                sample_format: "S24LE".to_string(),
                track_count: 1,
                tracks: vec!["kick".to_string()],
                has_midi: false,
                has_lighting: true,
                base_dir: "Song".to_string(),
                lighting_files: vec!["Song/show.light".to_string()],
                midi_dmx_files: vec![],
                beat_grid: grid.clone(),
                has_tempo_map: true,
                loop_playback: crossfade.is_some(),
                loop_crossfade_ms: crossfade,
                loop_crossfade_cut: vec!["click".to_string()],
                sections: vec![],
            };
            let failure = SongFailure {
                name: "Bad".to_string(),
                error: "no tracks".to_string(),
                base_dir: "Bad".to_string(),
                failed: true,
            };
            let old_info = json!({
                "name": "Song",
                "duration_ms": 1000,
                "duration_display": "0:01",
                "num_channels": 2,
                "sample_format": "S24LE",
                "track_count": 1,
                "tracks": ["kick"],
                "has_midi": false,
                "has_lighting": true,
                "base_dir": "Song",
                "lighting_files": ["Song/show.light"],
                "midi_dmx_files": [],
                "beat_grid": grid.map(|g| json!({
                    "beats": g.beats,
                    "measure_starts": g.measure_starts,
                })),
                "has_tempo_map": true,
                "loop_playback": crossfade.is_some(),
                "loop_crossfade_ms": crossfade,
                "loop_crossfade_cut": ["click"],
                "sections": [],
            });
            let old_failure = json!({
                "name": "Bad",
                "error": "no tracks",
                "base_dir": "Bad",
                "failed": true,
            });
            assert_eq!(
                json!(SongList {
                    songs: vec![info],
                    failures: vec![failure],
                }),
                json!({"songs": [old_info], "failures": [old_failure]})
            );
        }

        let tempo = GuessedTempo {
            start_seconds: 0.5,
            bpm: 120.0,
            time_signature: [4, 4],
            changes: vec![GuessedTempoChange {
                measure: 9,
                beat: 1,
                bpm: 90.0,
                time_signature: [3, 4],
                transition_beats: None,
            }],
            alignment_rms_ms: Some(3.0),
        };
        for (source, name) in [
            (TempoGuessSource::Midi, "midi"),
            (TempoGuessSource::BeatGrid, "beat_grid"),
        ] {
            assert_eq!(
                json!(TempoGuess {
                    source,
                    tempo: tempo.clone(),
                }),
                json!({"source": name, "tempo": tempo})
            );
        }

        let peaks = vec![("kick".to_string(), vec![0.5f32])];
        let tracks: Vec<serde_json::Value> = peaks
            .iter()
            .map(|(track_name, p)| json!({ "name": track_name, "peaks": p }))
            .collect();
        let waveform = crate::webui::wire::messages::Waveform {
            song_name: "Song".to_string(),
            tracks: peaks
                .into_iter()
                .map(|(name, peaks)| crate::webui::wire::messages::TrackPeaks { name, peaks })
                .collect(),
        };
        assert_eq!(
            json!(waveform),
            json!({ "song_name": "Song", "tracks": tracks })
        );
    }
}

mod status {
    #[test]
    fn status_matches_the_old_json() {
        use crate::player::{
            AudioOutputHealth, ControllerStatus, HardwareStatusSnapshot, LightingVenueStatus,
            SubsystemStatus, TestOutputStatus,
        };
        use crate::webui::wire::status::*;
        use serde_json::json;

        let subsystem = |status: &str, error: Option<&str>| SubsystemStatus {
            status: status.to_string(),
            name: Some("dev".to_string()),
            error: error.map(String::from),
        };
        let hardware = HardwareStatusSnapshot {
            init_done: true,
            hostname: Some("pi".to_string()),
            profile: None,
            audio: subsystem("connected", None),
            audio_output: Some(AudioOutputHealth {
                status: crate::audio::health::OutputStatus::NeverStarted,
                callback_alive: false,
                writing_signal: false,
                since_last_callback_ms: None,
                since_last_signal_ms: Some(5),
                callbacks: 0,
                recoveries: 1,
                underruns: 2,
                last_error: None,
            }),
            midi: subsystem("failed", Some("gone")),
            dmx: subsystem("not_connected", None),
            trigger: subsystem("initializing", None),
            lighting_venue: LightingVenueStatus {
                name: None,
                status: "none".to_string(),
                error: None,
            },
            test_output: Some(TestOutputStatus {
                universe: 1,
                address: 10,
                footprint: 4,
                fixture_type: "par".to_string(),
                mode: None,
                expires_in_secs: 2.5,
            }),
        };
        let controllers = vec![ControllerStatus {
            kind: "osc".to_string(),
            status: "running".to_string(),
            detail: Some("port 9000".to_string()),
            error: None,
        }];
        let status = Status {
            build: BuildInfo {
                version: "1".to_string(),
                git_hash: "abc".to_string(),
                build_time: "now".to_string(),
                ui_build: None,
            },
            hardware: hardware.clone(),
            controllers: controllers.clone(),
            locked: true,
        };
        let old_subsystem = |status: &str, error: Option<&str>| {
            let mut s = json!({"status": status, "name": "dev"});
            if let Some(error) = error {
                s["error"] = json!(error);
            }
            s
        };
        let old_hardware = json!({
            "init_done": true,
            "hostname": "pi",
            "profile": null,
            "audio": old_subsystem("connected", None),
            "audio_output": {
                "status": "never_started",
                "callback_alive": false,
                "writing_signal": false,
                "since_last_callback_ms": null,
                "since_last_signal_ms": 5,
                "callbacks": 0,
                "recoveries": 1,
                "underruns": 2,
                "last_error": null,
            },
            "midi": old_subsystem("failed", Some("gone")),
            "dmx": old_subsystem("not_connected", None),
            "trigger": old_subsystem("initializing", None),
            "lighting_venue": {"name": null, "status": "none", "error": null},
            "test_output": {
                "universe": 1,
                "address": 10,
                "footprint": 4,
                "fixture_type": "par",
                "mode": null,
                "expires_in_secs": 2.5,
            },
        });
        let old_controllers = json!([{
            "kind": "osc",
            "status": "running",
            "detail": "port 9000",
            "error": null,
        }]);
        assert_eq!(
            json!(status),
            json!({
                "build": {
                    "version": "1",
                    "git_hash": "abc",
                    "build_time": "now",
                    "ui_build": null,
                },
                "hardware": old_hardware,
                "controllers": old_controllers,
                "locked": true,
            })
        );
        assert_eq!(
            json!(ControllersRestarted {
                status: "restarted".to_string(),
                controllers,
            }),
            json!({"status": "restarted", "controllers": old_controllers})
        );
    }
}

mod lighting {
    use serde_json::json;

    /// Readiness and fit are what the `json!` they replaced built: that
    /// `json!`, kept here as the oracle.
    #[test]
    fn readiness_and_fit_match_the_old_json() {
        use crate::lighting::findings::{LibraryFinding, VenueError, Want, Warning};
        use crate::lighting::fit::*;
        use crate::webui::wire::readiness::*;

        let warning = Warning {
            kind: "patch-overlap",
            message: "a overlaps b".to_string(),
        };
        let w = |w: &Warning| json!({"kind": w.kind, "message": w.message});
        for olad in [
            None,
            Some(OladPatch {
                reachable: true,
                unpatched: vec![2],
            }),
        ] {
            let readiness = Readiness {
                dmx: true,
                venue: Some(ReadinessVenue {
                    name: "club".to_string(),
                    fixtures: 2,
                    placed: 1,
                    focus_points: vec!["dsc".to_string()],
                }),
                venue_error: Some(VenueError {
                    venue: "club".to_string(),
                    fixture: "p1".to_string(),
                    reason: "no type".to_string(),
                }),
                patch_warnings: vec![warning.clone()],
                fixture_types: ReadinessTypes {
                    in_use: vec!["par".to_string()],
                    unresolved: vec![UnresolvedFixture {
                        fixture: "p1".to_string(),
                        fixture_type: "spot".to_string(),
                        reason: "missing".to_string(),
                    }],
                    library: vec![LibraryFinding {
                        kind: "gdtf-unreadable",
                        file: "x.gdtf".to_string(),
                        message: "bad".to_string(),
                    }],
                },
                groups: vec![ReadinessGroup {
                    name: "front".to_string(),
                    fixtures: 2,
                    songs: vec!["Song".to_string()],
                }],
                shows: vec![
                    ReadinessShow {
                        song: "A".to_string(),
                        files: vec!["a.light".to_string()],
                        error: None,
                        warnings: vec![warning.clone()],
                    },
                    ReadinessShow {
                        song: "B".to_string(),
                        files: vec![],
                        error: Some("parse".to_string()),
                        warnings: vec![],
                    },
                ],
                output: ReadinessOutput {
                    universes: vec![1, 2],
                    unconfigured: vec![2],
                    olad: olad.clone(),
                },
            };
            let old_olad = match &olad {
                Some(r) => json!({"reachable": r.reachable, "unpatched": r.unpatched}),
                None => serde_json::Value::Null,
            };
            assert_eq!(
                json!(readiness),
                json!({
                    "dmx": true,
                    "venue": {"name": "club", "fixtures": 2, "placed": 1, "focus_points": ["dsc"]},
                    "venue_error": {"venue": "club", "fixture": "p1", "reason": "no type"},
                    "patch_warnings": [w(&warning)],
                    "fixture_types": {
                        "in_use": ["par"],
                        "unresolved": [json!({"fixture": "p1", "type": "spot", "reason": "missing"})],
                        "library": [json!({"kind": "gdtf-unreadable", "file": "x.gdtf", "message": "bad"})],
                    },
                    "groups": [json!({"name": "front", "fixtures": 2, "songs": ["Song"]})],
                    "shows": [
                        json!({"song": "A", "files": ["a.light"], "warnings": [w(&warning)]}),
                        json!({"song": "B", "files": [], "error": "parse", "warnings": []}),
                    ],
                    "output": {"universes": [1, 2], "unconfigured": [2], "olad": old_olad},
                })
            );
        }

        let cluster = Cluster {
            fixtures: vec!["p1".to_string()],
            fixture_type: "par".to_string(),
            place: Some("upstage truss".to_string()),
            height: Some(Height::Truss),
            depth: Some(Depth::Upstage),
        };
        let fit = Fit {
            suggestion: Some(Suggestion {
                fixtures: vec!["p1".to_string()],
                tags: vec!["front".to_string()],
                reason: Reason {
                    count: 1,
                    fixture_type: "par".to_string(),
                    place: None,
                    height: None,
                    depth: None,
                    can: vec![Want::Color],
                },
            }),
            others: vec![cluster],
            unmet: vec![Want::Move],
            unmet_together: false,
        };
        let needs = Needs {
            all_of: vec!["front".to_string()],
            any_of: vec![],
            prefer: vec!["wash".to_string()],
        };
        let report = FitReport {
            venue: Some(FitVenue {
                name: "club".to_string(),
                fixtures: vec![FitFixtureView {
                    name: "p1".to_string(),
                    fixture_type: "par".to_string(),
                    tags: vec![],
                    position: Some([0.0, 1.0, 2.0]),
                    capabilities: vec!["color".to_string()],
                }],
                focus_points: vec![],
                error: Some(FitVenueError {
                    fixture: None,
                    reason: "bad".to_string(),
                }),
            }),
            groups: vec![FitGroup {
                name: "front".to_string(),
                defined: true,
                needs: needs.clone(),
                fixtures: vec![],
                songs: vec!["Song".to_string()],
                wants: vec![Want::Color, Want::Strobe],
                fit: fit.clone(),
            }],
            focus_points_wanted: vec![FocusWanted {
                name: "dsc".to_string(),
                songs: vec!["Song".to_string()],
            }],
            output: FitOutput {
                unconfigured: vec![],
                unpatched: vec![3],
                reachable: None,
                ola_http_port: Some(9090),
            },
        };
        assert_eq!(
            json!(report),
            json!({
                "venue": {
                    "name": "club",
                    "fixtures": [json!({
                        "name": "p1",
                        "type": "par",
                        "tags": [],
                        "position": [0.0, 1.0, 2.0],
                        "capabilities": ["color"],
                    })],
                    "focus_points": [],
                    "error": {"fixture": null, "reason": "bad"},
                },
                "groups": [json!({
                    "name": "front",
                    "defined": true,
                    "needs": needs,
                    "fixtures": [],
                    "songs": ["Song"],
                    "wants": [Want::Color.name(), Want::Strobe.name()],
                    "suggestion": fit.suggestion,
                    "others": fit.others,
                    "unmet": fit.unmet,
                    "unmet_together": fit.unmet_together,
                })],
                "focus_points_wanted": [json!({"name": "dsc", "songs": ["Song"]})],
                "output": {
                    "unconfigured": [],
                    "unpatched": [3],
                    "reachable": null,
                    "ola_http_port": 9090,
                },
            })
        );
    }

    /// The preview is what the `json!` it replaced built: that `json!`,
    /// kept here as the oracle.
    #[test]
    fn preview_matches_the_old_json() {
        use crate::webui::wire::messages::StageValues;
        use crate::webui::wire::preview::*;

        let frame = StageValues {
            fixtures: std::collections::BTreeMap::from([(
                "p".to_string(),
                std::collections::HashMap::from([("red".to_string(), 9u8)]),
            )]),
            strobe_rates: std::collections::BTreeMap::from([("p".to_string(), 4.0)]),
            ..Default::default()
        };
        for groups in [None, Some(vec!["front".to_string()])] {
            let preview = Preview {
                song: "Song".to_string(),
                evaluations: vec![PreviewEvaluation {
                    time: 1.5,
                    frame: frame.clone(),
                    active_effects: vec![PreviewEffect {
                        id: "e1".to_string(),
                        groups: groups.clone(),
                        kind: "Static".to_string(),
                        layer: "foreground".to_string(),
                        elapsed: 0.5,
                        duration: 2.0,
                        fixtures: vec!["p".to_string()],
                    }],
                }],
                untouched: vec!["q".to_string()],
                venue_error: None,
            };
            assert_eq!(
                json!(preview),
                json!({
                    "song": "Song",
                    "evaluations": [json!({
                        "time": 1.5,
                        "fixtures": frame.fixtures,
                        "poses": frame.poses,
                        "cells": frame.cells,
                        "strobe_rates": frame.strobe_rates,
                        "active_effects": [json!({
                            "id": "e1",
                            "groups": groups,
                            "kind": "Static",
                            "layer": "foreground",
                            "elapsed": 0.5,
                            "duration": 2.0,
                            "fixtures": ["p"],
                        })],
                    })],
                    "untouched": ["q"],
                    "venue_error": null,
                })
            );
        }
    }

    #[test]
    fn venue_patch_and_save_match_the_old_json() {
        use crate::lighting::findings::VenueError;
        use crate::lighting::patch::{PatchOverlap, PatchOverrun};
        use crate::webui::wire::venue::*;

        let overrun = PatchOverrun {
            fixture: "End".to_string(),
            universe: 1,
            address: 510,
            footprint: 8,
            last: 517,
        };
        let overlap = PatchOverlap {
            first: "A".to_string(),
            second: "B".to_string(),
            first_gang: vec!["A".to_string()],
            second_gang: vec!["B".to_string(), "C".to_string()],
            universe: 1,
            from: 3,
            to: 4,
        };
        let patch = VenuePatch {
            spans: vec![PatchedFixture {
                fixture: "A".to_string(),
                universe: 1,
                address: 1,
                footprint: None,
                fixture_type: "par".to_string(),
                mode: Some("8ch".to_string()),
            }],
            overlaps: vec![overlap.clone()],
            overruns: vec![overrun.clone()],
        };
        // The overlap as the player's type names its fields: the one shape
        // change, from `a`, `b`, `a_gang`, `b_gang`.
        assert_eq!(
            json!(patch),
            json!({
                "spans": [json!({
                    "fixture": "A",
                    "universe": 1,
                    "address": 1,
                    "footprint": null,
                    "type": "par",
                    "mode": "8ch",
                })],
                "overlaps": [json!({
                    "first": "A",
                    "second": "B",
                    "first_gang": ["A"],
                    "second_gang": ["B", "C"],
                    "universe": 1,
                    "from": 3,
                    "to": 4,
                })],
                "overruns": [overrun],
            })
        );
        for venue_error in [
            None,
            Some(VenueError {
                venue: "v".to_string(),
                fixture: "B".to_string(),
                reason: "no type".to_string(),
            }),
        ] {
            let old_error = match &venue_error {
                Some(e) => json!({"venue": e.venue, "fixture": e.fixture, "reason": e.reason}),
                None => serde_json::Value::Null,
            };
            assert_eq!(
                json!(VenueSaved {
                    status: "saved".to_string(),
                    name: "v".to_string(),
                    reloaded: true,
                    version: "abc".to_string(),
                    venue_error,
                }),
                json!({
                    "status": "saved",
                    "name": "v",
                    "reloaded": true,
                    "version": "abc",
                    "venue_error": old_error,
                })
            );
        }
    }
}

mod fixture_test {
    use crate::lighting::fixture_test::{
        ControlSupport, DegreeRange, DimmerControl, FrameChannel, HzRange,
    };
    use crate::webui::wire::fixture_test::*;
    use serde_json::json;

    #[test]
    fn options_and_answer_match_the_old_json() {
        let controls = ControlSupport {
            color: true,
            dimmer: Some(DimmerControl::Folded),
            strobe: Some(HzRange {
                min_hz: 1.0,
                max_hz: 20.0,
            }),
            pan: Some(DegreeRange {
                min: -270.0,
                max: 270.0,
            }),
            tilt: None,
            levels: vec!["white".to_string()],
        };
        let options = TestOptions {
            fixture_type: "brick".to_string(),
            gdtf: true,
            modes: vec![
                TestMode {
                    name: Some("8ch".to_string()),
                    drivable: true,
                    error: None,
                    footprint: Some(8),
                    controls: Some(controls.clone()),
                    channels: vec![TestChannel {
                        offset: 1,
                        name: "red".to_string(),
                    }],
                },
                TestMode {
                    name: None,
                    drivable: false,
                    error: Some("bad".to_string()),
                    footprint: None,
                    controls: None,
                    channels: vec![],
                },
            ],
            default_mode: Some("8ch".to_string()),
            universes: vec![TestUniverse {
                universe: 1,
                name: None,
                patched: Some(true),
            }],
            olad: TestOlad {
                reachable: Some(true),
                port: Some(9090),
            },
            available: false,
            unavailable: Some(TestUnavailable::NoUniverses),
            unavailable_message: Some("none".to_string()),
        };
        let old_controls = json!({
            "color": true,
            "dimmer": "folded",
            "strobe": {"min_hz": 1.0, "max_hz": 20.0},
            "pan": {"min": -270.0, "max": 270.0},
            "tilt": null,
            "levels": ["white"],
        });
        assert_eq!(json!(controls), old_controls);
        assert_eq!(
            json!(options),
            json!({
                "fixture_type": "brick",
                "gdtf": true,
                "modes": [
                    json!({
                        "name": "8ch",
                        "drivable": true,
                        "error": null,
                        "footprint": 8,
                        "controls": old_controls,
                        "channels": [json!({"offset": 1, "name": "red"})],
                    }),
                    json!({
                        "name": null,
                        "drivable": false,
                        "error": "bad",
                        "footprint": null,
                        "controls": null,
                        "channels": [],
                    }),
                ],
                "default_mode": "8ch",
                "universes": [json!({"universe": 1, "name": null, "patched": true})],
                "olad": {"reachable": true, "port": 9090},
                "available": false,
                "unavailable": "no_universes",
                "unavailable_message": "none",
            })
        );

        let frame = FrameChannel {
            offset: 1,
            address: 10,
            name: "red".to_string(),
            value: 255,
            raw: false,
        };
        let answer = TestAnswer {
            active: true,
            universe: 1,
            address: 10,
            footprint: 1,
            fixture_type: "brick".to_string(),
            mode: None,
            expires_in_secs: 5.0,
            frame: vec![frame],
            warnings: vec![
                TestWarning::VenueOverlap {
                    fixtures: vec!["Front".to_string()],
                    message: "m1".to_string(),
                },
                TestWarning::PastUniverseEnd {
                    message: "m2".to_string(),
                },
                TestWarning::OladUnreachable {
                    message: "m3".to_string(),
                },
                TestWarning::UniverseUnpatched {
                    message: "m4".to_string(),
                },
                TestWarning::UnsupportedControl {
                    control: "pan".to_string(),
                    message: "m5".to_string(),
                },
            ],
        };
        assert_eq!(
            json!(answer),
            json!({
                "active": true,
                "universe": 1,
                "address": 10,
                "footprint": 1,
                "fixture_type": "brick",
                "mode": null,
                "expires_in_secs": 5.0,
                "frame": [json!({"offset": 1, "address": 10, "name": "red", "value": 255, "raw": false})],
                "warnings": [
                    json!({"kind": "venue_overlap", "fixtures": ["Front"], "message": "m1"}),
                    json!({"kind": "past_universe_end", "message": "m2"}),
                    json!({"kind": "olad_unreachable", "message": "m3"}),
                    json!({"kind": "universe_unpatched", "message": "m4"}),
                    json!({"kind": "unsupported_control", "control": "pan", "message": "m5"}),
                ],
            })
        );
    }
}

mod devices {
    use crate::webui::wire::devices::*;
    use serde_json::json;

    #[test]
    fn probe_and_noise_floor_match_the_old_json() {
        for (kind, name, reason, callbacks) in [
            (ProbeKind::Streaming, "streaming", None, Some(12)),
            (
                ProbeKind::CouldNotOpen,
                "could_not_open",
                Some("busy"),
                None,
            ),
            (ProbeKind::OpenedButSilent, "opened_but_silent", None, None),
        ] {
            assert_eq!(
                json!(AudioProbe {
                    outcome: kind,
                    ok: false,
                    reason: Some(reason.map(String::from)),
                    callbacks,
                    health: None,
                }),
                json!({"outcome": name, "ok": false, "reason": reason, "callbacks": callbacks})
            );
        }
        assert_eq!(
            json!(AudioProbe {
                outcome: ProbeKind::InUse,
                ok: true,
                reason: None,
                callbacks: None,
                health: Some(None),
            }),
            json!({"outcome": "in_use", "ok": true, "callbacks": null, "health": null})
        );
        assert_eq!(
            json!(NoiseFloor {
                peak: 0.5,
                rms: 0.25,
                low_freq_energy: 0.125,
                channel: 3,
                sample_rate: 48000,
                device_channels: 8,
            }),
            json!({
                "peak": 0.5,
                "rms": 0.25,
                "low_freq_energy": 0.125,
                "channel": 3,
                "sample_rate": 48000,
                "device_channels": 8,
            })
        );
    }
}

mod fixture_types {
    use std::collections::{BTreeMap, HashMap};

    use serde_json::json;

    use crate::lighting::findings::VenueError;
    use crate::lighting::types::{FixtureType, MovementLimits, StrobeCurve, Venue};
    use crate::webui::wire::fixture_types::*;

    fn par() -> FixtureType {
        FixtureType::new("par".to_string(), HashMap::from([("red".to_string(), 1)]))
    }

    #[test]
    fn fixture_types_match_the_old_json() {
        let ft = par();
        let summary = GdtfSummary {
            fixture: "Brick".to_string(),
            manufacturer: "Astera".to_string(),
            modes: 2,
            beam: Some(BeamBrief {
                beam_type: Some("Wash".to_string()),
                angle: None,
            }),
            thumbnail: None,
            used_by: 3,
            in_use: vec![ModeUse {
                mode: "8ch".to_string(),
                count: 3,
            }],
        };
        let old_summary = json!({
            "fixture": "Brick",
            "manufacturer": "Astera",
            "modes": 2,
            "beam": {"type": "Wash", "angle": null},
            "thumbnail": null,
            "used_by": 3,
            "in_use": [json!({"mode": "8ch", "count": 3})],
        });
        assert_eq!(json!(summary.clone()), old_summary);
        let native = FixtureTypeEntry {
            referential: false,
            footprint: Some(Some(4)),
            rich: false,
            fixture_type: ft.clone(),
            file: Some("par.light".to_string()),
            extension: Some("light".to_string()),
            gdtf: None,
            dsl: None,
        };
        let library = FixtureTypeEntry {
            referential: true,
            footprint: Some(None),
            rich: false,
            fixture_type: ft.clone(),
            file: None,
            extension: None,
            gdtf: Some(Some(summary)),
            dsl: None,
        };
        let list = FixtureTypeList {
            fixture_types: HashMap::from([
                ("par".to_string(), native),
                ("brick".to_string(), library),
            ]),
            errors: vec![LightingFileError {
                file: "x.light".to_string(),
                error: "bad".to_string(),
            }],
        };
        assert_eq!(
            json!(list),
            json!({
                "fixture_types": {
                    "par": {
                        "referential": false,
                        "footprint": 4,
                        "rich": false,
                        "fixture_type": ft,
                        "file": "par.light",
                        "extension": "light",
                    },
                    "brick": {
                        "referential": true,
                        "footprint": null,
                        "rich": false,
                        "fixture_type": ft,
                        "file": null,
                        "extension": null,
                        "gdtf": old_summary,
                    },
                },
                "errors": [json!({"file": "x.light", "error": "bad"})],
            })
        );
        for (file, dsl) in [(None, None), (Some("par.light"), Some("fixture_type"))] {
            let single = FixtureTypeEntry {
                referential: file.is_none(),
                footprint: None,
                rich: false,
                fixture_type: ft.clone(),
                file: file.map(String::from),
                extension: file.map(|_| "light".to_string()),
                gdtf: None,
                dsl: Some(dsl.map(String::from)),
            };
            assert_eq!(
                json!(single),
                json!({
                    "referential": file.is_none(),
                    "rich": false,
                    "fixture_type": ft,
                    "dsl": dsl,
                    "file": file,
                    "extension": file.map(|_| "light"),
                })
            );
        }
    }

    #[test]
    fn gdtf_pages_match_the_old_json() {
        let refused = GdtfMode {
            name: "bad".to_string(),
            footprint: 4,
            capabilities: None,
            cells: None,
            strobe_range: None,
            channels: None,
            refused: Some("why".to_string()),
        };
        let mode = GdtfMode {
            name: "8ch".to_string(),
            footprint: 8,
            capabilities: Some(vec!["color".to_string()]),
            cells: Some(0),
            strobe_range: Some(StrobeRange {
                min_hz: None,
                max_hz: 20.0,
            }),
            channels: Some(vec![(1, "red".to_string())]),
            refused: None,
        };
        let gdtf = FixtureTypeGdtf {
            archive: "lighting/library/b.gdtf".to_string(),
            rig: None,
            thumbnail: Some("t.png".to_string()),
            beam: Some(GdtfBeam {
                beam_type: None,
                beam_angle: Some(30.0),
                field_angle: None,
                luminous_flux: None,
                color_temperature: None,
                power: Some(10.0),
            }),
            about: None,
            venues: vec![TypeVenue {
                name: "club".to_string(),
                fixtures: vec![TypeVenueFixture {
                    name: "b1".to_string(),
                    mode: None,
                }],
            }],
            inspection: GdtfInspection {
                modes: vec![mode, refused],
            },
        };
        let name = "red".to_string();
        let red: Vec<(&u16, &String)> = vec![(&1, &name)];
        assert_eq!(
            json!(gdtf),
            json!({
                "archive": "lighting/library/b.gdtf",
                "rig": null,
                "thumbnail": "t.png",
                "beam": {
                    "type": null,
                    "beam_angle": 30.0,
                    "field_angle": null,
                    "luminous_flux": null,
                    "color_temperature": null,
                    "power": 10.0,
                },
                "about": null,
                "venues": [json!({"name": "club", "fixtures": [json!({"name": "b1", "mode": null})]})],
                "inspection": {"modes": [
                    json!({
                        "name": "8ch",
                        "footprint": 8,
                        "capabilities": ["color"],
                        "cells": 0,
                        "strobe_range": {"min_hz": null, "max_hz": 20.0},
                        "channels": red,
                    }),
                    json!({"name": "bad", "footprint": 4, "refused": "why"}),
                ]},
            })
        );
    }

    #[test]
    fn settings_deletes_and_venues_match_the_old_json() {
        let settings = FixtureSettingsData {
            name: "Brick".to_string(),
            movement: MovementLimits::default(),
            strobe_curve: None,
            strobe: Some(StrobeTable {
                steps: 0,
                automatic: StrobeCurve::Declared,
            }),
            version: "v".to_string(),
        };
        assert_eq!(
            json!(settings),
            json!({
                "name": "Brick",
                "movement": MovementLimits::default(),
                "strobe_curve": null,
                "strobe": {"steps": 0, "automatic": StrobeCurve::Declared},
                "version": "v",
            })
        );
        let saved = SettingsSaved {
            version: "v2".to_string(),
            rename: Some(Rename {
                from: "A".to_string(),
                to: "B".to_string(),
                venues: vec![RenamedLines {
                    venue: "club".to_string(),
                    file: "club.venue".to_string(),
                    lines: 2,
                }],
                lines: 2,
            }),
            venue_versions: BTreeMap::from([("club.venue".to_string(), "x".to_string())]),
            venue_error: Some(VenueError {
                venue: "club".to_string(),
                fixture: "b1".to_string(),
                reason: "r".to_string(),
            }),
        };
        assert_eq!(
            json!(saved),
            json!({
                "version": "v2",
                "rename": {
                    "from": "A",
                    "to": "B",
                    "venues": [json!({"venue": "club", "file": "club.venue", "lines": 2})],
                    "lines": 2,
                },
                "venue_versions": {"club.venue": "x"},
                "venue_error": {"venue": "club", "fixture": "b1", "reason": "r"},
            })
        );
        assert_eq!(
            json!(FixtureTypeDeleted {
                status: "deleted".to_string(),
                name: "A".to_string(),
                archive_removed: None,
                archive_kept: true,
            }),
            json!({"status": "deleted", "name": "A", "archive_removed": null, "archive_kept": true})
        );
        let venue = Venue::new("club".to_string(), HashMap::new());
        assert_eq!(
            json!(VenueList {
                venues: HashMap::new(),
                errors: None,
                versions: None,
            }),
            json!({"venues": {}})
        );
        assert_eq!(
            json!(VenueList {
                venues: HashMap::from([("club".to_string(), venue.clone())]),
                errors: Some(vec![]),
                versions: Some(HashMap::from([("club".to_string(), "v".to_string())])),
            }),
            json!({"venues": {"club": venue}, "errors": [], "versions": {"club": "v"}})
        );
        assert_eq!(
            json!(VenueDetail {
                venue: venue.clone(),
                dsl: "venue".to_string(),
                version: "v".to_string(),
            }),
            json!({"venue": venue, "dsl": "venue", "version": "v"})
        );
    }

    #[test]
    fn venue_scene_matches_the_old_json() {
        let scene = VenueScene {
            fixtures: BTreeMap::from([(
                "p1".to_string(),
                SceneFixture {
                    tags: vec!["front".to_string()],
                    fixture_type: "par".to_string(),
                    mode: None,
                    position: Some([0.0, 1.0, 2.0]),
                    rotation: None,
                    beam_angle: Some(40.0),
                    rig: None,
                },
            )]),
            venue: SceneVenue {
                name: "club".to_string(),
                focus_points: BTreeMap::new(),
                scenery: None,
                scenery_error: Some("gone".to_string()),
            },
        };
        assert_eq!(
            json!(scene),
            json!({
                "fixtures": {"p1": {
                    "tags": ["front"],
                    "type": "par",
                    "mode": null,
                    "position": [0.0, 1.0, 2.0],
                    "rotation": null,
                    "beam_angle": 40.0,
                    "rig": null,
                }},
                "venue": {
                    "name": "club",
                    "focus_points": {},
                    "scenery": null,
                    "scenery_error": "gone",
                },
            })
        );
    }
}

mod mvr {
    use std::collections::BTreeMap;

    use serde_json::json;

    use crate::lighting::findings::VenueError;
    use crate::lighting::mvr_types::*;
    use crate::webui::wire::mvr::*;

    fn plan() -> MvrPlan {
        MvrPlan {
            venue_name: "club".to_string(),
            venue_file: "lighting/venues/club.venue".to_string(),
            archive: "lighting/mvr/club.mvr".to_string(),
            merge: false,
            origin: [0.0, 1.0, 0.0],
            fixture_types: vec![],
            fixtures: vec![],
            removed_fixtures: vec![],
            kept_fixtures: vec![],
            focus_points: vec![],
            removed_focus_points: vec![],
            kept_focus_points: vec![],
            scenery_objects: 0,
            scenery_meshes_undrawn: 0,
            warnings: vec!["w".to_string()],
        }
    }

    fn export() -> MvrExport {
        MvrExport {
            venue: "club".to_string(),
            output: "club.mvr".to_string(),
            fixtures: 1,
            focus_points: 0,
            embedded_gdtfs: vec![],
            generated_gdtfs: BTreeMap::new(),
            positioned_fixtures: 1,
            fixed_fixtures: vec![FixedFixture {
                name: "p1".to_string(),
                focus: None,
            }],
            warnings: vec![],
        }
    }

    #[test]
    fn mvr_answers_match_the_old_json() {
        let scene = SceneView {
            fixtures: vec![],
            focus_points: vec![],
            scenery: vec![],
            deck_mm: None,
        };
        let old_scene = json!(scene);
        assert_eq!(
            json!(MvrInspection {
                file_name: "club.mvr".to_string(),
                report: plan(),
                scene,
            }),
            json!({"file_name": "club.mvr", "report": plan(), "scene": old_scene})
        );
        assert_eq!(
            json!(MvrImportAnswer {
                write: false,
                plan: Some(plan()),
                report: None,
                reloaded: None,
                venue_error: None,
            }),
            json!({"write": false, "plan": plan()})
        );
        let report = || MvrImport {
            plan: plan(),
            written: vec!["a".to_string()],
            distillation_warnings: BTreeMap::new(),
        };
        let error = VenueError {
            venue: "club".to_string(),
            fixture: "p1".to_string(),
            reason: "r".to_string(),
        };
        assert_eq!(
            json!(MvrImportAnswer {
                write: true,
                plan: None,
                report: Some(report()),
                reloaded: Some(true),
                venue_error: Some(Some(error.clone())),
            }),
            json!({
                "write": true,
                "report": report(),
                "reloaded": true,
                "venue_error": {"venue": "club", "fixture": "p1", "reason": "r"},
            })
        );
        let mut old_summary = serde_json::to_value(export()).unwrap();
        old_summary["file_name"] = json!("club.mvr");
        assert_eq!(
            json!(MvrExportSummary {
                file_name: "club.mvr".to_string(),
                export: export(),
            }),
            old_summary
        );
        assert_eq!(
            json!(MvrKept {
                kept: "club.mvr".to_string()
            }),
            json!({"kept": "club.mvr"})
        );
        assert_eq!(
            json!(AimPoints {
                created: vec![AimPoint {
                    name: "p1 aim".to_string(),
                    fixture: "p1".to_string(),
                    point: [1.0, 2.0, 0.0],
                }],
                reloaded: false,
                version: "v".to_string(),
                venue_error: None,
            }),
            json!({
                "created": [json!({"name": "p1 aim", "fixture": "p1", "point": [1.0, 2.0, 0.0]})],
                "reloaded": false,
                "version": "v",
                "venue_error": null,
            })
        );
    }
}
