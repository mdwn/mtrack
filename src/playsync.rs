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
use parking_lot::{Condvar, Mutex};
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc};
use std::time::Duration;

/// Drop guard for the readiness channel.
///
/// Wraps a `Sender<()>` and guarantees that exactly one message is sent — either
/// explicitly via [`send`] or automatically on [`Drop`]. This prevents deadlocks
/// in the `play_files` ready-wait loop when a subsystem returns early without
/// signaling readiness.
///
/// [`send`]: ReadyGuard::send
pub struct ReadyGuard {
    tx: Option<std::sync::mpsc::Sender<()>>,
}

impl ReadyGuard {
    /// Creates a new guard that will send on drop if not already sent.
    pub fn new(tx: std::sync::mpsc::Sender<()>) -> Self {
        Self { tx: Some(tx) }
    }

    /// Explicitly send the ready signal, consuming the inner sender.
    ///
    /// Subsequent calls and the `Drop` impl are no-ops.
    pub fn send(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for ReadyGuard {
    fn drop(&mut self) {
        self.send();
    }
}

/// Bundles the playback synchronization state passed to `play_from` / `play`.
/// Includes the cancel handle, clock, ready signal, start time, and loop control.
pub struct PlaybackSync {
    pub cancel_handle: CancelHandle,
    pub ready_tx: ReadyGuard,
    pub clock: crate::clock::PlaybackClock,
    pub start_time: Duration,
    pub loop_control: LoopControl,
}

/// Bundles the shared loop-control state threaded through audio, MIDI, and DMX
/// `play_from` / `play` methods.
#[derive(Clone)]
pub struct LoopControl {
    /// Shared flag to break out of the whole-song loop gracefully.
    pub loop_break: Arc<AtomicBool>,
    /// Active section loop bounds (shared with player).
    pub active_section: Arc<parking_lot::RwLock<Option<crate::player::SectionBounds>>>,
    /// Shared flag to break out of a section loop.
    pub section_loop_break: Arc<AtomicBool>,
    /// Accumulated time consumed by section loop iterations.
    pub loop_time_consumed: Arc<parking_lot::Mutex<Duration>>,
}

impl LoopControl {
    /// Creates a new `LoopControl` with all flags in their default (inactive) state.
    pub fn new() -> Self {
        Self {
            loop_break: Arc::new(AtomicBool::new(false)),
            active_section: Arc::new(parking_lot::RwLock::new(None)),
            section_loop_break: Arc::new(AtomicBool::new(false)),
            loop_time_consumed: Arc::new(parking_lot::Mutex::new(Duration::ZERO)),
        }
    }
}

impl Default for LoopControl {
    fn default() -> Self {
        Self::new()
    }
}

/// Represents the current cancel state.
#[derive(PartialEq)]
enum CancelState {
    Untouched,
    Cancelled,
}

/// A cancel handle is passed to the device during a play operation. It's the player's responsibility
/// to respect a cancel request.
#[derive(Clone)]
pub struct CancelHandle {
    /// A boolean that should be set to true if the underlying operation should be cancelled.
    cancelled: Arc<Mutex<CancelState>>,
    /// The condvar will handle notification of cancelling.
    condvar: Arc<Condvar>,
}

impl CancelHandle {
    /// Creates a new cancel handle.
    pub fn new() -> CancelHandle {
        CancelHandle {
            cancelled: Arc::new(Mutex::new(CancelState::Untouched)),
            condvar: Arc::new(Condvar::new()),
        }
    }
}

impl Default for CancelHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl CancelHandle {
    /// Returns true if the device process has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        *self.cancelled.lock() == CancelState::Cancelled
    }

    /// Waits for the cancel handle to be cancelled or for finished to be set to true.
    pub fn wait(&self, finished: Arc<AtomicBool>) {
        let mut guard = self.cancelled.lock();
        self.condvar.wait_while(&mut guard, |cancelled| {
            *cancelled == CancelState::Untouched && !finished.load(Ordering::Relaxed)
        });
    }

    /// Waits for the cancel handle to be cancelled or for finished to be set to true,
    /// with a timeout. Returns `true` if the condition was met, `false` if timed out.
    pub fn wait_with_timeout(
        &self,
        finished: Arc<AtomicBool>,
        timeout: std::time::Duration,
    ) -> bool {
        let mut guard = self.cancelled.lock();
        let result = self.condvar.wait_while_for(
            &mut guard,
            |cancelled| *cancelled == CancelState::Untouched && !finished.load(Ordering::Relaxed),
            timeout,
        );
        !result.timed_out()
    }

    /// Notifies the cancel handle to see if this the song has been cancelled or if the
    /// particular element has finished.
    ///
    /// Acquires the mutex before signaling so the notification cannot be lost
    /// between a waiter's predicate check and its condvar sleep.
    pub fn notify(&self) {
        let _guard = self.cancelled.lock();
        self.condvar.notify_all();
    }

    /// Cancel the device process.
    pub fn cancel(&self) {
        let mut cancel_state = self.cancelled.lock();
        if *cancel_state == CancelState::Untouched {
            *cancel_state = CancelState::Cancelled;
            // Signal directly — we already hold the mutex.
            self.condvar.notify_all();
        }
    }
}

#[cfg(test)]
mod test {
    use std::thread;

