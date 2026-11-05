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
//! The exports answer exactly what the player's own functions answer: every
//! measure and (half-)beat of every grid and tempo map here, and a sweep of
//! times through each. The cases are the web UI's mock songs
//! (`e2e/mock-server/test-data.ts`), the grids the player's own tests use,
//! and tempo maps with every feature a `tempo:` block has (lead-in, snap and
//! ramped tempo changes, signature changes on and off the bar line).
//!
//! With `MTRACK_RULES_DUMP=<path>` the inputs and answers are also written
//! as JSON, for a one-off comparison against another implementation.

use std::time::Duration;

use mtrack_rules_wasm::metronome_config::{MetronomeChange, MetronomeConfig, Subdivision};
use mtrack_rules_wasm::tempo::position_label_for;
use mtrack_rules_wasm::tempo_config::{TempoChangeConfig, TempoConfig, TransitionConfig};
use mtrack_rules_wasm::{
    grid_position, signature, tempo_normalized, tempo_point, tempo_verdict, BeatGrid, GridQuery,
    TempoQuery, TempoSource,
};
use serde::Serialize;

/// A grid from measure beat-counts at a steady tempo, starting at `lead_in`
/// seconds, with a trailing downbeat so the last measure is complete.
fn steady_grid(lead_in: f64, bpm: f64, measures: &[usize]) -> BeatGrid {
    let spacing = 60.0 / bpm;
    let mut beats = Vec::new();
    let mut measure_starts = Vec::new();
    let mut t = lead_in;
    for count in measures {
        measure_starts.push(beats.len());
        for _ in 0..*count {
            beats.push(t);
            t += spacing;
        }
    }
    measure_starts.push(beats.len());
    beats.push(t);
    BeatGrid {
        beats,
        measure_starts,
    }
}

fn grids() -> Vec<(&'static str, BeatGrid)> {
    let mut grids = vec![
        // Test Song Beta / Gamma: 16 measures of 4/4 at 120 BPM, no trailing
        // downbeat (the grid ends inside its last measure).
        (
            "mock-beta",
            BeatGrid {
                beats: (0..64).map(|i| i as f64 * 0.5).collect(),
                measure_starts: (0..16).map(|i| i * 4).collect(),
            },
        ),
        ("steady-4-4", steady_grid(0.0, 120.0, &[4, 4, 4, 4])),
        ("lead-in-3-4", steady_grid(1.25, 90.0, &[3, 3, 3])),
        ("short-bar", steady_grid(0.0, 80.0, &[4, 4, 4, 2, 4, 4])),
        ("seven-eight", steady_grid(0.0, 160.0, &[7, 7, 7])),
        (
            "pickup",
            BeatGrid {
                beats: vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0],
                measure_starts: vec![1, 5],
            },
        ),
        (
            "empty",
            BeatGrid {
                beats: vec![],
                measure_starts: vec![],
            },
        ),
    ];
    for (name, tempo) in tempos() {
        if let Ok(map) = tempo.to_tempo_map() {
            grids.push((
                name,
                BeatGrid::from_tempo_map(&map, Duration::from_secs(40)),
            ));
        }
    }
    grids
}

fn change(measure: u32, beat: Option<f64>) -> TempoChangeConfig {
    TempoChangeConfig {
        measure,
        beat,
        bpm: None,
        time_signature: None,
        transition: None,
    }
}

fn tempos() -> Vec<(&'static str, TempoConfig)> {
    vec![
        (
            "plain",
            TempoConfig {
                bpm: 120.0,
                time_signature: "4/4".into(),
                start: None,
                changes: vec![],
            },
        ),
        (
            "lead-in-six-eight",
            TempoConfig {
                bpm: 96.0,
                time_signature: "6/8".into(),
                start: Some(0.35),
                changes: vec![],
            },
        ),
        (
            "snap-and-ramp",
            TempoConfig {
                bpm: 118.0,
                time_signature: "4/4".into(),
                start: Some(0.35),
                changes: vec![
                    TempoChangeConfig {
                        bpm: Some(126.0),
                        transition: Some(TransitionConfig::Measures(2.0)),
                        ..change(5, None)
                    },
                    TempoChangeConfig {
                        bpm: Some(96.0),
                        time_signature: Some("6/8".into()),
                        ..change(9, None)
                    },
                    TempoChangeConfig {
                        bpm: Some(140.0),
                        transition: Some(TransitionConfig::Beats(4.0)),
                        ..change(12, None)
                    },
                ],
            },
        ),
        (
            "mid-bar-signature",
            TempoConfig {
                bpm: 100.0,
                time_signature: "4/4".into(),
                start: None,
                changes: vec![
                    TempoChangeConfig {
                        time_signature: Some("7/8".into()),
                        ..change(3, Some(3.0))
                    },
                    TempoChangeConfig {
                        bpm: Some(110.0),
                        ..change(6, Some(2.5))
                    },
                    TempoChangeConfig {
                        time_signature: Some("3/4".into()),
                        ..change(8, None)
                    },
                ],
            },
        ),
        (
            "unsorted",
            TempoConfig {
                bpm: 120.0,
                time_signature: "4/4".into(),
                start: None,
                changes: vec![
                    TempoChangeConfig {
                        bpm: Some(100.0),
                        ..change(10, None)
                    },
                    TempoChangeConfig {
                        bpm: Some(90.0),
                        ..change(5, None)
                    },
                ],
            },
        ),
        (
            "repeated",
            TempoConfig {
                bpm: 120.0,
                time_signature: "4/4".into(),
                start: None,
                changes: vec![
                    TempoChangeConfig {
                        bpm: Some(100.0),
                        ..change(5, None)
                    },
                    TempoChangeConfig {
                        bpm: Some(90.0),
                        ..change(5, Some(1.0))
                    },
                ],
            },
        ),
        (
            "bad-signature",
            TempoConfig {
                bpm: 120.0,
                time_signature: "5/3".into(),
                start: None,
                changes: vec![],
            },
        ),
        (
            "zero-bpm",
            TempoConfig {
                bpm: 0.0,
                time_signature: "4/4".into(),
                start: None,
                changes: vec![],
            },
        ),
    ]
}

