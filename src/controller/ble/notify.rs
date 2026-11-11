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

//! Sends chunks to the phones subscribed to a characteristic.
//!
//! BlueZ notifies every subscriber of a characteristic when any one
//! notification session emits a value. With one session per subscribed
//! phone, a chunk sent on each would reach every phone once per session.
//! So one task owns all the sessions of a characteristic and sends each
//! chunk through just one of them.
//!
//! The `response` and `status` characteristics are the control ones: what
//! waits on them goes before the next chunk of a stream (see
//! [`super::stream`]). [`Control`] is how the stream pump knows.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use futures_util::future::BoxFuture;
use tokio::sync::{mpsc, Notify};

/// One phone's notification session on a characteristic.
pub(crate) trait Notifier: Send + 'static {
    /// Whether the phone ended the session.
    fn is_stopped(&self) -> bool;
    /// Sends a value. False when the session can no longer send.
    fn notify(&mut self, value: Vec<u8>) -> BoxFuture<'_, bool>;
    /// Resolves when the phone ends the session, for a pump with nothing to
    /// send, which would otherwise not look. One that cannot tell never
    /// resolves, and the end is found at the next send.
    fn stopped(&self) -> BoxFuture<'static, ()> {
        Box::pin(std::future::pending())
    }
}

/// The frames waiting on the control characteristics, `response` and
/// `status`: counted from when one is queued for its pump until the pump
/// has sent the last of its chunks. A stream sends no chunk while there
/// are any.
#[derive(Default)]
pub(crate) struct Control {
    waiting: AtomicUsize,
    idle: Notify,
}

impl Control {
    /// A frame was queued for a control pump, or is about to be.
    pub(crate) fn enter(&self) {
        self.waiting.fetch_add(1, Ordering::AcqRel);
    }

    /// That frame was sent, or will not be.
    pub(crate) fn leave(&self) {
        if self.waiting.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.idle.notify_waiters();
        }
    }

    /// How many frames wait.
    #[cfg(test)]
    pub(crate) fn waiting(&self) -> usize {
        self.waiting.load(Ordering::Acquire)
    }

    /// Returns once nothing waits on a control characteristic: at once when
    /// nothing does, which is the usual case.
    pub(crate) async fn clear(&self) {
        loop {
            // Created before the count is read, so a `leave` in between
            // still wakes it.
            let idle = self.idle.notified();
            if self.waiting.load(Ordering::Acquire) == 0 {
                return;
            }
            idle.await;
        }
    }
}

/// What a newly subscribed phone is sent at once, so it need not wait for
/// the next change: chunks, possibly none.
pub(crate) type Greeting = Box<dyn Fn() -> Vec<Vec<u8>> + Send + Sync>;

/// Sends each batch of chunks that arrives on `frames`, in order and with
/// no other batch between them, to the phones whose sessions arrive on
/// `sessions`. A batch is a whole frame: a phone that subscribes between
/// two batches starts at a frame boundary. Ends when either channel closes.
///
/// With `control`, this is a control characteristic's pump: whoever queues
/// a batch on `frames` has counted it in with [`Control::enter`], and it is
/// counted out here when its last chunk is sent.
pub(crate) async fn run<N: Notifier>(
    mut sessions: mpsc::UnboundedReceiver<N>,
    mut frames: mpsc::Receiver<Vec<Vec<u8>>>,
    greeting: Option<Greeting>,
    control: Option<Arc<Control>>,
) {
    let mut live: Vec<N> = Vec::new();
    loop {
        tokio::select! {
            session = sessions.recv() => {
                let Some(session) = session else { return };
                live.push(session);
                if let Some(greeting) = &greeting {
                    if let Some(control) = &control {
                        control.enter();
                    }
                    send(&mut live, greeting()).await;
                    if let Some(control) = &control {
                        control.leave();
                    }
                }
            }
            batch = frames.recv() => {
                let Some(batch) = batch else { return };
                send(&mut live, batch).await;
                if let Some(control) = &control {
                    control.leave();
                }
            }
        }
    }
}

/// Sends `batch` through the first session that will take it, dropping
/// sessions that ended. With none left the batch is dropped: nobody listens.
async fn send<N: Notifier>(live: &mut Vec<N>, batch: Vec<Vec<u8>>) {
    for chunk in batch {
        if !send_chunk(live, chunk).await {
            return;
        }
    }
}

