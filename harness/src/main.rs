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
//! Verifies a real mtrack installation against real hardware and reports what
//! it found.
//!
//! This is a report generator rather than a test suite. A hardware check has
//! four possible endings -- it passed, the software is wrong, this machine
//! cannot run it, or the measurement could not be trusted -- and a pass/fail
//! harness can express only two of them, mapping the last two onto "passed".
//!
//! Usage:
//!   mtrack-harness                     verify everything this machine supports
//!   mtrack-harness --only lighting     one area or check-name substring
//!   mtrack-harness --list              show the plan and exit
//!   mtrack-harness --repeat 25         repeat, to surface intermittent faults
//!   mtrack-harness --json report.json  also write machine-readable results

#![allow(dead_code)]

mod capabilities;
mod capture;
mod checks;
mod client;
mod discovery;
mod midi;
mod outcome;
mod plan;
mod project;
mod report;
mod runner;
mod server;
mod songs;

use std::path::PathBuf;
use std::process::ExitCode;

/// Command line options.
struct Options {
    filter: Option<String>,
    list_only: bool,
    repeat: usize,
    json: Option<PathBuf>,
}

fn parse_args() -> Result<Options, String> {
    let mut options = Options {
        filter: None,
        list_only: false,
        repeat: 1,
        json: None,
    };

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--only" => {
                options.filter = Some(args.next().ok_or("--only needs a value")?);
            }
            "--repeat" => {
                options.repeat = args
                    .next()
                    .ok_or("--repeat needs a value")?
                    .parse()
                    .map_err(|_| "--repeat needs a number")?;
            }
            "--json" => {
                options.json = Some(PathBuf::from(args.next().ok_or("--json needs a path")?));
            }
            "--list" => options.list_only = true,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }
    Ok(options)
}

fn print_help() {
    println!(
        "mtrack-harness -- verify mtrack against this machine's hardware\n\n\
         OPTIONS:\n  \
           --only <substring>   run only areas or checks matching this\n  \
           --list               print the plan and exit without running\n  \
           --repeat <n>         run n times; reports how often each check failed\n  \
           --json <path>        write machine-readable results alongside the report\n  \
           -h, --help           this message\n\n\
         ENVIRONMENT:\n  \
           MTRACK_BIN                path to the mtrack binary\n  \
           MTRACK_E2E_DISABLE        pretend a subsystem is absent: audio,midi,dmx\n  \
           MTRACK_E2E_REDISCOVER=1   re-measure cabling, ignoring the cache\n  \
           MTRACK_E2E_PROBE_ALL=1    probe every device pair, not just the selected one\n  \
           MTRACK_E2E_AMPLITUDE      tone amplitude, 0.0-1.0 (default 0.25)\n"
    );
}

#[tokio::main]
async fn main() -> ExitCode {
    let options = match parse_args() {
        Ok(options) => options,
        Err(e) => {
            eprintln!("{e}\n");
            print_help();
            return ExitCode::from(2);
        }
    };

    // Resolving the binary first means an unusable installation is reported
    // immediately rather than as every check failing to start a server.
    if let Err(e) = server::resolve_mtrack_binary() {
        eprintln!("Cannot find the mtrack binary: {e}");
        eprintln!("Build it with `cargo build` or set MTRACK_BIN.");
        return ExitCode::from(2);
    }

    // Listing must not measure: discovery plays tones and sends MIDI, which is
    // not what "print the plan and exit" should do on a live rig.
    if options.list_only {
        discovery::forbid_probing();
        plan::print_plan();
        // The plan is per-area; --only selects checks, so list them too or
        // `--list --only lighting` would show every area and contradict the
        // wrapper's own "what would run" description.
        runner::list_checks(&options.filter);
        return ExitCode::SUCCESS;
    }

    plan::print_plan();

    if !plan::anything_runs() {
        eprintln!(
            "Neither an audio output nor a MIDI port was found, so there is nothing to verify."
        );
        return ExitCode::from(2);
    }

    if options.repeat > 1 {
        return runner::run_repeated(&options.filter, options.repeat, options.json.as_deref())
            .await;
    }

    let report = runner::run_once(&options.filter).await;
    report.print();

    if let Some(path) = &options.json {
        match report.write_json(path) {
            Ok(()) => println!("\nJSON results written to {}", path.display()),
            Err(e) => eprintln!("\nCould not write {}: {e}", path.display()),
        }
    }

    if report.has_problems() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
