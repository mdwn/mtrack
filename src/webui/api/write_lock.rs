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

//! The locks the web UI's and MCP's writes of one kind of file take turns
//! by (#578).

use tokio::sync::{Mutex, MutexGuard};

/// Held by each write of one kind of file, here and over MCP, from reading
/// the file to writing it back, so neither writes over what the other saved
/// in between.
pub(crate) struct WriteLock {
    lock: Mutex<()>,
    /// How many have asked for it, per runtime: each test runs on its own,
    /// so a test can tell that what it started is waiting on the lock it
    /// holds, not some other test's write.
    #[cfg(test)]
    asked: std::sync::Mutex<Vec<(tokio::runtime::Id, usize)>>,
}

impl WriteLock {
    pub(crate) const fn new() -> WriteLock {
        WriteLock {
            lock: Mutex::const_new(()),
            #[cfg(test)]
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Waits for the lock.
    pub(crate) async fn lock(&self) -> MutexGuard<'_, ()> {
        #[cfg(test)]
        self.ask();
        self.lock.lock().await
    }

    /// Waits for the lock, on a thread that may block.
    pub(crate) fn blocking_lock(&self) -> MutexGuard<'_, ()> {
        #[cfg(test)]
        self.ask();
        self.lock.blocking_lock()
    }

    #[cfg(test)]
    fn ask(&self) {
        let Ok(runtime) = tokio::runtime::Handle::try_current().map(|h| h.id()) else {
            return;
        };
        let mut asked = self.asked.lock().expect("asked");
        match asked.iter_mut().find(|(id, _)| *id == runtime) {
            Some((_, n)) => *n += 1,
            None => asked.push((runtime, 1)),
        }
    }

    /// How many on this test's runtime have asked for the lock so far.
    #[cfg(test)]
    pub(crate) fn asked(&self) -> usize {
        let runtime = tokio::runtime::Handle::current().id();
        let asked = self.asked.lock().expect("asked");
        asked
            .iter()
            .find(|(id, _)| *id == runtime)
            .map_or(0, |(_, n)| *n)
    }

    /// Waits, as long as the test patience, until another on this test's
    /// runtime has asked for the lock since [`WriteLock::asked`] said
    /// `since`: what the test started is waiting on it.
    #[cfg(test)]
    pub(crate) async fn wait_for_another(&self, since: usize) {
        crate::testutil::eventually_async(
            || async { self.asked() > since },
            "what the test started never asked for the lock it holds",
        )
        .await;
    }
}
