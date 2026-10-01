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
//! Benchmarks for the lighting engine hot path (`EffectEngine::update`).
//!
//! The effects loop calls `update` once per frame at 44 Hz, so one call has a
//! budget of about 22.7 ms. These scenarios measure that call for rigs of
//! increasing size: each universe is packed with 85 six-channel RGBW strobe
//! fixtures (510 of its 512 channels), and a fixed show of effects targets
//! every fixture at once, which is the worst case for the engine since its
//! cost scales with fixtures × active effects, not with universes as such.
//!
//! Compare against a saved baseline when touching the engine:
//!
//! ```sh
//! cargo bench --bench lighting -- --save-baseline pre-change   # before changes
//! cargo bench --bench lighting -- --baseline pre-change        # after changes
//! ```
use std::collections::HashMap;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use mtrack::lighting::effects::{
    BlendMode, ChaseDirection, ChasePattern, Color, CycleDirection, CycleTransition, DimmerCurve,
    EffectInstance, EffectLayer, EffectType, FixtureInfo, TempoAwareFrequency, TempoAwareSpeed,
};
use mtrack::lighting::engine::EffectEngine;

/// Channels per fixture: dimmer, red, green, blue, white, strobe.
const FIXTURE_CHANNELS: u16 = 6;
/// Fixtures that fit in one 512-channel universe.
const FIXTURES_PER_UNIVERSE: u16 = 512 / FIXTURE_CHANNELS;
/// One effects-loop frame.
const FRAME: Duration = Duration::from_nanos(1_000_000_000 / 44);
/// Long enough that no effect reaches its end during a benchmark run; engine
/// time advances only by `FRAME` per iteration.
const FOREVER: Duration = Duration::from_secs(60 * 60 * 24);

fn fixture(name: String, universe: u16, address: u16) -> FixtureInfo {
    let channels: HashMap<String, u16> = [
        ("dimmer", 1),
        ("red", 2),
        ("green", 3),
        ("blue", 4),
        ("white", 5),
        ("strobe", 6),
    ]
    .into_iter()
    .map(|(n, o)| (n.to_string(), o))
    .collect();
    FixtureInfo::new(
        name,
        universe,
        address,
        "RGBW_Strobe".to_string(),
        channels,
        Some(20.0),
    )
}

/// An engine with `universes` fully packed universes. Returns the engine and
/// the names of every fixture in it, in patch order.
fn build_rig(universes: u16) -> (EffectEngine, Vec<String>) {
    let mut engine = EffectEngine::new();
    let mut names = Vec::new();
    for u in 1..=universes {
        for i in 0..FIXTURES_PER_UNIVERSE {
            let name = format!("u{u}_f{i}");
            let mut f = fixture(name.clone(), u, i * FIXTURE_CHANNELS + 1);
            // A position so chases order themselves spatially rather than by name.
            f.position = Some([f64::from(i), f64::from(u), 3.0]);
            engine.register_fixture(f);
            names.push(name);
        }
    }
    (engine, names)
}

fn effect(
    id: &str,
    kind: EffectType,
    targets: &[String],
    layer: EffectLayer,
    blend: BlendMode,
) -> EffectInstance {
    let mut e = EffectInstance::new(
        id.to_string(),
        kind,
        targets.to_vec(),
        None,
        Some(FOREVER),
        None,
    );
    e.layer = layer;
    e.blend_mode = blend;
    e
}