fn metronomes() -> Vec<(&'static str, MetronomeConfig)> {
    vec![
        ("default", MetronomeConfig::default()),
        (
            "grouping",
            MetronomeConfig {
                accent: vec![3, 2, 2],
                ..MetronomeConfig::default()
            },
        ),
        (
            "pattern",
            MetronomeConfig {
                accents: vec![3, 0, 2],
                subdivision: Subdivision::Even(2),
                ..MetronomeConfig::default()
            },
        ),
        (
            "changes",
            MetronomeConfig {
                accents: vec![3, 1, 2, 1],
                changes: vec![
                    MetronomeChange {
                        measure: 5,
                        accents: Some(vec![3, 2]),
                        subdivision: None,
                    },
                    MetronomeChange {
                        measure: 2,
                        accents: None,
                        subdivision: Some(Subdivision::Even(3)),
                    },
                    MetronomeChange {
                        measure: 5,
                        accents: Some(vec![2, 2, 2, 3, 3, 3, 3, 3]),
                        subdivision: None,
                    },
                ],
                ..MetronomeConfig::default()
            },
        ),
    ]
}

fn half_beats(upto: usize) -> Vec<f64> {
    (0..=upto * 2).map(|i| 1.0 + i as f64 * 0.5).collect()
}

fn secs_sweep(end: f64) -> Vec<f64> {
    let mut secs = vec![-1.0, 0.0];
    let mut t = 0.0;
    while t < end + 2.0 {
        secs.push(t);
        t += 0.137;
    }
    secs
}

#[derive(Serialize)]
struct Dump {
    grids: Vec<DumpGrid>,
    tempos: Vec<DumpTempo>,
    metronomes: Vec<DumpMetronome>,
    signatures: Vec<(String, Option<(u32, u32)>)>,
}

#[derive(Serialize)]
struct DumpGrid {
    name: String,
    grid: BeatGrid,
    positions: Vec<((u32, f64), Option<mtrack_rules_wasm::GridPosition>)>,
    times: Vec<(f64, Option<mtrack_rules_wasm::GridPosition>)>,
}

#[derive(Serialize)]
struct DumpTempo {
    name: String,
    config: TempoConfig,
    verdict: Option<String>,
    normalized: mtrack_rules_wasm::NormalizedTempo,
    positions: Vec<((u32, f64), Option<mtrack_rules_wasm::TempoPoint>)>,
    times: Vec<(f64, Option<mtrack_rules_wasm::TempoPoint>)>,
}

#[derive(Serialize)]
struct DumpMetronome {
    name: String,
    config: MetronomeConfig,
    patterns: Vec<((u32, u32), Vec<u8>)>,
}

