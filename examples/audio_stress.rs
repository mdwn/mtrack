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

//! Audio stress test for real hardware.
//!
//! Exercises the full audio callback pipeline against a real audio device
//! under various configurations to detect dropouts, jitter, and scheduling
//! issues. Captures `CallbackProfiler` metrics via a custom tracing layer
//! and reports pass/fail for each scenario.
//!
//! Usage:
//!   cargo run --example audio_stress -- --list
//!   cargo run --example audio_stress -- -d "hw:CARD=..." -t 30
//!   cargo run --example audio_stress -- -d "hw:CARD=..." -t 10 -n 4 --buffer-sizes 256

extern crate mtrack;

use clap::Parser;
use mtrack::audio;
use mtrack::audio::mixer::ActiveSource;
use mtrack::audio::sample_source::{ChannelMappedSource, LoopingSampleSource};
use mtrack::config;
use mtrack::playsync::CancelHandle;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::error::Error;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::Subscriber;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "audio_stress")]
#[command(about = "Stress test audio hardware for dropout and jitter detection")]
struct Args {
    /// Audio device name. Use --list to see available devices.
    #[arg(short, long)]
    device: Option<String>,

    /// List available audio devices and exit.
    #[arg(short, long)]
    list: bool,

    /// Duration of each test scenario in seconds.
    #[arg(short = 't', long = "duration", default_value = "30")]
    duration_secs: u64,

    /// Number of simultaneous sources to mix.
    #[arg(short = 'n', long, default_value = "8")]
    num_sources: usize,

    /// Maximum number of simultaneous sources for the ramp scenario.
    #[arg(long, default_value = "32")]
    max_sources: usize,

    /// Buffer sizes to test (comma-separated).
    #[arg(long, default_value = "64,128,256,512,1024", value_delimiter = ',')]
    buffer_sizes: Vec<usize>,

    /// Sample rates to test (comma-separated).
    #[arg(long, default_value = "44100,48000", value_delimiter = ',')]
    sample_rates: Vec<u32>,

    /// Sample formats to test (comma-separated: "float", "int").
    #[arg(long, default_value = "float,int", value_delimiter = ',')]
    formats: Vec<String>,

    /// Maximum acceptable callback gap in microseconds (overrides computed threshold).
    #[arg(long)]
    max_gap_us: Option<u64>,

    /// Maximum acceptable mix time in microseconds (overrides computed threshold).
    #[arg(long)]
    max_mix_us: Option<u64>,

    /// Print profiler snapshots during the run.
    #[arg(short, long)]
    verbose: bool,
}

// ---------------------------------------------------------------------------
// Profiler metric capture via tracing layer
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct ProfileSnapshot {
    mix_avg_us: u64,
    mix_max_us: u64,
    cb_avg_gap_us: u64,
    cb_max_gap_us: u64,
    callbacks: u64,
}

/// Collects CallbackProfiler tracing events into structured snapshots.
struct MetricsCollector {
    snapshots: Arc<Mutex<Vec<ProfileSnapshot>>>,
    verbose: bool,
}

/// Extracts named u64 fields from a tracing event.
#[derive(Default)]
struct ProfileFieldVisitor {
    mix_avg_us: Option<u64>,
    mix_max_us: Option<u64>,
    cb_avg_gap_us: Option<u64>,
    cb_max_gap_us: Option<u64>,
    callbacks: Option<u64>,
}

impl tracing::field::Visit for ProfileFieldVisitor {
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "mix_avg_us" => self.mix_avg_us = Some(value),
            "mix_max_us" => self.mix_max_us = Some(value),
            "cb_avg_gap_us" => self.cb_avg_gap_us = Some(value),
            "cb_max_gap_us" => self.cb_max_gap_us = Some(value),
            "callbacks" => self.callbacks = Some(value),
            _ => {}
        }
    }

    fn record_debug(&mut self, _field: &tracing::field::Field, _value: &dyn std::fmt::Debug) {}
}

