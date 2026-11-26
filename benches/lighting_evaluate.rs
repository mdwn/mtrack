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

//! How long reading a whole show offline takes: every frame of a two-minute
//! show with forty fixtures and a cue on every beat, read as snapshots, as
//! the bytes the wire holds, and as those bytes on several threads.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

use mtrack::lighting::effects::FixtureInfo;
use mtrack::lighting::evaluate::{evaluate_show_each, evaluate_show_parallel, snapshot};
use mtrack::lighting::parser::{parse_light_shows, LightShow};

const FIXTURES: usize = 40;
const SECONDS: u64 = 120;
/// The rate a rig is sent frames at.
const FRAMES_PER_SECOND: u64 = 44;
const CHANNELS: [&str; 6] = ["dimmer", "red", "green", "blue", "white", "strobe"];

fn fixture_names() -> Vec<String> {
    (0..FIXTURES).map(|i| format!("f{i:03}")).collect()
}

/// Forty six-channel fixtures on universe 1, one after another.
fn fixtures() -> Vec<FixtureInfo> {
    fixture_names()
        .into_iter()
        .zip((1u16..).step_by(CHANNELS.len()))
        .map(|(name, address)| {
            let channels: HashMap<String, u16> = CHANNELS
                .iter()
                .zip(1..)
                .map(|(channel, offset)| (channel.to_string(), offset))
                .collect();
            FixtureInfo::new(name, 1, address, "PAR".to_string(), channels.into(), None)
        })
        .collect()
}

/// A show at 120 bpm with the fixtures in four groups: a wash that changes
/// every measure, a pulse on every beat, and cycles, strobes, fades and
/// chases every few measures.
fn show() -> Vec<LightShow> {
    let names = fixture_names();
    let group = |g: usize| -> String {
        let of: Vec<&str> = names
            .iter()
            .skip(g)
            .step_by(4)
            .map(String::as_str)
            .collect();
        of.join(", ")
    };
    let (wash, beat, flash, sweep) = (group(0), group(1), group(2), group(3));
    let colors = ["red", "green", "blue", "purple", "orange", "cyan"];
    let mut source = String::from(
        "tempo {\n    start: 0.0s\n    bpm: 120\n    time_signature: 4/4\n}\n\nshow \"Bench\" {\n",
    );
    // Two seconds a measure.
    for measure in 1..=SECONDS / 2 {
        let color = colors[measure as usize % colors.len()];
        let _ = writeln!(source, "    @{measure}/1");
        let _ = writeln!(
            source,
            "    {wash}: static color: \"{color}\", dimmer: 100%, layer: background, blend_mode: replace, duration: 2s"
        );
        let _ = writeln!(
            source,
            "    {beat}: pulse duration: 500ms, layer: foreground, blend_mode: multiply"
        );
        if measure % 2 == 0 {
            let _ = writeln!(
                source,
                "    {sweep}: cycle color: \"red\", color: \"green\", color: \"blue\", speed: 1measure, layer: midground, duration: 4s"
            );
        }
        if measure % 4 == 0 {
            let _ = writeln!(
                source,
                "    {flash}: strobe frequency: 8, duration: 1s, layer: foreground, blend_mode: add"
            );
            let _ = writeln!(
                source,
                "    {sweep}: dimmer start_level: 0.0, end_level: 1.0, duration: 2s, curve: linear, layer: foreground, blend_mode: replace"
            );
        }
        if measure % 8 == 0 {
            let _ = writeln!(
                source,
                "    {wash}: chase duration: 1s, direction: left_to_right"
            );
        }
        for at in 2..=4 {
            let _ = writeln!(source, "    @{measure}/{at}");
            let _ = writeln!(
                source,
                "    {beat}: pulse duration: 500ms, layer: foreground, blend_mode: multiply"
            );
        }
    }
    source.push_str("}\n");
    parse_light_shows(&source)
        .expect("the show parses")
        .into_values()
        .collect()
}

fn bench_evaluate(c: &mut Criterion) {
    let (shows, fixtures) = (show(), fixtures());
    let times: Vec<Duration> = (0..=SECONDS * FRAMES_PER_SECOND)
        .map(|frame| Duration::from_secs_f64(frame as f64 / FRAMES_PER_SECOND as f64))
        .collect();
    let none = HashMap::new();

    let mut group = c.benchmark_group("lighting_evaluate");
    group.sample_size(10);
    group.throughput(Throughput::Elements(times.len() as u64));

    group.bench_function("snapshot_each_frame", |b| {
        b.iter(|| {
            evaluate_show_each(
                shows.clone(),
                &fixtures,
                &none,
                None,
                &times,
                |effect| effect,
                |engine, time| {
                    black_box(snapshot(engine, time));
                },
            )
        })
    });

    group.bench_function("wire_each_frame", |b| {
        b.iter(|| {
            evaluate_show_each(
                shows.clone(),
                &fixtures,
                &none,
                None,
                &times,
                |effect| effect,
                |engine, _| {
                    black_box(engine.wire().universe(1));
                },
            )
        })
    });

    group.bench_function("wire_each_frame_on_4_threads", |b| {
        b.iter(|| {
            black_box(evaluate_show_parallel(
                &shows,
                &fixtures,
                &none,
                None,
                &times,
                4,
                |effect| effect,
                |engine, _| engine.wire().universe(1),
            ))
        })
    });

    group.finish();
}

criterion_group!(benches, bench_evaluate);
criterion_main!(benches);
