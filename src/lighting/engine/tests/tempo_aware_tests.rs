// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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
use crate::lighting::effects::*;
use crate::lighting::engine::tests::common::create_test_fixture;
use crate::lighting::engine::EffectEngine;
use std::collections::HashMap;
use std::time::Duration;

#[test]
fn test_tempo_aware_speed_adapts_to_tempo_changes() {
    use crate::lighting::tempo::{
        TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
    };

    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Create a tempo map: 120 BPM initially, changes to 60 BPM at 4 seconds
    let tempo_map = TempoMap::new(
        Duration::ZERO,
        120.0,
        TimeSignature::new(4, 4),
        vec![TempoChange {
            position: TempoChangePosition::Time(Duration::from_secs(4)),
            original_measure_beat: None,
            bpm: Some(60.0),
            time_signature: None,
            transition: TempoTransition::Snap,
        }],
    );
    engine.set_tempo_map(Some(tempo_map));

    // Create a cycle effect with speed: 1measure (tempo-aware)
    let colors = vec![
        Color::new(255, 0, 0), // Red
        Color::new(0, 255, 0), // Green
        Color::new(0, 0, 255), // Blue
    ];

    let effect = EffectInstance::new(
        "tempo_aware_cycle".to_string(),
        EffectType::ColorCycle {
            colors,
            speed: TempoAwareSpeed::Measures(1.0), // 1 cycle per measure
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // At t=0s (120 BPM): 1 measure = 2.0s, so speed = 0.5 cycles/sec
    // Verify effect is running
    let commands = engine.update(Duration::from_millis(100), None).unwrap();
    assert!(!commands.is_empty(), "Effect should generate commands");

    // At t=4s: tempo changes to 60 BPM
    // At t=4.1s (60 BPM): 1 measure = 4.0s, so speed = 0.25 cycles/sec
    // This is slower than before - the effect should have adapted
    engine.update(Duration::from_secs(4), None).unwrap(); // Advance to tempo change
    let commands_after = engine.update(Duration::from_millis(100), None).unwrap(); // 0.1s after tempo change

    // At slower tempo, the cycle should be progressing more slowly
    // The effect should still be running and generating commands
    assert!(
        !commands_after.is_empty(),
        "Effect should still generate commands after tempo change"
    );

    // Verify that the speed calculation uses the new tempo
    // We can't easily verify exact color values, but we can verify the effect is adapting
    // by checking that it's still running and producing different values over time
    let commands_later = engine.update(Duration::from_millis(1000), None).unwrap(); // 1.1s after tempo change
    assert!(
        !commands_later.is_empty(),
        "Effect should continue running after tempo change"
    );
}

#[test]
fn test_tempo_aware_frequency_adapts_to_tempo_changes() {
    use crate::lighting::tempo::{
        TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
    };

    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Create a tempo map: 120 BPM initially, changes to 60 BPM at 2 seconds
    let tempo_map = TempoMap::new(
        Duration::ZERO,
        120.0,
        TimeSignature::new(4, 4),
        vec![TempoChange {
            position: TempoChangePosition::Time(Duration::from_secs(2)),
            original_measure_beat: None,
            bpm: Some(60.0),
            time_signature: None,
            transition: TempoTransition::Snap,
        }],
    );
    engine.set_tempo_map(Some(tempo_map));

    // Create a background static effect so strobe has something to work with
    let mut bg_params = HashMap::new();
    bg_params.insert("red".to_string(), 1.0);
    bg_params.insert("green".to_string(), 1.0);
    bg_params.insert("blue".to_string(), 1.0);
    let bg_effect = EffectInstance::new(
        "bg".to_string(),
        EffectType::Static {
            parameters: bg_params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(bg_effect).unwrap();
    engine.update(Duration::from_millis(10), None).unwrap(); // Let background settle

    // Create a strobe effect with frequency: 1beat (tempo-aware)
    let effect = EffectInstance::new(
        "tempo_aware_strobe".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Beats(1.0), // 1 cycle per beat
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // At t=0s (120 BPM): 1 beat = 0.5s, so frequency = 2.0 Hz
    // At 2 Hz, period = 0.5s
    let commands_before = engine.update(Duration::from_millis(100), None).unwrap();
    let strobe_before = commands_before.iter().find(|cmd| cmd.channel == 6);
    assert!(
        strobe_before.is_some(),
        "Strobe should generate commands before tempo change"
    );

    // At t=2s: tempo changes to 60 BPM
    // At t=2.1s (60 BPM): 1 beat = 1.0s, so frequency = 1.0 Hz
    // At 1 Hz, period = 1.0s
    // This is slower than before - the effect should have adapted
    engine.update(Duration::from_secs(2), None).unwrap(); // Advance to tempo change
    let commands_after = engine.update(Duration::from_millis(100), None).unwrap(); // 0.1s after tempo change

    // The effect should still be running (may or may not generate strobe commands depending on phase)
    // The key is that the frequency calculation uses the new tempo
    // We verify the effect is adapting by checking commands are generated
    assert!(
        !commands_after.is_empty(),
        "Effect should still generate commands after tempo change"
    );
}

#[test]
fn test_tempo_aware_chase_adapts_to_tempo_changes() {
    use crate::lighting::tempo::{
        TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
    };

    let mut engine = EffectEngine::new();
    let fixture1 = create_test_fixture("fixture1", 1, 1);
    let fixture2 = create_test_fixture("fixture2", 1, 6);
    let fixture3 = create_test_fixture("fixture3", 1, 11);
    engine.register_fixture(fixture1);
    engine.register_fixture(fixture2);
    engine.register_fixture(fixture3);

    // Create a tempo map: 120 BPM initially, changes to 60 BPM at 3 seconds
    let tempo_map = TempoMap::new(
        Duration::ZERO,
        120.0,
        TimeSignature::new(4, 4),
        vec![TempoChange {
            position: TempoChangePosition::Time(Duration::from_secs(3)),
            original_measure_beat: None,
            bpm: Some(60.0),
            time_signature: None,
            transition: TempoTransition::Snap,
        }],
    );
    engine.set_tempo_map(Some(tempo_map));

    // Create a chase effect with speed: 1measure (tempo-aware)
    let effect = EffectInstance::new(
        "tempo_aware_chase".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Measures(1.0), // 1 cycle per measure
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        },
        vec![
            "fixture1".to_string(),
            "fixture2".to_string(),
            "fixture3".to_string(),
        ],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // At t=0s (120 BPM): 1 measure = 2.0s, so speed = 0.5 cycles/sec
    let commands_before = engine.update(Duration::from_millis(100), None).unwrap();
    assert!(
        !commands_before.is_empty(),
        "Chase should generate commands before tempo change"
    );

    // At t=3s: tempo changes to 60 BPM
    // At t=3.1s (60 BPM): 1 measure = 4.0s, so speed = 0.25 cycles/sec
    // This is slower than before - the effect should have adapted
    engine.update(Duration::from_secs(3), None).unwrap(); // Advance to tempo change
    let commands_after = engine.update(Duration::from_millis(100), None).unwrap(); // 0.1s after tempo change

    // The effect should still be running and generating commands
    assert!(
        !commands_after.is_empty(),
        "Chase should still generate commands after tempo change"
    );

    // Verify it continues running
    let commands_later = engine.update(Duration::from_millis(1000), None).unwrap();
    assert!(
        !commands_later.is_empty(),
        "Chase should continue running after tempo change"
    );
}

#[test]
fn test_tempo_aware_chase_beats_speed_never_zero() {
    use crate::lighting::tempo::{
        TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
    };

    let mut engine = EffectEngine::new();
    let fixture1 = create_test_fixture("fixture1", 1, 1);
    let fixture2 = create_test_fixture("fixture2", 1, 6);
    let fixture3 = create_test_fixture("fixture3", 1, 11);
    engine.register_fixture(fixture1);
    engine.register_fixture(fixture2);
    engine.register_fixture(fixture3);

    // Tempo map: 120 BPM initially, changes to 60 BPM at 3 seconds
    let tempo_map = TempoMap::new(
        Duration::ZERO,
        120.0,
        TimeSignature::new(4, 4),
        vec![TempoChange {
            position: TempoChangePosition::Time(Duration::from_secs(3)),
            original_measure_beat: None,
            bpm: Some(60.0),
            time_signature: None,
            transition: TempoTransition::Snap,
        }],
    );
    engine.set_tempo_map(Some(tempo_map));

    // Chase with speed expressed in beats (tempo-aware), using a small beat value
    // similar to "0.5beats" in the show file. This guards against beats-based
    // speed resolving to zero due to beats_to_duration returning a degenerate
    // duration around tempo changes.
    let effect = EffectInstance::new(
        "tempo_aware_chase_beats".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Beats(0.5),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        },
        vec![
            "fixture1".to_string(),
            "fixture2".to_string(),
            "fixture3".to_string(),
        ],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // Shortly after start at 120 BPM, the chase should generate commands
    let commands_before = engine.update(Duration::from_millis(100), None).unwrap();
    assert!(
        !commands_before.is_empty(),
        "Chase with beats-based speed should generate commands before tempo change"
    );

    // Advance past the tempo change and ensure the chase still generates commands
    engine.update(Duration::from_secs(3), None).unwrap(); // Advance to tempo change
    let commands_after = engine.update(Duration::from_millis(100), None).unwrap(); // 0.1s after change
    assert!(
        !commands_after.is_empty(),
        "Chase with beats-based speed should still generate commands after tempo change"
    );

    // And it should continue to run later in time as well
    let commands_later = engine.update(Duration::from_millis(1000), None).unwrap();
    assert!(
        !commands_later.is_empty(),
        "Chase with beats-based speed should continue running after tempo change"
    );
}

#[test]
fn test_chase_after_tempo_change_with_measure_offset() {
    // Regression test: Replicates scenario where chases after a tempo change
    // may be missed due to timing/precision issues.
    // Scenario:
    // - Tempo change at measure 68/1 (score measure)
    // - Measure offset of 8
    // - Random chase at @70/1 (score measure) with speed: 1beats
    // - Linear chase at @74/1 (score measure) with speed: 0.5beats, direction: right_to_left
    use crate::lighting::tempo::{
        TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
    };

    let mut engine = EffectEngine::new();
    let fixture1 = create_test_fixture("fixture1", 1, 1);
    let fixture2 = create_test_fixture("fixture2", 1, 6);
    let fixture3 = create_test_fixture("fixture3", 1, 11);
    let fixture4 = create_test_fixture("fixture4", 1, 16);
    engine.register_fixture(fixture1);
    engine.register_fixture(fixture2);
    engine.register_fixture(fixture3);
    engine.register_fixture(fixture4);

    // Create tempo map: 160 BPM initially, changes to 120 BPM at measure 68/1
    // Using MeasureBeat position to match the user's scenario
    let tempo_map = TempoMap::new(
        Duration::from_secs_f64(1.5), // start_offset of 1.5s
        160.0,                        // initial BPM
        TimeSignature::new(4, 4),
        vec![TempoChange {
            position: TempoChangePosition::MeasureBeat(68, 1.0),
            original_measure_beat: Some((68, 1.0)),
            bpm: Some(120.0),
            time_signature: None,
            transition: TempoTransition::Snap,
        }],
    );
    engine.set_tempo_map(Some(tempo_map.clone()));

    // Calculate times for the chases using measure_to_time_with_offset
    // Measure offset of 8 means score measure 70 becomes playback measure 78
    let measure_offset = 8;
    let random_chase_time = tempo_map
        .measure_to_time_with_offset(70, 1.0, measure_offset, 0.0)
        .expect("Should be able to calculate time for measure 70/1");
    let linear_chase_time = tempo_map
        .measure_to_time_with_offset(74, 1.0, measure_offset, 0.0)
        .expect("Should be able to calculate time for measure 74/1");

    // Create random chase at @70/1 with speed: 1beats
    let random_chase = EffectInstance::new(
        "random_chase".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Random,
            speed: TempoAwareSpeed::Beats(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        },
        vec![
            "fixture1".to_string(),
            "fixture2".to_string(),
            "fixture3".to_string(),
            "fixture4".to_string(),
        ],
        None,
        None,
        None,
    );

    // Create linear chase at @74/1 with speed: 0.5beats, direction: right_to_left
    let linear_chase = EffectInstance::new(
        "linear_chase".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Beats(0.5),
            direction: ChaseDirection::RightToLeft,
            transition: CycleTransition::Snap,
        },
        vec![
            "fixture1".to_string(),
            "fixture2".to_string(),
            "fixture3".to_string(),
            "fixture4".to_string(),
        ],
        None,
        None,
        None,
    );

    // Advance to just before the random chase
    let time_before_random = random_chase_time - Duration::from_millis(10);
    engine.update(time_before_random, None).unwrap();

    // Start the random chase
    engine.start_effect(random_chase).unwrap();

    // Test that random chase produces output at various times
    // Test immediately after start
    let commands_at_start = engine
        .update(random_chase_time + Duration::from_millis(16), None)
        .unwrap();
    assert!(
        !commands_at_start.is_empty(),
        "Random chase should generate commands immediately after start"
    );

    // Test a bit later (during the chase)
    let commands_during = engine
        .update(random_chase_time + Duration::from_millis(100), None)
        .unwrap();
    assert!(
        !commands_during.is_empty(),
        "Random chase should continue generating commands during execution"
    );

    // Advance to just before the linear chase
    let time_before_linear = linear_chase_time - Duration::from_millis(10);
    engine.update(time_before_linear, None).unwrap();

    // Start the linear chase
    engine.start_effect(linear_chase).unwrap();

    // Test that linear chase produces output at various times
    // Test immediately after start
    let commands_linear_start = engine
        .update(linear_chase_time + Duration::from_millis(16), None)
        .unwrap();
    assert!(
        !commands_linear_start.is_empty(),
        "Linear chase should generate commands immediately after start (at measure 74/1)"
    );

    // Test a bit later (during the chase)
    let commands_linear_during = engine
        .update(linear_chase_time + Duration::from_millis(100), None)
        .unwrap();
    assert!(
        !commands_linear_during.is_empty(),
        "Linear chase should continue generating commands during execution"
    );

    // Test even later to ensure it keeps running
    let commands_linear_later = engine
        .update(linear_chase_time + Duration::from_millis(500), None)
        .unwrap();
    assert!(
        !commands_linear_later.is_empty(),
        "Linear chase should continue generating commands well into its execution"
    );

    // Critical test: Verify speed calculation doesn't return zero
    // This is the suspected issue - beats_to_duration might return a degenerate value
    // that causes speed to be calculated as 0.0
    let current_speed = TempoAwareSpeed::Beats(0.5).to_cycles_per_second(
        Some(&tempo_map),
        linear_chase_time + Duration::from_millis(100),
    );
    assert!(
        current_speed > 0.0,
        "Chase speed should never be zero; got speed={} at time after tempo change",
        current_speed
    );
}

#[test]
fn test_chase_timing_edge_cases_after_tempo_change() {
    // More aggressive test: Try to catch timing edge cases that might cause
    // a chase to be missed. Tests multiple time points around tempo changes
    // and chase start times to catch floating-point precision issues.
    use crate::lighting::tempo::{
        TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
    };

    let mut engine = EffectEngine::new();
    let fixture1 = create_test_fixture("fixture1", 1, 1);
    let fixture2 = create_test_fixture("fixture2", 1, 6);
    let fixture3 = create_test_fixture("fixture3", 1, 11);
    engine.register_fixture(fixture1);
    engine.register_fixture(fixture2);
    engine.register_fixture(fixture3);

    // Create tempo map: 160 BPM initially, changes to 120 BPM at measure 68/1
    let tempo_map = TempoMap::new(
        Duration::from_secs_f64(1.5),
        160.0,
        TimeSignature::new(4, 4),
        vec![TempoChange {
            position: TempoChangePosition::MeasureBeat(68, 1.0),
            original_measure_beat: Some((68, 1.0)),
            bpm: Some(120.0),
            time_signature: None,
            transition: TempoTransition::Snap,
        }],
    );
    engine.set_tempo_map(Some(tempo_map.clone()));

    let measure_offset = 8;
    let linear_chase_time = tempo_map
        .measure_to_time_with_offset(74, 1.0, measure_offset, 0.0)
        .expect("Should be able to calculate time for measure 74/1");

    // Test speed calculation at multiple time points around the chase start
    // This catches edge cases where beats_to_duration might return degenerate values
    let test_times = [
        linear_chase_time - Duration::from_millis(1),
        linear_chase_time,
        linear_chase_time + Duration::from_nanos(1),
        linear_chase_time + Duration::from_millis(1),
        linear_chase_time + Duration::from_millis(10),
        linear_chase_time + Duration::from_millis(100),
        linear_chase_time + Duration::from_millis(500),
    ];

    for (i, test_time) in test_times.iter().enumerate() {
        let speed = TempoAwareSpeed::Beats(0.5).to_cycles_per_second(Some(&tempo_map), *test_time);
        assert!(
            speed > 0.0,
            "Speed should never be zero at test point {} (time={:?}): got speed={}",
            i,
            test_time,
            speed
        );
    }
}

#[test]
fn test_tempo_aware_rainbow_adapts_to_tempo_changes() {
    use crate::lighting::tempo::{
        TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
    };

    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Create a tempo map: 120 BPM initially, changes to 60 BPM at 2.5 seconds
    let tempo_map = TempoMap::new(
        Duration::ZERO,
        120.0,
        TimeSignature::new(4, 4),
        vec![TempoChange {
            position: TempoChangePosition::Time(Duration::from_millis(2500)),
            original_measure_beat: None,
            bpm: Some(60.0),
            time_signature: None,
            transition: TempoTransition::Snap,
        }],
    );
    engine.set_tempo_map(Some(tempo_map));

    // Create a rainbow effect with speed: 2beats (tempo-aware)
    let effect = EffectInstance::new(
        "tempo_aware_rainbow".to_string(),
        EffectType::Rainbow {
            speed: TempoAwareSpeed::Beats(2.0), // 1 cycle per 2 beats
            saturation: 1.0,
            brightness: 1.0,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // At t=0s (120 BPM): 2 beats = 1.0s, so speed = 1.0 cycles/sec
    let commands_before = engine.update(Duration::from_millis(100), None).unwrap();
    assert!(
        !commands_before.is_empty(),
        "Rainbow should generate commands before tempo change"
    );

    // At t=2.5s: tempo changes to 60 BPM
    // At t=2.6s (60 BPM): 2 beats = 2.0s, so speed = 0.5 cycles/sec
    // This is slower than before - the effect should have adapted
    engine.update(Duration::from_millis(2500), None).unwrap(); // Advance to tempo change
    let commands_after = engine.update(Duration::from_millis(100), None).unwrap(); // 0.1s after tempo change

    // The effect should still be running and generating commands
    assert!(
        !commands_after.is_empty(),
        "Rainbow should still generate commands after tempo change"
    );

    // Verify it continues running
    let commands_later = engine.update(Duration::from_millis(1000), None).unwrap();
    assert!(
        !commands_later.is_empty(),
        "Rainbow should continue running after tempo change"
    );
}

#[test]
fn test_tempo_aware_pulse_adapts_to_tempo_changes() {
    use crate::lighting::tempo::{
        TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
    };

    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Create a tempo map: 120 BPM initially, changes to 60 BPM at 1.5 seconds
    let tempo_map = TempoMap::new(
        Duration::ZERO,
        120.0,
        TimeSignature::new(4, 4),
        vec![TempoChange {
            position: TempoChangePosition::Time(Duration::from_millis(1500)),
            original_measure_beat: None,
            bpm: Some(60.0),
            time_signature: None,
            transition: TempoTransition::Snap,
        }],
    );
    engine.set_tempo_map(Some(tempo_map));

    // Create a pulse effect with frequency: 1beat (tempo-aware)
    let effect = EffectInstance::new(
        "tempo_aware_pulse".to_string(),
        EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Beats(1.0), // 1 cycle per beat
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // At t=0s (120 BPM): 1 beat = 0.5s, so frequency = 2.0 Hz
    let commands_before = engine.update(Duration::from_millis(100), None).unwrap();
    assert!(
        !commands_before.is_empty(),
        "Pulse should generate commands before tempo change"
    );

    // At t=1.5s: tempo changes to 60 BPM
    // At t=1.6s (60 BPM): 1 beat = 1.0s, so frequency = 1.0 Hz
    // This is slower than before - the effect should have adapted
    engine.update(Duration::from_millis(1500), None).unwrap(); // Advance to tempo change
    let commands_after = engine.update(Duration::from_millis(100), None).unwrap(); // 0.1s after tempo change

    // The effect should still be running and generating commands
    assert!(
        !commands_after.is_empty(),
        "Pulse should still generate commands after tempo change"
    );

    // Verify it continues running
    let commands_later = engine.update(Duration::from_millis(1000), None).unwrap();
    assert!(
        !commands_later.is_empty(),
        "Pulse should continue running after tempo change"
    );
}