impl ProfileFieldVisitor {
    fn into_snapshot(self) -> Option<ProfileSnapshot> {
        Some(ProfileSnapshot {
            mix_avg_us: self.mix_avg_us?,
            mix_max_us: self.mix_max_us?,
            cb_avg_gap_us: self.cb_avg_gap_us?,
            cb_max_gap_us: self.cb_max_gap_us?,
            callbacks: self.callbacks?,
        })
    }
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for MetricsCollector {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let metadata = event.metadata();
        // CallbackProfiler logs with target "mtrack::audio::cpal" and message containing "audio profile"
        if !metadata.target().contains("audio::cpal") {
            return;
        }
        let mut visitor = ProfileFieldVisitor::default();
        event.record(&mut visitor);
        if let Some(snapshot) = visitor.into_snapshot() {
            if self.verbose {
                eprintln!(
                    "  [profile] mix: avg={}us max={}us  gap: avg={}us max={}us  callbacks={}",
                    snapshot.mix_avg_us,
                    snapshot.mix_max_us,
                    snapshot.cb_avg_gap_us,
                    snapshot.cb_max_gap_us,
                    snapshot.callbacks,
                );
            }
            self.snapshots.lock().push(snapshot);
        }
    }
}

// ---------------------------------------------------------------------------
// Sine wave generation
// ---------------------------------------------------------------------------

/// Generates a 1-second stereo sine wave buffer at the given frequency.
fn generate_sine_buffer(frequency: f32, sample_rate: u32, channels: u16) -> Arc<Vec<f32>> {
    let num_frames = sample_rate as usize;
    let mut samples = Vec::with_capacity(num_frames * channels as usize);
    for i in 0..num_frames {
        let t = i as f32 / sample_rate as f32;
        let value = (2.0 * std::f32::consts::PI * frequency * t).sin();
        for _ in 0..channels {
            samples.push(value);
        }
    }
    Arc::new(samples)
}

// ---------------------------------------------------------------------------
// Source creation helpers
// ---------------------------------------------------------------------------

/// Creates an ActiveSource with a looping sine wave and sends it to the device.
fn create_looping_source(
    sine_buffer: &Arc<Vec<f32>>,
    channels: u16,
    sample_rate: u32,
    volume: f32,
) -> (ActiveSource, Arc<AtomicBool>, CancelHandle) {
    let source_id = audio::next_source_id();

    let mem_source =
        LoopingSampleSource::from_shared(sine_buffer.clone(), channels, sample_rate, volume);

    let channel_labels: Vec<Vec<String>> = (0..channels)
        .map(|ch| vec![format!("stress_ch_{}", ch)])
        .collect();

    let channel_mapped = ChannelMappedSource::new(Box::new(mem_source), channel_labels, channels);

    let track_mappings: HashMap<String, Vec<u16>> = (0..channels)
        .map(|ch| (format!("stress_ch_{}", ch), vec![ch + 1]))
        .collect();

    let is_finished = Arc::new(AtomicBool::new(false));
    let cancel_handle = CancelHandle::new();

    let active_source = ActiveSource {
        id: source_id,
        source: Box::new(channel_mapped),
        track_mappings,
        channel_mappings: Vec::new(), // Precomputed by mixer.add_source
        cached_source_channel_count: channels,
        is_finished: is_finished.clone(),
        cancel_handle: cancel_handle.clone(),
        start_at_sample: None,
        cancel_at_sample: None,
        gain_envelope: None,
    };

    (active_source, is_finished, cancel_handle)
}

// ---------------------------------------------------------------------------
// Scenario results and reporting
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Verdict {
    Pass,
    Warn,
    Fail,
}

struct ScenarioResult {
    config_desc: String,
    num_sources: usize,
    budget_us: f64,
    mix_avg_us: u64,
    mix_max_us: u64,
    cb_avg_gap_us: u64,
    cb_max_gap_us: u64,
    total_callbacks: u64,
    verdict: Verdict,
    failures: Vec<String>,
}

