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

use clap::Parser;
// Examples can access the crate as a library
extern crate mtrack;
use mtrack::lighting::{
    effects::{EffectInstance, FixtureInfo},
    engine::EffectEngine,
    parser::{parse_light_shows, LightShow},
    timeline::LightingTimeline,
};
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "lighting_simulator")]
#[command(about = "Simulates lighting effects from a DSL file for debugging")]
struct Args {
    /// Path to the lighting DSL file
    #[arg(short, long)]
    file: PathBuf,

    /// Time step in milliseconds (default: 100ms)
    #[arg(long, default_value = "100")]
    step_ms: u64,

    /// Start time in seconds (default: 0.0)
    #[arg(short = 't', long, default_value = "0.0")]
    start_time: f64,

    /// End time in seconds (default: run until timeline finishes)
    #[arg(long)]
    end_time: Option<f64>,

    /// Show fixture states at each step
    #[arg(long)]
    show_states: bool,

    /// Only show effect start/stop events
    #[arg(long)]
    events_only: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    // Load and parse the DSL file
    let content = fs::read_to_string(&args.file)?;
    let shows: std::collections::HashMap<String, LightShow> = parse_light_shows(&content)?;

    if shows.is_empty() {
        return Err("No light shows found in file".into());
    }

    // Create a simple fixture registry for simulation
    let mut fixture_registry = HashMap::new();
    let fixture_names = collect_fixture_names(&shows);
    let mut address = 1u16;
    for name in fixture_names {
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), address);
        channels.insert("green".to_string(), address + 1);
        channels.insert("blue".to_string(), address + 2);
        channels.insert("white".to_string(), address + 3);
        channels.insert("dimmer".to_string(), address + 4);
        address += 5;

        fixture_registry.insert(
            name.clone(),
            FixtureInfo {
                name: name.clone(),
                universe: 1,
                address: address - 5,
                fixture_type: "RGBW".to_string(),
                channels,
                max_strobe_frequency: None,
            },
        );
    }

    // Create timeline and effect engine
    // parse_light_shows returns HashMap<String, LightShow>, convert to Vec
    let shows_vec: Vec<LightShow> = shows.values().cloned().collect();
    let mut timeline = LightingTimeline::new(shows_vec);
    let mut effect_engine = EffectEngine::new();

    // Set tempo map if available
    if let Some(tempo_map) = timeline.tempo_map() {
        effect_engine.set_tempo_map(Some(tempo_map.clone()));
    }

    // Register fixtures
    for fixture_info in fixture_registry.values() {
        effect_engine.register_fixture(fixture_info.clone());
    }

    // Start timeline
    let start_time = Duration::from_secs_f64(args.start_time);
    let mut timeline_update = timeline.start_at(start_time);

    // Start effects that should be active at start_time
    for (effect, elapsed_time) in timeline_update.effects_with_elapsed.values() {
        if let Err(e) = effect_engine.start_effect_with_elapsed(effect.clone(), *elapsed_time) {
            eprintln!("Error starting effect: {}", e);
        }
    }

    // Simulate time progression
    let step = Duration::from_millis(args.step_ms);
    let end_time = args
        .end_time
        .map(|t| Duration::from_secs_f64(t))
        .unwrap_or(Duration::from_secs_f64(1000.0)); // Default to 1000 seconds

    let mut current_time = start_time;
    let mut last_active_effects: HashMap<String, EffectInstance> = HashMap::new();

    println!("=== Lighting Simulator ===");
    println!("File: {}", args.file.display());
    println!("Time step: {}ms", args.step_ms);
    println!("Start time: {:.3}s", start_time.as_secs_f64());
    println!("End time: {:.3}s", end_time.as_secs_f64());
    println!();

    while current_time <= end_time && !timeline.is_finished() {
        // Update timeline
        timeline_update = timeline.update(current_time);

        // Start new effects
        for effect in &timeline_update.effects {
            if !args.events_only {
                println!(
                    "[{:.3}s] Starting effect: {} (cue_time={:.3}s, hold_time={:?})",
                    current_time.as_secs_f64(),
                    effect.id,
                    effect.cue_time.map(|t| t.as_secs_f64()).unwrap_or(0.0),
                    effect.hold_time
                );
            } else {
                println!(
                    "[{:.3}s] START: {} (cue_time={:.3}s)",
                    current_time.as_secs_f64(),
                    effect.id,
                    effect.cue_time.map(|t| t.as_secs_f64()).unwrap_or(0.0)
                );
            }

            if let Err(e) = effect_engine.start_effect(effect.clone()) {
                eprintln!("Error starting effect: {}", e);
            }
        }

        // Update effect engine
        let dt = step;
        if let Err(e) = effect_engine.update(dt, Some(current_time)) {
            eprintln!("Error updating effects: {}", e);
        }

        // Check for effects that stopped
        let current_active: HashMap<String, _> = effect_engine
            .get_active_effects()
            .iter()
            .map(|(id, eff)| (id.clone(), eff.clone()))
            .collect();

        for (id, effect) in &last_active_effects {
            if !current_active.contains_key(id) {
                if !args.events_only {
                    let run_duration = if let Some(cue_time) = effect.cue_time {
                        // Use score-time elapsed if available
                        current_time.saturating_sub(cue_time)
                    } else {
                        Duration::ZERO
                    };
                    println!(
                        "[{:.3}s] Stopped effect: {} (was running for {:.3}s)",
                        current_time.as_secs_f64(),
                        id,
                        run_duration.as_secs_f64()
                    );
                } else {
                    println!("[{:.3}s] STOP: {}", current_time.as_secs_f64(), id);
                }
            }
        }

        // Show active effects summary if requested
        if args.show_states && !args.events_only && !current_active.is_empty() {
            println!("  Active effects: {}", current_active.len());
            for (id, effect) in &current_active {
                let elapsed = if let Some(cue_time) = effect.cue_time {
                    // Use score-time elapsed if available
                    current_time.saturating_sub(cue_time).as_secs_f64()
                } else {
                    // No cue_time available, can't calculate elapsed
                    0.0
                };
                let duration_str = if let Some(total) = effect.total_duration() {
                    format!(" / {:.2}s", total.as_secs_f64())
                } else {
                    " (perpetual)".to_string()
                };
                println!(
                    "    {}: elapsed {:.2}s{}, hold_time={:?}",
                    id, elapsed, duration_str, effect.hold_time
                );
            }
        }

        // Update last_active_effects after we're done using current_active
        last_active_effects = current_active;

        // Advance time
        current_time += step;
    }

    println!();
    println!("=== Simulation Complete ===");
    println!("Final time: {:.3}s", current_time.as_secs_f64());
    println!("Timeline finished: {}", timeline.is_finished());

    Ok(())
}

fn collect_fixture_names(shows: &std::collections::HashMap<String, LightShow>) -> Vec<String> {
    let mut names = std::collections::HashSet::new();
    for show in shows.values() {
        for cue in &show.cues {
            for effect in &cue.effects {
                names.extend(effect.groups.iter().cloned());
            }
        }
    }
    names.into_iter().collect()
}
