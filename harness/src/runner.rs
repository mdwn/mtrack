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
use crate::outcome::{CheckError, CheckOutcome, CheckResult, Outcome};
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
    // Tracked apart: a harness-side error recurring across runs is our bug, and
    // labelling it "an intermittent mtrack defect" would send the reader hunting
    // in the wrong codebase.
    let mut failures: BTreeMap<String, usize> = BTreeMap::new();
    let mut harness_errors: BTreeMap<String, usize> = BTreeMap::new();
    let mut last: Option<Report> = None;
    let mut bad_runs = 0;

    for run in 1..=times {
        println!("\n===== run {run} of {times} =====");
        let report = run_once(filter).await;

        for result in &report.results {
            match result.outcome {
                Outcome::Failed => *failures.entry(result.name.clone()).or_insert(0) += 1,
                Outcome::HarnessError => {
                    *harness_errors.entry(result.name.clone()).or_insert(0) += 1
                }
                _ => {}
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
    if !harness_errors.is_empty() {
        println!("  Harness errors (our bug, not mtrack's):");
        for (name, count) in &harness_errors {
            println!("    {name}: {count}/{times}");
        }
    }
    if failures.is_empty() {
        println!("  No check reported an mtrack defect in any run.");
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

/// Prints the checks a run would execute, honouring the same filter.
pub fn list_checks(filter: &Option<String>) {
    println!("=== checks selected ===");
    let mut shown = 0;
    for check in checks::all() {
        if let Some(needle) = filter {
            if !check.name.contains(needle.as_str()) && !check.area.contains(needle.as_str()) {
                continue;
            }
        }
        shown += 1;
        let blocked = crate::plan::blocked_reason_for(check.area);
        match blocked {
            Some(reason) => println!("  SKIP  {:<14} {:<44} {reason}", check.area, check.name),
            None => println!("  RUN   {:<14} {}", check.area, check.name),
        }
    }
    match filter {
        Some(needle) => println!("\n  {shown} check(s) match '{needle}'."),
        None => println!("\n  {shown} check(s) total."),
    }
    println!("=======================\n");
}

/// Runs every check with its premise deliberately broken and requires each to
/// stop passing.
///
/// A check that still passes here cannot fail, which means it has been
/// reporting success unconditionally. That is a defect in the harness and is
/// reported as one, with a non-zero exit.
pub async fn run_self_test(filter: &Option<String>) -> ExitCode {
    println!("\n{}", "=".repeat(72));
    println!("  SELF-TEST — breaking each check's premise; none may pass");
    println!("{}", "=".repeat(72));
    println!("  A check that passes here reports success unconditionally.\n");

    let mut cannot_fail = Vec::new();
    let mut proved = 0;
    let mut not_runnable = Vec::new();
    let mut no_control = Vec::new();
    let mut predicate_level = 0;
    let mut unproven: Vec<(&str, &str)> = Vec::new();

    for check in checks::all() {
        if let Some(needle) = filter {
            if !check.name.contains(needle.as_str()) && !check.area.contains(needle.as_str()) {
                continue;
            }
        }

        // Each control is independent. Several sabotages deliberately stop the
        // player reaching readiness, and without this the first one would latch
        // and block every control after it -- which is exactly what happened on
        // the first run, suppressing twelve of twenty-six.
        crate::server::reset_init_latch();

        crate::sabotage::enable();
        let outcome = execute(&check).await;
        crate::sabotage::disable();

        // Whether the check's own assertion produced this, as opposed to a
        // helper failing first. A control that dies inside `Server::start`
        // reports Failed and would otherwise be indistinguishable from one
        // that drove its assertion to failure.
        let from_assertion = matches!(&outcome, Err(e) if e.is_assertion());

        let result = CheckResult::from_outcome(
            check.area,
            check.name,
            check.description,
            outcome,
            std::time::Duration::ZERO,
        );
        match result.outcome {
            // The only outcome that proves anything: the assertion fired and
            // reported a defect.
            // Some checks' strongest verdict is "I cannot trust this", so an
            // inconclusive raised by the check itself is its assertion firing.
            Outcome::Failed | Outcome::Inconclusive if from_assertion => {
                let kind = if checks::is_predicate_level(check.name) {
                    "assertion fired (predicate-level control)"
                } else {
                    "assertion fired"
                };
                println!("  ok           {:<46} {kind}", check.name);
                if checks::is_predicate_level(check.name) {
                    predicate_level += 1;
                }
                proved += 1;
            }
            // Failed, but before the assertion was reached.
            Outcome::Failed => {
                println!(
                    "  NO ASSERTION {:<46} failed before the assertion ran",
                    check.name
                );
                unproven.push((check.name, "pre-assertion failure"));
            }
            Outcome::Passed => {
                println!("  CANNOT FAIL  {:<46}", check.name);
                cannot_fail.push(check.name);
            }
            // The machine cannot run this check at all, so the control says
            // nothing either way.
            // The check itself runs here; only its control is impossible. A
            // distinct variant rather than a string prefix, so the two cannot
            // drift apart in spelling.
            Outcome::NoControlHere => {
                println!(
                    "  NO CONTROL   {:<46} {}",
                    check.name,
                    result.detail.as_deref().unwrap_or("")
                );
                no_control.push(check.name);
            }
            Outcome::Skipped | Outcome::Blocked => {
                println!(
                    "  not run here {:<46} {}",
                    check.name,
                    result.detail.as_deref().unwrap_or("")
                );
                not_runnable.push(check.name);
            }
            // The control broke the run before the assertion could be reached.
            // A check that crashes under sabotage has been shown to crash, not
            // to detect anything -- counting this as proof is what let several
            // useless controls report "ok".
            other => {
                println!(
                    "  NO ASSERTION {:<46} control ended in {}",
                    check.name,
                    other.label()
                );
                unproven.push((check.name, other.label()));
            }
        }
    }

    println!("\n{}", "=".repeat(72));
    println!("  {proved} proved capable of failing (the assertion fired)");
    if predicate_level > 0 {
        println!(
            "  of which {predicate_level} rest on predicate-level controls: they substitute the\n             \x20 value the assertion reads, so they prove it is reachable but would not catch a\n             \x20 check made vacuous by reading something insensitive to mtrack's behaviour."
        );
    }
    for stale in checks::stale_world_level_entries() {
        println!(
            "  WARNING: WORLD_LEVEL lists '{stale}', which is not a registered check. \
             Remove it -- while it is listed, nothing is credited, but the name is misleading."
        );
    }
    if !not_runnable.is_empty() {
        println!(
            "  {} not runnable on this machine: {}",
            not_runnable.len(),
            not_runnable.join(", ")
        );
    }
    if !no_control.is_empty() {
        println!(
            "\n  {} check(s) run here but have no usable control on this hardware:",
            no_control.len()
        );
        for name in &no_control {
            println!("    {name}");
        }
        println!("  They pass in a normal run; nothing here proves they could fail.");
    }
    if !unproven.is_empty() {
        println!(
            "\n  {} control(s) never reached their assertion:",
            unproven.len()
        );
        for (name, how) in &unproven {
            println!("    {name} (ended in {how})");
        }
        println!("  These prove the check can break, not that it can detect anything.");
        println!("  Move the sabotage closer to what the assertion reads.");
    }
    if !cannot_fail.is_empty() {
        println!(
            "\n  {} CANNOT FAIL — these report success unconditionally:",
            cannot_fail.len()
        );
        for name in &cannot_fail {
            println!("    {name}");
        }
        println!("  Give each a sabotage point (see harness/src/sabotage.rs).");
    }
    // Proving nothing is not success. `--self-test --only <typo>` selects no
    // checks, and hardware trouble can push every area into Skipped -- both
    // would otherwise print a clean verdict and exit 0, so CI wired to this
    // would go green having verified nothing.
    if proved == 0 {
        println!("\n  NOTHING PROVED — no check's assertion fired.");
        if not_runnable.is_empty()
            && cannot_fail.is_empty()
            && unproven.is_empty()
            && no_control.is_empty()
        {
            println!("  No checks were selected. Check the --only filter.");
        } else {
            println!("  Every selected check was unrunnable here or had a broken control.");
        }
    } else if cannot_fail.is_empty() && unproven.is_empty() && no_control.is_empty() {
        println!("\n  Every check runnable here proved capable of failing.");
    }
    println!("{}", "=".repeat(72));

    if proved > 0 && cannot_fail.is_empty() && unproven.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
