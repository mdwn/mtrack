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
//! Where headless log lines are written.
//!
//! Log lines must not go through file descriptor 1. Audio device enumeration
//! silences ALSA by swapping the process's fd 1 and fd 2 for a pipe (see
//! `silence_enumeration` in `audio::cpal::device`), and a line any thread
//! writes to fd 1 in that window goes into the pipe and is thrown away with it.
//! At startup the window covers everything the DMX engine and the web server
//! log while the audio device opens — under systemd, with olad already up,
//! that was "DMX engine initialized", "Effects loop started", olad's
//! unpatched-universe warning and "mtrack is running", every time.
//!
//! So the log writes to its own copy of stdout, taken before anything can
//! swap it: the same journal stream or terminal, on a descriptor nothing
//! redirects.

use std::io::IsTerminal;

use tracing_subscriber::fmt::writer::BoxMakeWriter;

/// A writer onto the process's stdout as it is now, unaffected by later
/// redirection of fd 1.
///
/// Call it before anything can redirect fd 1 (first thing in `main`). If
/// stdout cannot be duplicated — it is closed, say — this falls back to
/// writing through fd 1, which is what a closed stdout would get anyway.
pub fn stdout_writer() -> BoxMakeWriter {
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        if let Some(writer) = copy_of(std::io::stdout().as_fd()) {
            return writer;
        }
    }
    BoxMakeWriter::new(std::io::stdout)
}

/// A writer onto a copy of `fd`, if it can be duplicated.
#[cfg(unix)]
fn copy_of(fd: std::os::fd::BorrowedFd<'_>) -> Option<BoxMakeWriter> {
    let copy = fd.try_clone_to_owned().ok()?;
    // The mutex keeps two threads' lines from interleaving, as the lock inside
    // `Stdout` did.
    Some(BoxMakeWriter::new(std::sync::Mutex::new(
        std::fs::File::from(copy),
    )))
}

/// Whether log lines should carry ANSI colour: only on a terminal, and not when
/// `NO_COLOR` is set. Under systemd the escape codes went into every journal
/// line, which the journal then stores as binary data rather than text.
///
/// The writer from [`stdout_writer`] is a plain file to the formatter, so this
/// has to be decided here rather than left to it.
pub fn stdout_wants_colour() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::Read;
    use std::os::fd::AsFd;

    use tracing_subscriber::fmt::writer::BoxMakeWriter;
    use tracing_subscriber::layer::SubscriberExt;

    // These run on fd 2 rather than fd 1: the test harness reports results on
    // fd 1, and capturing it here would swallow other tests' lines. ALSA's
    // silence swaps both the same way, so a copy that survives one survives
    // the other.

    /// Logs one line through a subscriber writing to `writer`, while fd 2 is
    /// silenced the way ALSA enumeration silences it.
    fn log_while_silenced(writer: BoxMakeWriter) {
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer),
        );
        tracing::subscriber::with_default(subscriber, || {
            let _stderr = shh::stderr().expect("silence stderr");
            tracing::info!(lighting = true, "DMX engine initialized");
        });
    }

    /// What reached fd 2's real destination while `f` ran. fd 2 is captured
    /// with `shh` to stand in for the journal; the ALSA silence then nests
    /// inside that capture, as it nests inside the journal stream in
    /// production.
    fn captured_stderr(f: impl FnOnce()) -> String {
        // No device walk may swap the descriptors under this test.
        let _lock = crate::audio::cpal::ENUMERATION_LOCK.lock();
        let mut journal = shh::stderr().expect("capture stderr");
        f();
        let mut out = String::new();
        journal.read_to_string(&mut out).expect("read capture");
        out
    }

    #[test]
    fn a_line_logged_to_a_copy_while_alsa_is_silenced_arrives() {
        let out = captured_stderr(|| {
            let writer = super::copy_of(std::io::stderr().as_fd()).expect("dup stderr");
            log_while_silenced(writer)
        });
        assert!(
            out.contains("DMX engine initialized"),
            "the line was lost; the journal got {out:?}"
        );
    }

    /// The bug the copy exists for: a line written through the standard
    /// descriptor while ALSA is silenced is lost.
    #[test]
    fn a_line_written_through_the_descriptor_while_silenced_is_lost() {
        let out = captured_stderr(|| log_while_silenced(BoxMakeWriter::new(std::io::stderr)));
        assert!(
            !out.contains("DMX engine initialized"),
            "the journal got {out:?}"
        );
    }
}