#[test]
fn exports_equal_the_players_rules() {
    let mut dump = Dump {
        grids: vec![],
        tempos: vec![],
        metronomes: vec![],
        signatures: vec![],
    };

    for (name, grid) in grids() {
        let measures = grid.measure_starts.len() + 1;
        let mut positions = vec![];
        for measure in 0..=measures as u32 {
            for beat in half_beats(9) {
                let got = grid_position(&grid, GridQuery::Position { measure, beat });
                let expected = measure.checked_sub(1).and_then(|m| {
                    let secs = grid.beat_time(m as usize, beat)?;
                    Some((secs, grid.beats_in_measure(m as usize)? as u32))
                });
                assert_eq!(
                    got.map(|p| (p.secs, p.beats_in_measure)),
                    expected,
                    "{name}: grid position {measure}/{beat}"
                );
                if let Some(p) = got {
                    assert_eq!((p.measure, p.beat), (measure, beat), "{name}: echo");
                }
                positions.push(((measure, beat), got));
            }
        }
        let mut times = vec![];
        let end = grid.beats.last().copied().unwrap_or(0.0);
        let mut sweep = secs_sweep(end);
        sweep.extend(grid.beats.iter().flat_map(|b| [*b, b + 1e-4, b - 1e-4]));
        for secs in sweep {
            let got = grid_position(&grid, GridQuery::Time { secs });
            let expected = grid.position_at(secs).map(|(m, b)| {
                (
                    m as u32 + 1,
                    b as f64,
                    grid.beat_time(m, b as f64)
                        .expect("the nearest beat exists"),
                    grid.beats_in_measure(m).expect("its measure exists") as u32,
                )
            });
            assert_eq!(
                got.map(|p| (p.measure, p.beat, p.secs, p.beats_in_measure)),
                expected,
                "{name}: grid time {secs}"
            );
            times.push((secs, got));
        }
        dump.grids.push(DumpGrid {
            name: name.into(),
            grid,
            positions,
            times,
        });
    }

    for (name, config) in tempos() {
        let source = TempoSource::Config {
            config: config.clone(),
        };
        let verdict = tempo_verdict(&config);
        assert_eq!(verdict, config.to_tempo_map().err(), "{name}: verdict");
        let normalized = tempo_normalized(&config);
        if verdict.is_none() {
            assert_eq!(
                normalized.config, config,
                "{name}: a valid config is left as it is"
            );
            assert_eq!(normalized.duplicates, 0);
        }
        let map = config.to_tempo_map().ok();

        let mut positions = vec![];
        for measure in 0..=14 {
            for beat in half_beats(9) {
                let got = tempo_point(&source, TempoQuery::Position { measure, beat });
                let expected = map.as_ref().and_then(|map| {
                    let at = map.measure_to_time_with_offset(measure, beat, 0, 0.0)?;
                    let sig = map.time_signature_at_measure(measure);
                    Some((
                        at.as_secs_f64() * 1000.0,
                        map.bpm_at_time(at, 0.0),
                        (sig.numerator, sig.denominator),
                        position_label_for(measure, beat),
                    ))
                });
                assert_eq!(
                    got.as_ref().map(|p| (p.ms, p.bpm, p.sig, p.label.clone())),
                    expected,
                    "{name}: tempo position {measure}/{beat}"
                );
                positions.push(((measure, beat), got));
            }
        }
        let mut times = vec![];
        for secs in secs_sweep(40.0) {
            let ms = secs * 1000.0;
            let got = tempo_point(&source, TempoQuery::Time { ms });
            let expected = map.as_ref().and_then(|map| {
                if ms < 0.0 {
                    return None;
                }
                let at = Duration::from_secs_f64(secs);
                let (measure, beat) = map.time_to_measure_beat(at);
                let sig = map.time_signature_at_time(at, 0.0);
                Some((
                    measure,
                    beat,
                    map.bpm_at_time(at, 0.0),
                    (sig.numerator, sig.denominator),
                    map.position_label(at),
                ))
            });
            assert_eq!(
                got.as_ref()
                    .map(|p| (p.measure, p.beat, p.bpm, p.sig, p.label.clone())),
                expected,
                "{name}: tempo time {ms}"
            );
            times.push((ms, got));
        }
        dump.tempos.push(DumpTempo {
            name: name.into(),
            config,
            verdict,
            normalized,
            positions,
            times,
        });
    }

    // Normalizing puts an unsorted config in the order a save needs and
    // keeps the last of a repeated position.
    let unsorted = tempos()
        .into_iter()
        .find(|(n, _)| *n == "unsorted")
        .unwrap()
        .1;
    let fixed = tempo_normalized(&unsorted);
    assert_eq!(fixed.duplicates, 0);
    assert_eq!(tempo_verdict(&fixed.config), None);
    assert_eq!(
        fixed
            .config
            .changes
            .iter()
            .map(|c| c.measure)
            .collect::<Vec<_>>(),
        vec![5, 10]
    );
    let repeated = tempos()
        .into_iter()
        .find(|(n, _)| *n == "repeated")
        .unwrap()
        .1;
    let fixed = tempo_normalized(&repeated);
    assert_eq!(fixed.duplicates, 1);
    assert_eq!(tempo_verdict(&fixed.config), None);
    assert_eq!(fixed.config.changes.len(), 1);
    assert_eq!(fixed.config.changes[0].bpm, Some(90.0));

    for (name, config) in metronomes() {
        let mut patterns = vec![];
        for measure in 1..=8 {
            for beats in 1..=9 {
                patterns.push((
                    (measure, beats),
                    config.accent_pattern_at(measure, beats as usize),
                ));
            }
        }
        dump.metronomes.push(DumpMetronome {
            name: name.into(),
            config,
            patterns,
        });
    }

    for raw in [
        "4/4", "7/8", "6/8", " 3 / 4 ", "5/3", "0/4", "4/0", "4", "", "abc", "12/16", "33/8",
        "4/64",
    ] {
        let got = signature(raw).map(|s| s.0);
        let expected = mtrack_rules_wasm::tempo_config::parse_time_signature(raw)
            .ok()
            .map(|s| (s.numerator, s.denominator));
        assert_eq!(got, expected, "signature {raw:?}");
        dump.signatures.push((raw.into(), got));
    }

    if let Ok(path) = std::env::var("MTRACK_RULES_DUMP") {
        std::fs::write(path, serde_json::to_string(&dump).unwrap()).unwrap();
    }
}
