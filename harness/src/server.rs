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
//! Runs the real mtrack binary against a generated project.
//!
//! The suite drives the shipped executable rather than embedding the player in
//! process. Startup ordering, device acquisition, and config load are a large
//! part of what can break against real hardware, and none of it is exercised
//! by constructing a `Player` directly.

use std::fs::File;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::project::Project;

/// Locates the mtrack binary to run.
///
/// `CARGO_BIN_EXE_mtrack` exists only for test targets, so the harness looks
/// where cargo would have put it, honouring `MTRACK_BIN` for an installed or
/// cross-built binary.
pub fn resolve_mtrack_binary() -> Result<PathBuf, String> {
    if let Ok(explicit) = std::env::var("MTRACK_BIN") {
        let path = PathBuf::from(&explicit);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(format!("MTRACK_BIN={explicit} is not a file"))
        };
    }

    // The harness lives in a workspace member, so the shared target directory
    // is one level up from its manifest.
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();

    // Release before debug, unconditionally -- mtime is not consulted. Callers
    // that care (the wrapper script) set MTRACK_BIN explicitly.
    let candidates = [
        workspace.join("target/release/mtrack"),
        workspace.join("target/debug/mtrack"),
    ];
    for candidate in &candidates {
        if candidate.is_file() {
            return Ok(candidate.clone());
        }
    }
    Err(format!(
        "looked for {}",
        candidates
            .iter()
            .map(|c| c.display().to_string())
            .collect::<Vec<_>>()
            .join(" and ")
    ))
}

/// How long to wait for the player to report readiness. Real devices,
/// especially USB interfaces and OLA, are not instant.
const READY_TIMEOUT: Duration = Duration::from_secs(45);

/// How long to wait for a killed process to actually exit.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Set once a player has timed out reaching readiness.
///
/// Hardware that never initializes fails identically for every check, and
/// mtrack deliberately waits and retries forever on a subsystem its profile
/// declares. Paying the readiness timeout once per check turns one problem
/// into twenty-five timeouts and a run that reports nothing useful.
///
/// Latched *only* on a genuine readiness timeout. A harness-side failure -- a
/// log file that could not be created, a port that collided -- says nothing
/// about the player and must not suppress the rest of the run.
static INIT_TIMED_OUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Clears the latch. Called at the start of each run so that `--repeat` gives
/// N independent runs rather than one run followed by N-1 blocked ones.
pub fn reset_init_latch() {
    INIT_TIMED_OUT.store(false, std::sync::atomic::Ordering::SeqCst);
}

/// Why a server failed to come up. The distinction decides whether the failure
/// is reported against mtrack or against the harness.
enum StartFailure {
    /// The player never reported readiness before the deadline. Latching on
    /// this is what stops a wedged device costing one timeout per check.
    Timeout(String),
    /// The player exited. Config-specific, so it must NOT latch: one profile
    /// that makes mtrack exit would otherwise hide every independent defect
    /// behind ~20 blocked checks.
    Exited(String),
    /// The harness could not even get that far, or could not tell. Says
    /// nothing about the player.
    Harness(String),
}

/// A running mtrack process.
pub struct Server {
    child: Option<Child>,
    /// Cached once the child has been reaped; `try_wait` only reports an exit
    /// once.
    exit_status: Option<std::process::ExitStatus>,
    web_port: u16,
    grpc_port: u16,
    log_path: PathBuf,
}