/// The pool of effects a show draws from, in the order they are layered. Each
/// one targets every fixture in the rig.
fn show_effects(targets: &[String]) -> Vec<EffectInstance> {
    vec![
        effect(
            "wash",
            EffectType::Static {
                parameters: [("dimmer", 1.0), ("red", 0.8), ("green", 0.2), ("blue", 0.5)]
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
                duration: FOREVER,
            },
            targets,
            EffectLayer::Background,
            BlendMode::Replace,
        ),
        effect(
            "rainbow",
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Fixed(0.25),
                saturation: 1.0,
                brightness: 1.0,
                duration: FOREVER,
            },
            targets,
            EffectLayer::Background,
            BlendMode::Replace,
        ),
        effect(
            "chase",
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Fade,
                duration: FOREVER,
            },
            targets,
            EffectLayer::Midground,
            BlendMode::Multiply,
        ),
        effect(
            "pulse",
            EffectType::Pulse {
                base_level: 0.3,
                pulse_amplitude: 0.7,
                frequency: TempoAwareFrequency::Fixed(2.0),
                duration: FOREVER,
            },
            targets,
            EffectLayer::Midground,
            BlendMode::Multiply,
        ),
        effect(
            "cycle",
            EffectType::ColorCycle {
                colors: vec![
                    Color::new(255, 0, 0),
                    Color::new(0, 255, 0),
                    Color::new(0, 0, 255),
                ],
                speed: TempoAwareSpeed::Fixed(0.5),
                direction: CycleDirection::Forward,
                transition: CycleTransition::Fade,
                duration: FOREVER,
            },
            targets,
            EffectLayer::Background,
            BlendMode::Add,
        ),
        effect(
            "fade",
            EffectType::Dimmer {
                start_level: 1.0,
                end_level: 0.0,
                duration: FOREVER,
                curve: DimmerCurve::Sine,
            },
            targets,
            EffectLayer::Midground,
            BlendMode::Multiply,
        ),
        effect(
            "strobe",
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(8.0),
                duration: FOREVER,
            },
            targets,
            EffectLayer::Foreground,
            BlendMode::Replace,
        ),
        effect(
            "accent",
            EffectType::Static {
                parameters: [("white", 0.4)]
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
                duration: FOREVER,
            },
            targets,
            EffectLayer::Foreground,
            BlendMode::Add,
        ),
    ]
}

/// A rig with the first `effects` of the show running on every fixture.
fn build_show(universes: u16, effects: usize) -> EffectEngine {
    let (mut engine, names) = build_rig(universes);
    for e in show_effects(&names).into_iter().take(effects) {
        engine
            .start_effect(e)
            .expect("effect targets registered fixtures");
    }
    // One frame so every effect has a start time behind it.
    engine
        .update(FRAME, Some(Duration::ZERO))
        .expect("first frame");
    engine
}

/// Drives one frame. Engine time advances with each call, so consecutive
/// iterations render consecutive frames of the same show.
fn frame(engine: &mut EffectEngine, song_time: &mut Duration) -> usize {
    *song_time += FRAME;
    engine.update(FRAME, Some(*song_time)).expect("frame").len()
}

/// Cost of one frame as the rig grows, with a fixed four-effect show.
fn bench_universes(c: &mut Criterion) {
    let mut group = c.benchmark_group("lighting/universes");
    for universes in [1u16, 2, 4, 8, 16] {
        let fixtures = u64::from(universes) * u64::from(FIXTURES_PER_UNIVERSE);
        group.throughput(Throughput::Elements(fixtures));
        group.bench_with_input(
            BenchmarkId::from_parameter(universes),
            &universes,
            |b, &u| {
                let mut engine = build_show(u, 4);
                let mut song_time = Duration::ZERO;
                b.iter(|| frame(&mut engine, &mut song_time));
            },
        );
    }
    group.finish();
}

/// Cost of one frame as more effects stack on a fixed four-universe rig.
fn bench_effects(c: &mut Criterion) {
    let mut group = c.benchmark_group("lighting/effects");
    let fixtures = 4 * u64::from(FIXTURES_PER_UNIVERSE);
    group.throughput(Throughput::Elements(fixtures));
    for effects in [1usize, 2, 4, 8] {
        group.bench_with_input(BenchmarkId::from_parameter(effects), &effects, |b, &e| {
            let mut engine = build_show(4, e);
            let mut song_time = Duration::ZERO;
            b.iter(|| frame(&mut engine, &mut song_time));
        });
    }
    group.finish();
}

/// The idle path: a rig with nothing running returns the cached frame.
fn bench_idle(c: &mut Criterion) {
    let mut group = c.benchmark_group("lighting/idle");
    group.bench_function("16_universes_no_effects", |b| {
        let mut engine = build_show(16, 0);
        let mut song_time = Duration::ZERO;
        b.iter(|| frame(&mut engine, &mut song_time));
    });
    group.finish();
}

criterion_group!(benches, bench_universes, bench_effects, bench_idle);
criterion_main!(benches);
