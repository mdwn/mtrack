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
}
