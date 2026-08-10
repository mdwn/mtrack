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
//! What a check can conclude.
//!
//! A hardware check has more than two possible endings, and collapsing them
//! into pass/fail is what makes a test harness the wrong tool for this job.
//! "Could not run here" and "ran but the measurement is untrustworthy" are
//! both *not* passes, and neither is a defect in mtrack.
//!
//! Failure is a value rather than a panic. Panics are for bugs; "mtrack routed
//! the wrong channel" is the result this tool exists to report, so it travels
//! as a `Result` like any other outcome.

use std::error::Error;
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

/// Why a check did not simply pass.
#[derive(Debug)]
pub enum CheckError {
    /// The behaviour under test is wrong. This is a defect in mtrack.
    ///
    /// `from_assertion` records whether the check's own assertion produced
    /// this, as opposed to a helper failing before the assertion was reached.
    /// Both are real defect reports in a normal run; only the former proves
    /// anything in `--self-test`, where a control that dies inside
    /// `Server::start` would otherwise look identical to one that drove its
    /// assertion to failure.
    Failed {
        message: String,
        from_assertion: bool,
    },
    /// The check cannot run on this machine, typically for missing hardware.
    /// This will never pass here no matter how often it is run.
    Skipped(String),
    /// The check could have run, but an earlier failure made it moot. Unlike a
    /// skip this says nothing about the machine -- fix the earlier failure and
    /// it becomes runnable.
    Blocked(String),
    /// The check ran but its measurement cannot be trusted, e.g. the audio
    /// interface underran mid-capture. Not a defect, but not evidence either.
    ///
    /// Carries the same origin flag as `Failed`: for a check whose strongest
    /// verdict is "I cannot trust this", an inconclusive raised by its own
    /// logic *is* its assertion firing.
    Inconclusive {
        message: String,
        from_assertion: bool,
    },
    /// The harness itself broke: a connection failed, a temp dir could not be
    /// written. This says nothing about mtrack.
    Harness(String),
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CheckError::Failed { message: m, .. } => write!(f, "{m}"),
            CheckError::Skipped(m) | CheckError::Blocked(m) | CheckError::Harness(m) => {
                write!(f, "{m}")
            }
            CheckError::Inconclusive { message: m, .. } => write!(f, "{m}"),
        }
    }
}

/// Infrastructure errors classify as harness problems, never as defects.
///
/// This is the point of the conversion: `?` on a gRPC call, a file read, or a
/// JSON parse can only ever produce `Harness`, so the sole way to report a
/// defect is an explicit [`check!`]. The two are impossible to confuse.
impl From<Box<dyn Error>> for CheckError {
    fn from(e: Box<dyn Error>) -> CheckError {
        CheckError::Harness(e.to_string())
    }
}

impl From<std::io::Error> for CheckError {
    fn from(e: std::io::Error) -> CheckError {
        CheckError::Harness(e.to_string())
    }
}

impl From<String> for CheckError {
    fn from(e: String) -> CheckError {
        CheckError::Harness(e)
    }
}

impl From<tokio::task::JoinError> for CheckError {
    fn from(e: tokio::task::JoinError) -> CheckError {
        CheckError::Harness(format!("a blocking task failed: {e}"))
    }
}

impl From<tonic::Status> for CheckError {
    fn from(e: tonic::Status) -> CheckError {
        // A Status covers two different things. Unavailable/DeadlineExceeded
        // means we could not reach the player; anything else is the player
        // rejecting a request, which is a statement about its behaviour.
        match e.code() {
            tonic::Code::Unavailable | tonic::Code::DeadlineExceeded | tonic::Code::Unknown => {
                CheckError::Harness(e.to_string())
            }
            // A real defect, but not *this check's assertion*: the rejection
            // came from a helper, and most call sites reach it from setup.
            _ => CheckError::before_assertion(format!("the player rejected a request: {e}")),
        }
    }
}

impl CheckError {
    /// A defect reported by a check's own assertion.
    pub fn assertion(message: impl Into<String>) -> CheckError {
        CheckError::Failed {
            message: message.into(),
            from_assertion: true,
        }
    }

    /// A defect detected before the check's assertion was reached, e.g. the
    /// player never becoming ready. Real in a normal run; proves nothing as a
    /// negative control.
    pub fn before_assertion(message: impl Into<String>) -> CheckError {
        CheckError::Failed {
            message: message.into(),
            from_assertion: false,
        }
    }

    /// A verdict of "this measurement cannot be trusted", from the check itself.
    pub fn inconclusive_assertion(message: impl Into<String>) -> CheckError {
        CheckError::Inconclusive {
            message: message.into(),
            from_assertion: true,
        }
    }

    /// Whether this came from a check's own assertion.
    pub fn is_assertion(&self) -> bool {
        matches!(
            self,
            CheckError::Failed {
                from_assertion: true,
                ..
            } | CheckError::Inconclusive {
                from_assertion: true,
                ..
            }
        )
    }
}

/// Facts a check measured, reported whether or not it passed.
///
/// The evidence is the actual product of a run: "beat clock 41.60 Hz against
/// 41.60 expected" is worth more than the word "passed".
pub type Evidence = Vec<String>;