fn compute_budget_us(buffer_size: usize, sample_rate: u32) -> f64 {
    (buffer_size as f64 / sample_rate as f64) * 1_000_000.0
}

fn evaluate_snapshots(
    snapshots: &[ProfileSnapshot],
    budget_us: f64,
    max_gap_override: Option<u64>,
    max_mix_override: Option<u64>,
) -> (u64, u64, u64, u64, u64, Verdict, Vec<String>) {
    if snapshots.is_empty() {
        return (
            0,
            0,
            0,
            0,
            0,
            Verdict::Fail,
            vec!["No profiler data collected (is MTRACK_PROFILE_AUDIO=1 set?)".to_string()],
        );
    }

    let total_callbacks: u64 = snapshots.iter().map(|s| s.callbacks).sum();
    let mix_max_us = snapshots.iter().map(|s| s.mix_max_us).max().unwrap_or(0);
    let cb_max_gap_us = snapshots.iter().map(|s| s.cb_max_gap_us).max().unwrap_or(0);

    // Weighted average by callback count
    let (weighted_mix_sum, weighted_gap_sum, weight_sum) =
        snapshots
            .iter()
            .fold((0u128, 0u128, 0u64), |(ms, gs, ws), s| {
                (
                    ms + s.mix_avg_us as u128 * s.callbacks as u128,
                    gs + s.cb_avg_gap_us as u128 * s.callbacks as u128,
                    ws + s.callbacks,
                )
            });
    let mix_avg_us = if weight_sum > 0 {
        (weighted_mix_sum / weight_sum as u128) as u64
    } else {
        0
    };
    let cb_avg_gap_us = if weight_sum > 0 {
        (weighted_gap_sum / weight_sum as u128) as u64
    } else {
        0
    };

    let mut failures = Vec::new();
    let mut verdict = Verdict::Pass;

    // Check mix time against budget
    let mix_threshold = max_mix_override.unwrap_or((budget_us * 0.8) as u64);
    let mix_warn_threshold = (budget_us * 0.5) as u64;

    if mix_max_us > mix_threshold {
        failures.push(format!(
            "mix_max ({}us) > threshold ({}us)",
            mix_max_us, mix_threshold
        ));
        verdict = Verdict::Fail;
    } else if max_mix_override.is_none() && mix_max_us > mix_warn_threshold {
        verdict = Verdict::Warn;
    }

    // Check callback gap
    let gap_threshold = max_gap_override.unwrap_or((budget_us * 2.0) as u64);

    if cb_max_gap_us > gap_threshold {
        failures.push(format!(
            "cb_max_gap ({}us) > threshold ({}us)",
            cb_max_gap_us, gap_threshold
        ));
        verdict = Verdict::Fail;
    }

    (
        mix_avg_us,
        mix_max_us,
        cb_avg_gap_us,
        cb_max_gap_us,
        total_callbacks,
        verdict,
        failures,
    )
}

fn print_result(result: &ScenarioResult) {
    let tag = match result.verdict {
        Verdict::Pass => "[PASS]",
        Verdict::Warn => "[WARN]",
        Verdict::Fail => "[FAIL]",
    };
    let headroom = if result.budget_us > 0.0 {
        (1.0 - result.mix_max_us as f64 / result.budget_us) * 100.0
    } else {
        0.0
    };

    println!(
        "{} {} sources={}",
        tag, result.config_desc, result.num_sources,
    );
    println!(
        "       mix: avg={}us max={}us  gap: avg={}us max={}us  headroom: {:.1}%  callbacks: {}",
        result.mix_avg_us,
        result.mix_max_us,
        result.cb_avg_gap_us,
        result.cb_max_gap_us,
        headroom,
        result.total_callbacks,
    );
    for failure in &result.failures {
        println!("       FAILURE: {}", failure);
    }
}