    use super::*;

    #[test]
    fn test_cancel_handle_cancelled() {
        let cancel_handle = CancelHandle::new();
        assert!(!cancel_handle.is_cancelled());

        let join = {
            let cancel_handle = cancel_handle.clone();
            thread::spawn(move || cancel_handle.wait(Arc::new(AtomicBool::new(false))))
        };

        cancel_handle.cancel();
        assert!(join.join().is_ok());
        assert!(cancel_handle.is_cancelled());
    }

    #[test]
    fn test_cancel_handle_finished() {
        let cancel_handle = CancelHandle::new();
        assert!(!cancel_handle.is_cancelled());

        let join = {
            let cancel_handle = cancel_handle.clone();
            thread::spawn(move || cancel_handle.wait(Arc::new(AtomicBool::new(true))))
        };

        assert!(join.join().is_ok());
        assert!(!cancel_handle.is_cancelled());
    }

    #[test]
    fn test_wait_with_timeout_returns_true_when_finished() {
        let cancel_handle = CancelHandle::new();
        let finished = Arc::new(AtomicBool::new(true));
        assert!(cancel_handle.wait_with_timeout(finished, std::time::Duration::from_secs(1)));
    }

    #[test]
    fn test_wait_with_timeout_returns_false_on_timeout() {
        let cancel_handle = CancelHandle::new();
        let finished = Arc::new(AtomicBool::new(false));
        assert!(!cancel_handle.wait_with_timeout(finished, std::time::Duration::from_millis(50)));
    }

    #[test]
    fn test_wait_with_timeout_returns_true_when_cancelled() {
        let cancel_handle = CancelHandle::new();
        let finished = Arc::new(AtomicBool::new(false));

        let join = {
            let cancel_handle = cancel_handle.clone();
            thread::spawn(move || {
                cancel_handle.wait_with_timeout(finished, std::time::Duration::from_secs(10))
            })
        };

        cancel_handle.cancel();
        assert!(join.join().unwrap());
    }

    #[test]
    fn test_cancel_idempotent() {
        let cancel_handle = CancelHandle::new();
        assert!(!cancel_handle.is_cancelled());

        cancel_handle.cancel();
        assert!(cancel_handle.is_cancelled());

        // Second cancel should be a no-op
        cancel_handle.cancel();
        assert!(cancel_handle.is_cancelled());
    }

    #[test]
    fn test_default_impl() {
        let cancel_handle = CancelHandle::default();
        assert!(!cancel_handle.is_cancelled());
    }

    #[test]
    fn test_clone_shares_state() {
        let handle1 = CancelHandle::new();
        let handle2 = handle1.clone();

        assert!(!handle2.is_cancelled());
        handle1.cancel();
        assert!(handle2.is_cancelled());
    }

    #[test]
    fn test_notify_wakes_waiter_when_finished() {
        let cancel_handle = CancelHandle::new();
        let finished = Arc::new(AtomicBool::new(false));

        let join = {
            let cancel_handle = cancel_handle.clone();
            let finished = finished.clone();
            thread::spawn(move || cancel_handle.wait(finished))
        };

        // Set finished and notify
        finished.store(true, Ordering::Relaxed);
        cancel_handle.notify();

        assert!(join.join().is_ok());
        // Should NOT be cancelled — just finished
        assert!(!cancel_handle.is_cancelled());
    }

    #[test]
    fn test_wait_returns_immediately_when_already_cancelled() {
        let cancel_handle = CancelHandle::new();
        cancel_handle.cancel();

        // Wait should return immediately since already cancelled
        let finished = Arc::new(AtomicBool::new(false));
        cancel_handle.wait(finished);
        assert!(cancel_handle.is_cancelled());
    }

    #[test]
    fn test_wait_with_timeout_returns_immediately_when_already_finished() {
        let cancel_handle = CancelHandle::new();
        let finished = Arc::new(AtomicBool::new(true));

        let result = cancel_handle.wait_with_timeout(finished, std::time::Duration::from_millis(1));
        assert!(result);
    }

    #[test]
    fn test_ready_guard_explicit_send() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let mut guard = ReadyGuard::new(tx);
        guard.send();
        assert!(rx.try_recv().is_ok(), "explicit send should deliver");
        // Second send is a no-op.
        guard.send();
        assert!(
            rx.try_recv().is_err(),
            "second send should not deliver again"
        );
    }

    #[test]
    fn test_ready_guard_sends_on_drop() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        {
            let _guard = ReadyGuard::new(tx);
            // guard dropped here without explicit send
        }
        assert!(
            rx.try_recv().is_ok(),
            "drop should send the ready signal automatically"
        );
    }

    #[test]
    fn test_ready_guard_no_double_send_on_drop() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        {
            let mut guard = ReadyGuard::new(tx);
            guard.send();
            // guard dropped here; should not send again
        }
        assert!(rx.try_recv().is_ok(), "explicit send should arrive");
        assert!(
            rx.try_recv().is_err(),
            "drop after explicit send should not send again"
        );
    }
}