/// Evidence recorded by the check currently running.
///
/// Held aside rather than returned, because a check that fails abandons its
/// return value -- and that is exactly when its measurements matter most. A
/// beat-clock failure without the "via a loopback port, NOT the configured
/// device" line is a failure that invites the wrong conclusion.
///
/// Checks run strictly one at a time, so a plain mutex is sufficient.
static EVIDENCE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Records a measurement for the running check.
pub fn record(line: impl Into<String>) {
    EVIDENCE
        .lock()
        .expect("evidence buffer poisoned")
        .push(line.into());
}

/// Takes and clears everything recorded so far.
pub fn take_evidence() -> Evidence {
    std::mem::take(&mut *EVIDENCE.lock().expect("evidence buffer poisoned"))
}

/// What a check body returns. Measurements travel through [`record`], so the
/// success value carries nothing.
pub type CheckOutcome = Result<(), CheckError>;

/// How a completed check ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Passed,
    Failed,
    /// Cannot be verified on this machine.
    Skipped,
    /// Runnable in principle, but an earlier failure made it moot.
    Blocked,
    Inconclusive,
    /// The harness broke rather than the software under test.
    HarnessError,
}

impl Outcome {
    /// Whether this outcome should make the run exit non-zero.
    ///
    /// Only a real defect or a broken harness does. A skip is the suite
    /// correctly declining to test absent hardware, and an inconclusive result
    /// is an honest "we could not tell".
    pub fn is_bad(&self) -> bool {
        matches!(self, Outcome::Failed | Outcome::HarnessError)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Outcome::Passed => "PASS",
            Outcome::Failed => "FAIL",
            Outcome::Skipped => "SKIP",
            Outcome::Blocked => "BLOCKED",
            Outcome::Inconclusive => "INCONCLUSIVE",
            Outcome::HarnessError => "HARNESS-ERROR",
        }
    }
}

/// A finished check.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CheckResult {
    pub area: String,
    pub name: String,
    pub description: String,
    pub outcome: Outcome,
    /// Why it did not pass. Absent when it did.
    pub detail: Option<String>,
    pub evidence: Evidence,
    pub duration_ms: u128,
}

impl CheckResult {
    pub fn from_outcome(
        area: &str,
        name: &str,
        description: &str,
        result: CheckOutcome,
        elapsed: Duration,
    ) -> CheckResult {
        // Collected regardless of outcome, so a failure keeps the measurements
        // that explain it.
        let evidence = take_evidence();
        let (outcome, detail) = match result {
            Ok(()) => (Outcome::Passed, None),
            Err(CheckError::Failed { message, .. }) => (Outcome::Failed, Some(message)),
            Err(CheckError::Skipped(m)) => (Outcome::Skipped, Some(m)),
            Err(CheckError::Blocked(m)) => (Outcome::Blocked, Some(m)),
            Err(CheckError::Inconclusive { message, .. }) => (Outcome::Inconclusive, Some(message)),
            Err(CheckError::Harness(m)) => (Outcome::HarnessError, Some(m)),
        };

        CheckResult {
            area: area.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            outcome,
            detail,
            evidence,
            duration_ms: elapsed.as_millis(),
        }
    }
}

/// Reports a defect when `cond` is false.
///
/// The counterpart to `assert!`, except it yields a value instead of
/// unwinding, so the caller keeps control and the run keeps its report.
#[macro_export]
macro_rules! check {
    ($cond:expr, $($arg:tt)+) => {
        // Matched rather than negated: call sites routinely pass float
        // comparisons, and `!(a < b)` on a partially ordered type is both a
        // clippy warning and genuinely ambiguous about NaN.
        match $cond {
            true => {}
            false => {
                return Err($crate::outcome::CheckError::assertion(format!($($arg)+)));
            }
        }
    };
}

/// Reports a defect when two values differ, including both in the message.
#[macro_export]
macro_rules! check_eq {
    ($left:expr, $right:expr, $($arg:tt)+) => {{
        let left = &$left;
        let right = &$right;
        if left != right {
            return Err($crate::outcome::CheckError::assertion(format!(
                "{}\n  expected: {:?}\n  actual:   {:?}",
                format!($($arg)+), right, left
            )));
        }
    }};
}

/// Reports a defect unconditionally.
#[macro_export]
macro_rules! fail {
    ($($arg:tt)+) => {
        return Err($crate::outcome::CheckError::assertion(format!($($arg)+)))
    };
}

/// Declines to run because an earlier failure made this moot.
#[macro_export]
macro_rules! blocked {
    ($($arg:tt)+) => {
        return Err($crate::outcome::CheckError::Blocked(format!($($arg)+)))
    };
}

/// Declines to run, because this machine cannot.
#[macro_export]
macro_rules! skip {
    ($($arg:tt)+) => {
        return Err($crate::outcome::CheckError::Skipped(format!($($arg)+)))
    };
}

/// Ran, but the measurement cannot be trusted.
#[macro_export]
macro_rules! inconclusive {
    ($($arg:tt)+) => {
        return Err($crate::outcome::CheckError::inconclusive_assertion(format!($($arg)+)))
    };
}