impl Server {
    /// Starts mtrack against the project and waits until it reports ready.
    ///
    /// Readiness means the web API answers *and* hardware initialization has
    /// finished. Polling only for a listening socket would let cases race
    /// against device acquisition and fail intermittently.
    pub async fn start(project: &Project) -> Result<Server, crate::outcome::CheckError> {
        use std::sync::atomic::Ordering;

        if INIT_TIMED_OUT.load(Ordering::SeqCst) {
            return Err(crate::outcome::CheckError::Blocked(
                "the player could not initialize its hardware in an earlier check; this would \
                 fail the same way after the same timeout, so it was not repeated"
                    .to_string(),
            ));
        }

        match Server::start_inner(project, true).await {
            Ok(server) => Ok(server),
            Err(StartFailure::Timeout(msg)) => {
                INIT_TIMED_OUT.store(true, Ordering::SeqCst);
                // A real finding: the player could not come up against a config
                // naming devices it had itself advertised.
                Err(crate::outcome::CheckError::Failed(msg))
            }
            // Also a real finding, but specific to this check's config, so the
            // rest of the run still gets to say something.
            Err(StartFailure::Exited(msg)) => Err(crate::outcome::CheckError::Failed(msg)),
            // Never latched: the harness broke, not the player.
            Err(StartFailure::Harness(msg)) => Err(crate::outcome::CheckError::Harness(msg)),
        }
    }

    /// Starts mtrack but only waits for the API to answer, not for hardware
    /// initialization to succeed. Used by cases that deliberately misconfigure
    /// a device and need to inspect the resulting state.
    pub async fn start_degraded(project: &Project) -> Result<Server, crate::outcome::CheckError> {
        Server::start_inner(project, false)
            .await
            .map_err(|e| match e {
                StartFailure::Timeout(m) | StartFailure::Exited(m) | StartFailure::Harness(m) => {
                    crate::outcome::CheckError::Harness(m)
                }
            })
    }

    async fn start_inner(project: &Project, require_init: bool) -> Result<Server, StartFailure> {
        let harness = StartFailure::Harness;

        // A distinct log per start. Reusing one path meant a restart truncated
        // the previous process's log -- including the errors from the run that
        // performed the mutation being tested.
        static NEXT_LOG: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let index = NEXT_LOG.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let log_path = project.root().join(format!("mtrack-{index}.log"));
        let log = File::create(&log_path).map_err(|e| harness(e.to_string()))?;
        let log_err = log.try_clone().map_err(|e| harness(e.to_string()))?;

        let binary = resolve_mtrack_binary().map_err(harness)?;
        let child = Command::new(binary)
            .arg("start")
            .arg(project.root())
            .arg("--web-port")
            .arg(project.web_port().to_string())
            .arg("--web-address")
            .arg("127.0.0.1")
            // Without this the player's own logging is near-silent, and the log
            // is the only record of what the hardware did during a failure.
            .env(
                "RUST_LOG",
                std::env::var("MTRACK_E2E_LOG").as_deref().unwrap_or("info"),
            )
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| harness(e.to_string()))?;

        let mut server = Server {
            child: Some(child),
            exit_status: None,
            web_port: project.web_port(),
            grpc_port: project.grpc_port(),
            log_path,
        };