// ---------------------------------------------------------------------------
// Scenario runners
// ---------------------------------------------------------------------------

/// Shared context for all scenario runners.
struct ScenarioContext<'a> {
    device_name: &'a str,
    snapshots: &'a Arc<Mutex<Vec<ProfileSnapshot>>>,
    max_gap_override: Option<u64>,
    max_mix_override: Option<u64>,
}

/// Runs a single scenario: creates a device, adds sources, waits, collects metrics.
fn run_scenario(
    ctx: &ScenarioContext<'_>,
    buffer_size: usize,
    sample_rate: u32,
    format: &str,
    bits: u16,
    num_sources: usize,
    duration: Duration,
) -> Result<ScenarioResult, Box<dyn Error>> {
    let config_desc = format!(
        "buffer={} rate={} fmt={}/{}",
        buffer_size, sample_rate, format, bits
    );

    println!(
        "  [RUN] {} sources={} duration={}s ...",
        config_desc,
        num_sources,
        duration.as_secs()
    );

    // Clear previous snapshots
    ctx.snapshots.lock().clear();

    // Build audio config
    let audio_config = config::Audio::new(ctx.device_name)
        .with_sample_rate(sample_rate)
        .with_buffer_size(buffer_size)
        .with_sample_format(format)
        .with_bits_per_sample(bits)
        .with_stream_buffer_size(config::StreamBufferSize::Fixed(buffer_size));

    // Create device
    let device = audio::get_device(Some(audio_config))?;

    let source_tx = device
        .source_sender()
        .ok_or("Device does not support source_sender")?;

    // Use 2 channels (stereo)
    let channels = 2u16;
    let volume = 0.1 / num_sources.max(1) as f32; // Scale down with source count

    // Create and add sources with different frequencies
    let mut cancel_handles = Vec::new();
    for i in 0..num_sources {
        let freq = 220.0 * (i as f32 + 1.0); // 220, 440, 660, ...
        let sine_buffer = generate_sine_buffer(freq, sample_rate, channels);
        let (active_source, _is_finished, cancel_handle) =
            create_looping_source(&sine_buffer, channels, sample_rate, volume);
        source_tx.send(active_source)?;
        cancel_handles.push(cancel_handle);
    }

    // Let it run
    std::thread::sleep(duration);

    // Cancel all sources
    for handle in &cancel_handles {
        handle.cancel();
    }

    // Brief pause for cleanup
    std::thread::sleep(Duration::from_millis(100));

    // Evaluate collected metrics
    let collected = ctx.snapshots.lock().clone();
    let budget_us = compute_budget_us(buffer_size, sample_rate);

    let (mix_avg, mix_max, gap_avg, gap_max, total_cb, verdict, failures) = evaluate_snapshots(
        &collected,
        budget_us,
        ctx.max_gap_override,
        ctx.max_mix_override,
    );

    // Drop the device (joins output thread)
    drop(device);
    std::thread::sleep(Duration::from_millis(500));

    Ok(ScenarioResult {
        config_desc,
        num_sources,
        budget_us,
        mix_avg_us: mix_avg,
        mix_max_us: mix_max,
        cb_avg_gap_us: gap_avg,
        cb_max_gap_us: gap_max,
        total_callbacks: total_cb,
        verdict,
        failures,
    })
}

