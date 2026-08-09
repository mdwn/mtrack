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
//! Runs the checks and assembles the report.
//!
//! Checks run one at a time. The hardware is a singleton -- two checks holding
//! the same interface would produce failures that look like defects -- so this
//! is a property of the subject, not a limitation of the runner.

use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use crate::capabilities::Capabilities;
use crate::checks;
use crate::discovery::Discovery;
use crate::outcome::{CheckError, CheckOutcome, CheckResult};
use crate::report::Report;

/// Runs every check whose name or area matches `filter`.
pub async fn run_once(filter: &Option<String>) -> Report {
    // Each run is independent; without this, one readiness timeout in run 1
    // would block every server-backed check for the rest of a --repeat sweep.
    crate::server::reset_init_latch();

    let mut results = Vec::new();
    for check in checks::all() {
        if let Some(needle) = filter {
            if !check.name.contains(needle.as_str()) && !check.area.contains(needle.as_str()) {
                continue;
            }
        }

        eprint!("  running {:<44}\r", check.name);
        let started = Instant::now();
        let outcome = execute(&check).await;
        let result = CheckResult::from_outcome(
            check.area,
            check.name,
            check.description,
            outcome,
            started.elapsed(),
        );
        eprintln!("  {:<14} {}", result.outcome.label(), result.name);
        results.push(result);
    }

    let caps = Capabilities::get();
    Report {
        host: hostname(),
        mtrack_version: mtrack_version(),
        hardware: {
            let mut lines = caps.summary_lines();
            // Probe-time problems (an unmatched MTRACK_E2E_* override, a
            // subsystem that would not enumerate) were previously recorded and
            // never shown, so a typo'd device name produced silence.
            for skip in caps.probe_skips() {
                lines.push(format!("note ({}): {}", skip.area, skip.reason));
            }
            lines
        },
        cabling: Discovery::get().describe(),
        cabling_source: Discovery::get().source().to_string(),
        results,
    }
}

/// Runs one check, converting an unexpected panic into a harness error.
///
/// This is the only place unwinding is caught, and it is supervision rather
/// than control flow: an expected failure is a `CheckError`, but a genuine bug
/// in the harness must not be allowed to destroy the report.
async fn execute(check: &checks::Check) -> CheckOutcome {
    use futures_util::FutureExt;

    match AssertUnwindSafe((check.run)()).catch_unwind().await {
        Ok(outcome) => outcome,
        Err(panic) => Err(CheckError::Harness(format!(
            "the check panicked, which is a bug in the harness rather than a finding: {}",
            panic_message(&panic)
        ))),
    }
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

/// Runs the checks repeatedly and reports how often each one failed.
///
/// Some defects appear in a minority of runs. A single pass cannot distinguish
/// "works" from "works most of the time", and the latter is worse on stage.
pub async fn run_repeated(filter: &Option<String>, times: usize, json: Option<&Path>) -> ExitCode {
    let mut failures: BTreeMap<String, usize> = BTreeMap::new();
    let mut last: Option<Report> = None;
    let mut bad_runs = 0;

    for run in 1..=times {
        println!("\n===== run {run} of {times} =====");
        let report = run_once(filter).await;

        for result in &report.results {
            if result.outcome.is_bad() {
                *failures.entry(result.name.clone()).or_insert(0) += 1;
            }
        }
        if report.has_problems() {
            bad_runs += 1;
        }
        last = Some(report);
    }

    // The final run's report is printed in full so the evidence format is the
    // same however many times it ran.
    if let Some(report) = &last {
        report.print();
        if let Some(path) = json {
            let _ = report.write_json(path);
        }
    }

    println!("\n{}", "=".repeat(72));
    println!("  Repeat summary: {times} runs, {bad_runs} with problems");
    println!("{}", "=".repeat(72));
    if failures.is_empty() {
        println!("  No check failed in any run.");
    } else {
        for (name, count) in &failures {
            let rate = (*count as f64 / times as f64) * 100.0;
            let note = if *count == times {
                "consistent"
            } else {
                "INTERMITTENT -- a fault that appears in only some runs"
            };
            println!("  {name}: failed {count}/{times} ({rate:.0}%) [{note}]");
        }
    }
    println!("{}", "=".repeat(72));

    if bad_runs > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn mtrack_version() -> String {
    crate::server::resolve_mtrack_binary()
        .ok()
        .and_then(|bin| {
            std::process::Command::new(bin)
                .arg("--version")
                .output()
                .ok()
        })
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Convenience for checks that only need to know an area is runnable.
pub fn require_area(name: &'static str) -> Result<(), CheckError> {
    match crate::plan::blocked_reason_for(name) {
        Some(reason) => Err(CheckError::Skipped(reason)),
        None => Ok(()),
    }
}