        server.wait_until_ready(require_init).await?;
        Ok(server)
    }

    async fn wait_until_ready(&mut self, require_init: bool) -> Result<(), StartFailure> {
        let client = reqwest::Client::new();
        let url = format!("{}/api/status", self.base_url());
        let deadline = Instant::now() + READY_TIMEOUT;

        let mut last_state = String::from("no response");
        while Instant::now() < deadline {
            if let Some(status) = self.exit_status() {
                return Err(StartFailure::Exited(format!(
                    "mtrack exited with {status} during startup.\n--- log ---\n{}",
                    self.log()
                )));
            }

            match client.get(&url).send().await {
                Ok(response) if response.status().is_success() => {
                    let body: serde_json::Value = match response.json().await {
                        Ok(body) => body,
                        Err(e) => {
                            last_state = format!("status body did not parse: {e}");
                            tokio::time::sleep(Duration::from_millis(200)).await;
                            continue;
                        }
                    };
                    let init_done = body
                        .get("hardware")
                        .and_then(|h| h.get("init_done"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if init_done || !require_init {
                        return Ok(());
                    }
                    last_state = format!("hardware still initializing: {}", body["hardware"]);
                }
                Ok(response) => last_state = format!("HTTP {}", response.status()),
                Err(e) => last_state = e.to_string(),
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        Err(StartFailure::Timeout(format!(
            "mtrack was not ready within {READY_TIMEOUT:?} ({last_state}).\n--- log ---\n{}",
            self.log()
        )))
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.web_port)
    }

    pub fn grpc_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.grpc_port)
    }

    /// The process exit status, if it has exited.
    ///
    /// Reaps with `try_wait` rather than probing with `kill(pid, 0)`. An exited
    /// but unreaped child is a zombie, which still owns its pid, so signal 0
    /// *succeeds* for it -- the process would read as alive forever and a
    /// player that died 200 ms into startup would be waited on for the full
    /// readiness timeout instead of being reported immediately.
    fn exit_status(&mut self) -> Option<std::process::ExitStatus> {
        if let Some(status) = self.exit_status {
            return Some(status);
        }
        let status = self.child.as_mut()?.try_wait().ok().flatten()?;
        self.exit_status = Some(status);
        Some(status)
    }

    /// The captured server log.
    pub fn log(&self) -> String {
        std::fs::read_to_string(&self.log_path).unwrap_or_else(|e| format!("<no log: {e}>"))
    }

    /// Lines the player logged at ERROR level, plus any panic output.
    pub fn errors(&self) -> Vec<String> {
        self.log()
            .lines()
            .filter(|line| {
                line.contains("ERROR") || line.contains("panicked at") || line.contains("PANIC")
            })
            .map(|l| l.to_string())
            .collect()
    }

    /// Fails if the player panicked or logged an error that is not expected.
    ///
    /// Hardware runs legitimately produce some noise, so cases pass the
    /// substrings they are willing to tolerate rather than the check being
    /// all-or-nothing.
    pub fn check_clean_log(&self, allowed: &[&str]) -> Result<(), crate::outcome::CheckError> {
        let unexpected: Vec<_> = self
            .errors()
            .into_iter()
            .filter(|line| !allowed.iter().any(|a| line.contains(a)))
            .collect();

        if unexpected.is_empty() {
            Ok(())
        } else {
            Err(crate::outcome::CheckError::Failed(format!(
                "mtrack logged {} unexpected error(s):\n{}",
                unexpected.len(),
                unexpected.join("\n")
            )))
        }
    }

    /// Fails if mtrack reported that it ignored part of the generated config.
    ///
    /// These arrive as warnings, so the error scan does not see them, and the
    /// symptom downstream is a subsystem that silently never appears. That is
    /// a harness bug masquerading as a product bug, so it is worth its own
    /// check.
    pub fn check_config_understood(&self) -> Result<(), crate::outcome::CheckError> {
        const IGNORED_CONFIG: &[&str] = &[
            "ignored when 'profiles' is present",
            "unknown field",
            "unused config",
        ];

        let ignored: Vec<_> = self
            .log()
            .lines()
            .filter(|line| IGNORED_CONFIG.iter().any(|marker| line.contains(marker)))
            .map(|l| l.to_string())
            .collect();

        if ignored.is_empty() {
            Ok(())
        } else {
            Err(crate::outcome::CheckError::Failed(format!(
                "mtrack ignored part of the generated config:\n{}",
                ignored.join("\n")
            )))
        }
    }

    /// Stops the process and waits for it to exit.
    pub fn stop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };

        // Nothing to signal if the child was already reaped. Its pid has gone
        // back to the OS, and raw-signalling it could hit an unrelated process
        // that inherited it. `Child::kill` guards against this; the hand-rolled
        // `libc_kill` does not, so the check has to come first. Reachable
        // whenever a player dies during startup and `wait_until_ready` reaps it.
        if self.exit_status.is_some() {
            return;
        }

        // SIGTERM first so the player can release devices; a killed process can
        // leave a USB interface or OLA universe claimed for the next case.
        #[cfg(unix)]
        unsafe {
            libc_kill(child.id() as i32, 15);
        }
        #[cfg(not(unix))]
        let _ = child.kill();

        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        while Instant::now() < deadline {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => break,
            }
        }

        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Minimal `kill(2)` binding. The suite needs SIGTERM rather than SIGKILL so
/// devices are released cleanly, and pulling in a libc dependency for one call
/// is not worth it.
#[cfg(unix)]
unsafe fn libc_kill(pid: i32, sig: i32) -> i32 {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    unsafe { kill(pid, sig) }
}
