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
//! The run's output.
//!
//! The report is the product, not a side effect of the checks, and it is
//! printed in full however the run ends. A verification tool that loses its
//! findings because one check blew up is worse than useless, because the
//! absence of a finding reads as an absence of a problem.

use std::collections::BTreeMap;

use crate::outcome::{CheckResult, Outcome};

/// Everything one run produced.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Report {
    pub host: String,
    pub mtrack_version: String,
    pub hardware: Vec<String>,
    pub cabling: Vec<String>,
    /// How the cabling map was obtained, e.g. freshly measured or cached.
    pub cabling_source: String,
    pub results: Vec<CheckResult>,
}

impl Report {
    pub fn count(&self, outcome: Outcome) -> usize {
        self.results.iter().filter(|r| r.outcome == outcome).count()
    }

    /// Whether the run should exit non-zero.
    pub fn has_problems(&self) -> bool {
        self.results.iter().any(|r| r.outcome.is_bad())
    }

    /// Renders the full report to stdout.
    pub fn print(&self) {
        println!("\n{}", "=".repeat(72));
        println!("  mtrack hardware verification report");
        println!("{}", "=".repeat(72));
        println!("  host:    {}", self.host);
        println!("  mtrack:  {}", self.mtrack_version);

        if !self.hardware.is_empty() {
            println!("\n  Hardware");
            for line in &self.hardware {
                println!("    {line}");
            }
        }

        println!("\n  Cabling ({})", self.cabling_source);
        if self.cabling.is_empty() {
            println!("    none detected -- nothing can be verified by capture");
        } else {
            for line in &self.cabling {
                println!("    {line}");
            }
        }

        self.print_by_area();
        self.print_problems();
        self.print_gaps();
        self.print_summary();
    }

    fn print_by_area(&self) {
        let mut by_area: BTreeMap<&str, Vec<&CheckResult>> = BTreeMap::new();
        for result in &self.results {
            by_area.entry(result.area.as_str()).or_default().push(result);
        }

        println!("\n  Results");
        for (area, results) in by_area {
            println!("\n    {area}");
            for result in results {
                println!(
                    "      {:<14} {:<44} {:>6}ms",
                    result.outcome.label(),
                    result.name,
                    result.duration_ms
                );
                // Evidence belongs with passes too: the measured numbers are
                // the reason to trust a pass at all.
                for line in &result.evidence {
                    println!("                     - {line}");
                }
            }
        }
    }

    fn print_problems(&self) {
        let problems: Vec<&CheckResult> =
            self.results.iter().filter(|r| r.outcome.is_bad()).collect();
        if problems.is_empty() {
            return;
        }

        println!("\n{}", "-".repeat(72));
        println!("  Problems ({})", problems.len());
        println!("{}", "-".repeat(72));
        for result in problems {
            println!("\n  [{}] {}", result.outcome.label(), result.name);
            println!("  {}", result.description);
            if let Some(detail) = &result.detail {
                for line in detail.lines() {
                    println!("      {line}");
                }
            }
        }
    }

    /// Everything the run could not verify, and why.
    ///
    /// Split by whether the machine *can* verify it. "This rig has no loopback,
    /// so routing is unverifiable here" and "this did not run because something
    /// earlier broke" look identical in a flat skip list, but they call for
    /// completely different responses: change the rig, versus fix the bug.
    fn print_gaps(&self) {
        let unverifiable: Vec<&CheckResult> = self
            .results
            .iter()
            .filter(|r| r.outcome == Outcome::Skipped)
            .collect();
        let blocked: Vec<&CheckResult> = self
            .results
            .iter()
            .filter(|r| r.outcome == Outcome::Blocked)
            .collect();
        let inconclusive: Vec<&CheckResult> = self
            .results
            .iter()
            .filter(|r| r.outcome == Outcome::Inconclusive)
            .collect();

        if unverifiable.is_empty() && blocked.is_empty() && inconclusive.is_empty() {
            return;
        }

        let detail_of = |r: &CheckResult| {
            r.detail
                .clone()
                .unwrap_or_else(|| "no reason recorded".to_string())
        };

        if !unverifiable.is_empty() {
            println!("\n{}", "-".repeat(72));
            println!(
                "  Cannot be verified on this machine ({})",
                unverifiable.len()
            );
            println!("{}", "-".repeat(72));
            println!("  These need different hardware or cabling. Re-running changes nothing.\n");
            for result in unverifiable {
                println!("  {}\n      {}", result.name, detail_of(result));
            }
        }

        if !blocked.is_empty() {
            println!("\n{}", "-".repeat(72));
            println!("  Blocked by an earlier failure ({})", blocked.len());
            println!("{}", "-".repeat(72));
            println!("  These are runnable here. Fix the failures above and they will run.\n");
            for result in blocked {
                println!("  {}\n      {}", result.name, detail_of(result));
            }
        }

        if !inconclusive.is_empty() {
            println!("\n{}", "-".repeat(72));
            println!("  Ran, but the measurement is not trustworthy ({})", inconclusive.len());
            println!("{}", "-".repeat(72));
            for result in inconclusive {
                println!("  {}\n      {}", result.name, detail_of(result));
            }
        }
    }

    fn print_summary(&self) {
        let passed = self.count(Outcome::Passed);
        let failed = self.count(Outcome::Failed);
        let skipped = self.count(Outcome::Skipped);
        let blocked = self.count(Outcome::Blocked);
        let inconclusive = self.count(Outcome::Inconclusive);
        let harness = self.count(Outcome::HarnessError);

        println!("\n{}", "=".repeat(72));
        print!("  {passed} passed");
        if failed > 0 {
            print!(", {failed} failed");
        }
        if harness > 0 {
            print!(", {harness} harness error(s)");
        }
        if skipped > 0 {
            print!(", {skipped} unverifiable here");
        }
        if blocked > 0 {
            print!(", {blocked} blocked");
        }
        if inconclusive > 0 {
            print!(", {inconclusive} inconclusive");
        }
        println!(" (of {} checks)", self.results.len());

        if failed > 0 {
            println!("  {failed} defect(s) found in mtrack.");
        }
        if harness > 0 {
            println!("  {harness} check(s) could not run because the harness itself failed.");
        }
        if skipped > 0 {
            println!("  {skipped} check(s) cannot be verified on this machine's hardware.");
        }
        if blocked > 0 {
            println!("  {blocked} check(s) were blocked by an earlier failure, not by hardware.");
        }
        if inconclusive > 0 {
            println!("  {inconclusive} measurement(s) could not be trusted.");
        }
        println!("{}", "=".repeat(72));
    }

    /// Writes the report as JSON, for diffing runs over time.
    pub fn write_json(&self, path: &std::path::Path) -> std::io::Result<()> {
        let body = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(path, body)
    }
}