/// Runs the source count ramp scenario: progressively adds sources until failure.
fn run_ramp_scenario(
    ctx: &ScenarioContext<'_>,
    buffer_size: usize,
    sample_rate: u32,
    max_sources: usize,
    step_duration: Duration,
) -> Result<(), Box<dyn Error>> {
    println!("\n--- Source Count Ramp ---");
    let config_desc = format!("buffer={} rate={} fmt=float/32", buffer_size, sample_rate);
    println!(
        "[INFO] {} (adding sources every {}s)",
        config_desc,
        step_duration.as_secs()
    );

    ctx.snapshots.lock().clear();

    let audio_config = config::Audio::new(ctx.device_name)
        .with_sample_rate(sample_rate)
        .with_buffer_size(buffer_size)
        .with_sample_format("float")
        .with_bits_per_sample(32)
        .with_stream_buffer_size(config::StreamBufferSize::Fixed(buffer_size));

    let device = audio::get_device(Some(audio_config))?;
    let source_tx = device
        .source_sender()
        .ok_or("Device does not support source_sender")?;

    let channels = 2u16;
    let budget_us = compute_budget_us(buffer_size, sample_rate);
    let mut cancel_handles = Vec::new();
    let mut max_clean_sources = 0usize;
    let sources_per_step = 4;

    let mut current_sources = 0usize;
    while current_sources < max_sources {
        let to_add = sources_per_step.min(max_sources - current_sources);
        for i in 0..to_add {
            let freq = 220.0 * ((current_sources + i) as f32 + 1.0);
            let volume = 0.1 / max_sources.max(1) as f32;
            let sine_buffer = generate_sine_buffer(freq, sample_rate, channels);
            let (active_source, _is_finished, cancel_handle) =
                create_looping_source(&sine_buffer, channels, sample_rate, volume);
            source_tx.send(active_source)?;
            cancel_handles.push(cancel_handle);
        }
        current_sources += to_add;

        // Clear snapshots for this step only
        ctx.snapshots.lock().clear();
        std::thread::sleep(step_duration);

        // Evaluate this step
        let collected = ctx.snapshots.lock().clone();
        let (mix_avg, mix_max, _gap_avg, gap_max, _total_cb, verdict, _failures) =
            evaluate_snapshots(
                &collected,
                budget_us,
                ctx.max_gap_override,
                ctx.max_mix_override,
            );

        let tag = match verdict {
            Verdict::Pass => "OK",
            Verdict::Warn => "WARN",
            Verdict::Fail => "FAIL",
        };
        println!(
            "       {:>3} sources: mix avg={}us max={}us  gap max={}us  [{}]",
            current_sources, mix_avg, mix_max, gap_max, tag,
        );

        if verdict == Verdict::Pass {
            max_clean_sources = current_sources;
        }
        if verdict == Verdict::Fail {
            break;
        }
    }

    println!(
        "[INFO] Maximum clean sources: {} ({})",
        max_clean_sources, config_desc,
    );

    // Cleanup
    for handle in &cancel_handles {
        handle.cancel();
    }
    std::thread::sleep(Duration::from_millis(100));
    drop(device);
    std::thread::sleep(Duration::from_millis(500));

    Ok(())
}

