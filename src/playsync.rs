// Copyright (C) 2024 Michael Wilson <mike@mdwn.dev>
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
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc, Condvar, Mutex};

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

    /// Returns true if the device process has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        *self.cancelled.lock().expect("Error getting lock") == CancelState::Cancelled
    }

    /// Waits for the cancel handle to be cancelled or for finished to be set to true.
    pub fn wait(&self, finished: Arc<AtomicBool>) {
        let _unused = self
            .condvar
            .wait_while(
                self.cancelled.lock().expect("Error getting lock"),
                |cancelled| {
                    *cancelled == CancelState::Untouched && !finished.load(Ordering::Relaxed)
                },
            )
            .expect("Error getting lock");
    }

    /// Notifies the cancel handle to see if this the song has been cancelled or if the
    /// particular element has finished.
    pub fn notify(&self) {
        self.condvar.notify_all();
    }

    /// Cancel the device process.
    pub fn cancel(&self) {
        let mut cancel_state = self.cancelled.lock().expect("Error getting lock");
        if *cancel_state == CancelState::Untouched {
            *cancel_state = CancelState::Cancelled;
            self.notify();
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
}