/// Sends one chunk through the first session that will take it, dropping
/// sessions that ended. False when none is left: nobody listens.
pub(super) async fn send_chunk<N: Notifier>(live: &mut Vec<N>, chunk: Vec<u8>) -> bool {
    loop {
        live.retain(|session| !session.is_stopped());
        let Some(session) = live.first_mut() else {
            return false;
        };
        if session.notify(chunk.clone()).await {
            return true;
        }
        live.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;

    /// What the sessions were sent, by session name.
    type Sent = Arc<parking_lot::Mutex<Vec<(&'static str, Vec<u8>)>>>;

    /// A session that records what it is sent.
    struct Fake {
        name: &'static str,
        sent: Sent,
        stopped: Arc<std::sync::atomic::AtomicBool>,
        fails: bool,
    }

    impl Notifier for Fake {
        fn is_stopped(&self) -> bool {
            self.stopped.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn notify(&mut self, value: Vec<u8>) -> BoxFuture<'_, bool> {
            Box::pin(async move {
                if self.fails {
                    return false;
                }
                self.sent.lock().push((self.name, value));
                true
            })
        }
    }

    fn fake(
        name: &'static str,
        sent: &Sent,
        fails: bool,
    ) -> (Fake, Arc<std::sync::atomic::AtomicBool>) {
        let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        (
            Fake {
                name,
                sent: sent.clone(),
                stopped: stopped.clone(),
                fails,
            },
            stopped,
        )
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn each_chunk_goes_out_once_however_many_phones_listen() {
        let sent: Sent = Default::default();
        let (sessions, sessions_rx) = mpsc::unbounded_channel();
        let (frames, frames_rx) = mpsc::channel(8);
        tokio::spawn(run(sessions_rx, frames_rx, None, None));

        sessions.send(fake("a", &sent, false).0).unwrap();
        sessions.send(fake("b", &sent, false).0).unwrap();
        settle().await;
        frames.send(vec![vec![1], vec![2]]).await.unwrap();
        frames.send(vec![vec![3]]).await.unwrap();
        settle().await;

        let sent = sent.lock().clone();
        assert_eq!(sent, vec![("a", vec![1]), ("a", vec![2]), ("a", vec![3])]);
    }

    #[tokio::test]
    async fn the_next_session_takes_over_when_one_ends_or_fails() {
        let sent: Sent = Default::default();
        let (sessions, sessions_rx) = mpsc::unbounded_channel();
        let (frames, frames_rx) = mpsc::channel(8);
        tokio::spawn(run(sessions_rx, frames_rx, None, None));

        let (a, a_stopped) = fake("a", &sent, false);
        sessions.send(a).unwrap();
        sessions.send(fake("broken", &sent, true).0).unwrap();
        sessions.send(fake("c", &sent, false).0).unwrap();
        settle().await;

        frames.send(vec![vec![1]]).await.unwrap();
        settle().await;
        a_stopped.store(true, std::sync::atomic::Ordering::SeqCst);
        frames.send(vec![vec![2]]).await.unwrap();
        settle().await;

        // The broken one was passed over on the way to "c".
        assert_eq!(sent.lock().clone(), vec![("a", vec![1]), ("c", vec![2])]);
    }

    #[tokio::test]
    async fn chunks_with_nobody_listening_are_dropped_not_kept() {
        let sent: Sent = Default::default();
        let (sessions, sessions_rx) = mpsc::unbounded_channel();
        let (frames, frames_rx) = mpsc::channel(8);
        tokio::spawn(run(sessions_rx, frames_rx, None, None));

        frames.send(vec![vec![1]]).await.unwrap();
        settle().await;
        sessions.send(fake("a", &sent, false).0).unwrap();
        settle().await;
        assert!(sent.lock().is_empty());
    }

    #[tokio::test]
    async fn a_new_subscriber_is_greeted_with_the_latest() {
        let sent: Sent = Default::default();
        let (sessions, sessions_rx) = mpsc::unbounded_channel();
        let (_frames, frames_rx) = mpsc::channel(8);
        let greeting: Greeting = Box::new(|| vec![vec![9, 9], vec![8]]);
        tokio::spawn(run(sessions_rx, frames_rx, Some(greeting), None));

        sessions.send(fake("a", &sent, false).0).unwrap();
        settle().await;
        assert_eq!(sent.lock().clone(), vec![("a", vec![9, 9]), ("a", vec![8])]);
    }
}