/// Runs the churn scenario: rapidly adds and removes short-lived sources.
fn run_churn_scenario(
    ctx: &ScenarioContext<'_>,
    buffer_size: usize,
    sample_rate: u32,
    duration: Duration,
) -> Result<ScenarioResult, Box<dyn Error>> {
    println!(
        "  [RUN] buffer={} rate={} fmt=float/32 duration={}s ...",
        buffer_size,
        sample_rate,
        duration.as_secs()
    );

    ctx.snapshots.lock().clear();

    let audio_config = config::Audio::new(ctx.device_name)
        .with_sample_rate(sample_rate)
        .with_buffer_size(buffer_size)
        .with_sample_format("float")
        .with_bits_per_sample(32)
        .with_stream_buffer_size(config::StreamBufferSize::Fixed(buffer_size));

    let device = audio::get_device(Some(audio_config))?;
    let source_tx = device
        .source_sender()
        .ok_or("Device does not support source_sender")?;

    let channels = 2u16;
    let budget_us = compute_budget_us(buffer_size, sample_rate);
    let mut cancel_handles: Vec<(CancelHandle, Instant)> = Vec::new();
    let source_lifetime = Duration::from_secs(2);
    let add_interval = Duration::from_millis(100);
    let start = Instant::now();
    let mut sources_created = 0u64;
    let mut next_add = Instant::now();

    while start.elapsed() < duration {
        let now = Instant::now();

        // Add a new source on schedule
        if now >= next_add {
            let freq = 220.0 + (sources_created % 10) as f32 * 110.0;
            let sine_buffer = generate_sine_buffer(freq, sample_rate, channels);
            let (active_source, _is_finished, cancel_handle) =
                create_looping_source(&sine_buffer, channels, sample_rate, 0.05);
            if source_tx.send(active_source).is_ok() {
                cancel_handles.push((cancel_handle, now));
                sources_created += 1;
            }
            next_add = now + add_interval;
        }

        // Cancel sources that have exceeded their lifetime
        cancel_handles.retain(|(handle, created)| {
            if created.elapsed() >= source_lifetime {
                handle.cancel();
                false
            } else {
                true
            }
        });

        std::thread::sleep(Duration::from_millis(10));
    }

    // Cancel remaining
    for (handle, _) in &cancel_handles {
        handle.cancel();
    }
    std::thread::sleep(Duration::from_millis(100));

    let collected = ctx.snapshots.lock().clone();
    let (mix_avg, mix_max, gap_avg, gap_max, total_cb, verdict, failures) = evaluate_snapshots(
        &collected,
        budget_us,
        ctx.max_gap_override,
        ctx.max_mix_override,
    );

    let config_desc = format!(
        "buffer={} rate={} fmt=float/32 churn({}ms add, {}s life, {} created)",
        buffer_size,
        sample_rate,
        add_interval.as_millis(),
        source_lifetime.as_secs(),
        sources_created,
    );

    drop(device);
    std::thread::sleep(Duration::from_millis(500));

    Ok(ScenarioResult {
        config_desc,
        num_sources: sources_created as usize,
        budget_us,
        mix_avg_us: mix_avg,
        mix_max_us: mix_max,
        cb_avg_gap_us: gap_avg,
        cb_max_gap_us: gap_max,
        total_callbacks: total_cb,
        verdict,
        failures,
    })
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    // Enable profiler unconditionally
    unsafe {
        std::env::set_var("MTRACK_PROFILE_AUDIO", "1");
    }

    // Set up tracing with our metrics collector layer
    let snapshots: Arc<Mutex<Vec<ProfileSnapshot>>> = Arc::new(Mutex::new(Vec::new()));
    let collector = MetricsCollector {
        snapshots: snapshots.clone(),
        verbose: args.verbose,
    };

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("off,mtrack=info"));

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter))
        .with(collector)
        .init();

    // Handle --list
    if args.list {
        let devices = audio::list_devices()?;
        if devices.is_empty() {
            println!("No audio devices found.");
        } else {
            println!("Available audio devices:");
            for device in devices {
                println!("  - {}", device);
            }
        }
        return Ok(());
    }

    let device_name = args
        .device
        .as_deref()
        .ok_or("No device specified. Use --device <name> or --list to see available devices.")?;

    let duration = Duration::from_secs(args.duration_secs);

    // Estimate total runtime.
    let sweep_scenarios = args.buffer_sizes.len() + args.sample_rates.len() + args.formats.len();
    let ramp_steps = args.max_sources.div_ceil(4); // sources_per_step = 4
    let estimated_secs = (sweep_scenarios as u64) * args.duration_secs
        + (ramp_steps as u64) * 5
        + args.duration_secs;

    println!("=== Audio Stress Test ===");
    println!("Device: {}", device_name);
    println!(
        "Duration per scenario: {}s, Sources: {}, Scenarios: {} + ramp + churn",
        args.duration_secs, args.num_sources, sweep_scenarios,
    );
    println!(
        "Estimated total runtime: ~{}m{}s",
        estimated_secs / 60,
        estimated_secs % 60
    );
    println!();

    let ctx = ScenarioContext {
        device_name,
        snapshots: &snapshots,
        max_gap_override: args.max_gap_us,
        max_mix_override: args.max_mix_us,
    };

    let mut all_results: Vec<ScenarioResult> = Vec::new();

    // Use the lowest sample rate and first buffer size as baselines for non-sweep scenarios.
    // Lowest rate is deterministic regardless of argument order and represents the most
    // standard/common rate. First buffer size lets the user control the baseline via ordering.
    let base_rate = *args.sample_rates.iter().min().unwrap_or(&44100);
    let base_buffer = *args.buffer_sizes.iter().min().unwrap_or(&256);

    // --- Scenario 1: Buffer Size Sweep ---
    println!("--- Buffer Size Sweep ---");
    for &buf_size in &args.buffer_sizes {
        match run_scenario(
            &ctx,
            buf_size,
            base_rate,
            "float",
            32,
            args.num_sources,
            duration,
        ) {
            Ok(result) => {
                print_result(&result);
                all_results.push(result);
            }
            Err(e) => {
                eprintln!(
                    "[ERROR] buffer={} rate={} fmt=float/32: {}",
                    buf_size, base_rate, e
                );
            }
        }
    }

    // --- Scenario 2: Sample Rate Sweep ---
    println!("\n--- Sample Rate Sweep ---");
    for &rate in &args.sample_rates {
        match run_scenario(
            &ctx,
            base_buffer,
            rate,
            "float",
            32,
            args.num_sources,
            duration,
        ) {
            Ok(result) => {
                print_result(&result);
                all_results.push(result);
            }
            Err(e) => {
                eprintln!(
                    "[ERROR] buffer={} rate={} fmt=float/32: {}",
                    base_buffer, rate, e
                );
            }
        }
    }

    // --- Scenario 3: Format Sweep ---
    println!("\n--- Format Sweep ---");
    for fmt in &args.formats {
        let (fmt_str, bits) = match fmt.as_str() {
            "float" => ("float", 32u16),
            "int" => ("int", 32u16),
            "int16" => ("int", 16u16),
            other => {
                eprintln!("[SKIP] Unknown format: {}", other);
                continue;
            }
        };
        match run_scenario(
            &ctx,
            base_buffer,
            base_rate,
            fmt_str,
            bits,
            args.num_sources,
            duration,
        ) {
            Ok(result) => {
                print_result(&result);
                all_results.push(result);
            }
            Err(e) => {
                eprintln!(
                    "[ERROR] buffer={} rate={} fmt={}/{}: {}",
                    base_buffer, base_rate, fmt_str, bits, e
                );
            }
        }
    }

    // --- Scenario 4: Source Count Ramp ---
    if let Err(e) = run_ramp_scenario(
        &ctx,
        base_buffer,
        base_rate,
        args.max_sources,
        Duration::from_secs(5),
    ) {
        eprintln!("[ERROR] Source count ramp: {}", e);
    }

    // --- Scenario 5: Churn ---
    println!("\n--- Churn ---");
    match run_churn_scenario(&ctx, base_buffer, base_rate, duration) {
        Ok(result) => {
            print_result(&result);
            all_results.push(result);
        }
        Err(e) => {
            eprintln!("[ERROR] Churn scenario: {}", e);
        }
    }

    // --- Summary ---
    let passed = all_results
        .iter()
        .filter(|r| r.verdict == Verdict::Pass)
        .count();
    let warned = all_results
        .iter()
        .filter(|r| r.verdict == Verdict::Warn)
        .count();
    let failed = all_results
        .iter()
        .filter(|r| r.verdict == Verdict::Fail)
        .count();
    let total = all_results.len();

    println!("\n=== Summary: {}/{} passed", passed, total);
    if warned > 0 {
        println!("    {} warned", warned);
    }
    if failed > 0 {
        println!("    {} FAILED", failed);
    }
    println!("===");

    if failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}
