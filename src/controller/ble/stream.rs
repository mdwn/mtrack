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

//! Server streams over the `stream` characteristic.
//!
//! A gRPC stream is ordered and loses nothing. A radio that is slower than
//! the stream would then fall further and further behind, and a lighting
//! frame that is late is worthless. So between the service's stream and the
//! radio each open stream has a small buffer, a [`Slot`], with one rule,
//! *newest wins*: a method says which of its messages are replaceable, at
//! most one of those waits, and a newer one takes its place. The rest are
//! never dropped and keep their order: one of them arriving discards the
//! replaceable message waiting in front of it, which was for what came
//! before, and its reader waits for room instead of the buffer growing.
//!
//! # Tasking
//!
//! Each open stream has a reader task, [`read`], which owns the service's
//! side of the stream and fills the stream's slot. One pump task, [`run`],
//! empties every slot onto the radio, a chunk at a time. Before each chunk
//! it waits until nothing is waiting on `response` or `status`
//! ([`Control`]), and until its [`Pacer`] lets it. A message is taken out
//! of its slot only when its first chunk is about to go, so until then a
//! newer one can still replace it, and from then on it is sent whole.
//!
//! # What the radio stack does not tell us
//!
//! BlueZ takes a notification as a D-Bus signal and queues it for the link
//! itself, with no word back on when it left or how many wait. Handing it a
//! chunk therefore never blocks, whatever the phone's speed, and a chunk
//! handed over cannot be overtaken: a Stop's answer would sit behind every
//! chunk of a layout already given to it. The pump's own rate is the only
//! bound on that queue, so it hands chunks over no faster than
//! [`STREAM_RATE`], which is meant to be under what any link carries. While
//! that holds, at most a chunk or so of a stream is ever ahead of a
//! response, and newest wins happens here, where it can. On a link slower
//! than the rate, the queue inside BlueZ grows and nothing here can see it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use prost::Message;
use tokio::sync::{mpsc, Notify};
use tokio::time::Instant;
use tonic::{Code, Status};

use crate::proto::ble::v1::Response;

use super::codec::{chunks, MAX_FRAME};
use super::notify::{send_chunk, Control, Notifier};
use super::session::PeerId;

/// How many streams one phone may have open at once.
pub(crate) const MAX_STREAMS: usize = 2;

/// The bytes a second the pump hands to the radio stack for streams, at
/// most. A Pixel's link was measured carrying about 50,000 a second and an
/// iPhone's is assumed to carry a third of that; this is under both. The
/// stage stream needs a few thousand.
pub(crate) const STREAM_RATE: u32 = 16_000;

/// How many refusals of streams (a third one asked for) may wait for the
/// pump. They are answers to requests that opened nothing, so past this
/// they are dropped, as a phone's excess requests are.
const NOTICE_DEPTH: usize = 8;

/// How far behind its rate the pacer lets the pump catch up at once: about
/// what a timer oversleeps by, so that short waits do not add up to a
/// slower rate.
const PACER_SLACK: Duration = Duration::from_millis(5);

/// Says whether a serialized message of a stream is replaceable: whether a
/// newer replaceable message of the same stream makes it worthless.
pub(crate) type Replaceable = fn(&[u8]) -> bool;

/// What a stream says next.
pub(crate) enum Step {
    /// A serialized message of the stream.
    Message(Vec<u8>),
    /// The stream is over, with this status. Nothing follows.
    End(Status),
}

/// The service's side of an open stream. Dropping it drops the service's
/// stream.
pub(crate) trait Messages: Send + 'static {
    fn next(&mut self) -> BoxFuture<'_, Step>;
}

/// Why a stream was not opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The phone has a stream open under the id already. The one refusal
    /// answered on the response characteristic: see `Link::open`.
    Duplicate,
    /// The phone has [`MAX_STREAMS`] open.
    TooMany,
    /// The phone is gone.
    Gone,
}

impl Refused {
    /// The refusal as the status the phone is sent; a phone that is gone is
    /// sent nothing.
    pub(crate) fn status(self, id: u32) -> Option<Status> {
        match self {
            Refused::Duplicate => Some(Status::already_exists(format!(
                "stream {id} is already open"
            ))),
            Refused::TooMany => Some(Status::resource_exhausted(format!(
                "{MAX_STREAMS} streams are open already"
            ))),
            Refused::Gone => None,
        }
    }
}

/// The last [`Response`] of stream `id`, encoded: its status and no more.
pub(crate) fn ending(id: u32, status: &Status) -> Vec<u8> {
    Response {
        id,
        code: status.code() as u32,
        message: status.message().to_string(),
        payload: Vec::new(),
        more: false,
    }
    .encode_to_vec()
}

/// Why a stream was stopped from the phone's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// The phone cancelled it: it is told so in a last response.
    Cancelled,
    /// Nobody is left to tell: the phone went, nobody is subscribed, or the
    /// controller stopped.
    Dropped,
}

/// A message waiting in a slot, as the encoded [`Response`] it goes out as.
struct Waiting {
    frame: Vec<u8>,
    replaceable: bool,
}

#[derive(Default)]
struct SlotState {
    /// The messages waiting, oldest first: at most one that is not
    /// replaceable, and after it at most one that is.
    waiting: VecDeque<Waiting>,
    /// The stream's last response, once it has one. It goes out after what
    /// waits.
    ending: Option<Vec<u8>>,
    stop: Option<Stop>,
}

impl SlotState {
    fn kept(&self) -> usize {
        self.waiting.iter().filter(|w| !w.replaceable).count()
    }
}

/// One open stream's buffer between the service and the radio.
pub(crate) struct Slot {
    peer: PeerId,
    id: u32,
    state: parking_lot::Mutex<SlotState>,
    /// Told when the pump takes a message, and when the stream is stopped:
    /// what the reader waits on.
    changed: Notify,
}

impl Slot {
    /// The id the stream was opened with.
    pub(crate) fn id(&self) -> u32 {
        self.id
    }

    /// Takes a replaceable message, in place of the one waiting if one is.
    fn replace(&self, frame: Vec<u8>) {
        let mut state = self.state.lock();
        if state.stop.is_some() {
            return;
        }
        match state.waiting.back_mut() {
            Some(last) if last.replaceable => last.frame = frame,
            _ => state.waiting.push_back(Waiting {
                frame,
                replaceable: true,
            }),
        }
    }

    /// Takes a message that must arrive, once the one before it has gone to
    /// the radio. The replaceable message waiting, if any, is older than
    /// this and is discarded. False when the stream was stopped instead.
    async fn keep(&self, frame: Vec<u8>) -> bool {
        loop {
            let changed = self.changed.notified();
            {
                let mut state = self.state.lock();
                if state.stop.is_some() {
                    return false;
                }
                if state.kept() == 0 {
                    state.waiting.clear();
                    state.waiting.push_back(Waiting {
                        frame,
                        replaceable: false,
                    });
                    return true;
                }
            }
            changed.await;
        }
    }

    /// Ends the stream with its last response. A stream nobody is left to
    /// tell has none.
    pub(crate) fn finish(&self, ending: Vec<u8>) {
        let mut state = self.state.lock();
        if state.stop != Some(Stop::Dropped) {
            state.ending = Some(ending);
        }
    }

    /// Stops the stream: what waits is discarded, and the reader, told,
    /// drops the service's side. False if it was over already.
    fn stop(&self, stop: Stop) -> bool {
        {
            let mut state = self.state.lock();
            match stop {
                // The service ended it first, or the phone cancelled twice.
                Stop::Cancelled if state.stop.is_some() || state.ending.is_some() => return false,
                Stop::Cancelled => {}
                Stop::Dropped => state.ending = None,
            }
            state.stop = Some(stop);
            state.waiting.clear();
        }
        self.changed.notify_waiters();
        true
    }

    /// Resolves when the stream is stopped.
    async fn stopped(&self) -> Stop {
        loop {
            let changed = self.changed.notified();
            if let Some(stop) = self.state.lock().stop {
                return stop;
            }
            changed.await;
        }
    }

    /// The next thing to send, and whether it is the last.
    fn take(&self) -> Option<(Vec<u8>, bool)> {
        let mut state = self.state.lock();
        if let Some(next) = state.waiting.pop_front() {
            drop(state);
            self.changed.notify_waiters();
            return Some((next.frame, false));
        }
        state.ending.take().map(|ending| (ending, true))
    }

    fn has_something(&self) -> bool {
        let state = self.state.lock();
        !state.waiting.is_empty() || state.ending.is_some()
    }
}

#[derive(Default)]
struct OutboxState {
    /// The open streams. One is open from the request that asked for it
    /// until its last response is taken for sending, or it is dropped.
    slots: Vec<Arc<Slot>>,
    /// Refusals of streams that were never opened, each a last response.
    notices: VecDeque<Vec<u8>>,
    /// Where the pump looks first, so that streams take turns.
    turn: usize,
    closed: bool,
}

/// Everything waiting to go out on the `stream` characteristic.
#[derive(Default)]
pub(crate) struct Outbox {
    state: parking_lot::Mutex<OutboxState>,
    /// Told when something is put in for the pump.
    ready: Notify,
    /// How many messages readers have put into slots.
    #[cfg(test)]
    accepted: std::sync::atomic::AtomicUsize,
}

impl Outbox {
    /// Opens stream `id` for `peer`, which is alive as long as `alive`
    /// says: the slot its reader fills.
    pub(crate) fn open(
        &self,
        peer: PeerId,
        id: u32,
        alive: &AtomicBool,
    ) -> Result<Arc<Slot>, Refused> {
        let mut state = self.state.lock();
        // Read under the lock `drop_peer` takes after the flag is cleared:
        // either this sees the phone gone, or that finds the slot.
        if state.closed || !alive.load(Ordering::Acquire) {
            return Err(Refused::Gone);
        }
        let open = || state.slots.iter().filter(|slot| slot.peer == peer);
        if open().any(|slot| slot.id == id) {
            return Err(Refused::Duplicate);
        }
        if open().count() >= MAX_STREAMS {
            return Err(Refused::TooMany);
        }
        let slot = Arc::new(Slot {
            peer,
            id,
            state: parking_lot::Mutex::new(SlotState::default()),
            changed: Notify::new(),
        });
        state.slots.push(slot.clone());
        Ok(slot)
    }

    /// The phone cancelled its stream `id`. False, and nothing happens, if
    /// it has no such stream, or the stream is already over.
    pub(crate) fn cancel(&self, peer: PeerId, id: u32) -> bool {
        let slot = {
            let state = self.state.lock();
            state
                .slots
                .iter()
                .find(|slot| slot.peer == peer && slot.id == id)
                .cloned()
        };
        slot.is_some_and(|slot| slot.stop(Stop::Cancelled))
    }

    /// The phone is gone: its streams end, with nobody to tell.
    pub(crate) fn drop_peer(&self, peer: PeerId) {
        self.drop_where(|slot| slot.peer == peer);
    }

    /// Every stream ends, with nobody to tell: nobody is subscribed.
    fn drop_all(&self) {
        self.drop_where(|_| true);
        self.state.lock().notices.clear();
    }

    /// The controller is stopping: every stream ends and none opens.
    pub(crate) fn close(&self) {
        self.state.lock().closed = true;
        self.drop_all();
    }

    fn drop_where(&self, gone: impl Fn(&Slot) -> bool) {
        let dropped: Vec<Arc<Slot>> = {
            let mut state = self.state.lock();
            let (dropped, kept): (Vec<_>, Vec<_>) =
                state.slots.drain(..).partition(|slot| gone(slot));
            state.slots = kept;
            dropped
        };
        for slot in dropped {
            slot.stop(Stop::Dropped);
        }
    }

    /// Queues the refusal of a stream that was not opened.
    pub(crate) fn notice(&self, ending: Vec<u8>) {
        {
            let mut state = self.state.lock();
            if state.closed || state.notices.len() >= NOTICE_DEPTH {
                tracing::debug!("Bluetooth stream refusal dropped: too many waiting");
                return;
            }
            state.notices.push_back(ending);
        }
        self.ready.notify_one();
    }

    /// Tells the pump a slot has something for it.
    pub(crate) fn wake(&self) {
        self.ready.notify_one();
    }

    /// How many streams are open.
    #[cfg(test)]
    pub(crate) fn open_count(&self) -> usize {
        self.state.lock().slots.len()
    }

    /// How many messages readers have put into slots, replaced or not.
    #[cfg(test)]
    pub(crate) fn accepted(&self) -> usize {
        self.accepted.load(Ordering::Acquire)
    }

    fn has_something(&self) -> bool {
        let state = self.state.lock();
        !state.notices.is_empty() || state.slots.iter().any(|slot| slot.has_something())
    }

    /// Resolves when there is something to send.
    async fn something(&self) {
        loop {
            let ready = self.ready.notified();
            if self.has_something() {
                return;
            }
            ready.await;
        }
    }

    /// The next frame to send: a refusal, or the next of a stream's, the
    /// streams taking turns. A stream whose last response this is, is no
    /// longer open.
    fn take(&self) -> Option<Vec<u8>> {
        let mut state = self.state.lock();
        if let Some(notice) = state.notices.pop_front() {
            return Some(notice);
        }
        let count = state.slots.len();
        for offset in 0..count {
            let index = (state.turn + offset) % count;
            if let Some((frame, last)) = state.slots[index].take() {
                if last {
                    state.slots.remove(index);
                    state.turn = index;
                } else {
                    state.turn = index + 1;
                }
                return Some(frame);
            }
        }
        None
    }
}

/// Reads the service's side of one stream into its slot until the stream
/// ends, by itself or because it was stopped, and then drops it. The reader
/// is the only holder of `messages`, so its end is the stream's in the
/// player.
pub(crate) async fn read(
    outbox: Arc<Outbox>,
    slot: Arc<Slot>,
    mut messages: Box<dyn Messages>,
    replaceable: Replaceable,
) {
    let id = slot.id();
    let cancelled = || Status::cancelled("the stream was cancelled");
    let said = |stop: Stop| (stop == Stop::Cancelled).then(cancelled);
    let status = loop {
        let step = tokio::select! {
            biased;
            stop = slot.stopped() => break said(stop),
            step = messages.next() => step,
        };
        let payload = match step {
            Step::Message(payload) => payload,
            Step::End(status) => break Some(status),
        };
        let replaceable = replaceable(&payload);
        let frame = Response {
            id,
            code: Code::Ok as u32,
            message: String::new(),
            payload,
            more: true,
        }
        .encode_to_vec();
        if frame.len() > MAX_FRAME {
            break Some(Status::resource_exhausted(format!(
                "a stream message of {} bytes is too large for Bluetooth",
                frame.len()
            )));
        }
        if replaceable {
            slot.replace(frame);
        } else if !slot.keep(frame).await {
            break said(slot.stopped().await);
        }
        #[cfg(test)]
        outbox.accepted.fetch_add(1, Ordering::AcqRel);
        outbox.wake();
    };
    // The service's stream ends here, before the phone is told.
    drop(messages);
    if let Some(status) = status {
        slot.finish(ending(id, &status));
        outbox.wake();
    }
}

/// Holds the pump to a rate in bytes a second.
struct Pacer {
    /// The time a byte takes; `None` for no limit.
    per_byte: Option<Duration>,
    /// When the next chunk may go.
    next: Instant,
}

impl Pacer {
    fn new(rate: Option<u32>) -> Pacer {
        Pacer {
            per_byte: rate.map(|rate| Duration::from_secs(1) / rate.max(1)),
            next: Instant::now(),
        }
    }

    /// Returns when the next chunk may go.
    async fn ready(&self) {
        if self.per_byte.is_some() && self.next > Instant::now() {
            tokio::time::sleep_until(self.next).await;
        }
    }

    /// A chunk of `len` bytes went.
    fn sent(&mut self, len: usize) {
        if let Some(per_byte) = self.per_byte {
            let floor = Instant::now().checked_sub(PACER_SLACK).unwrap_or(self.next);
            self.next = self.next.max(floor) + per_byte * len as u32;
        }
    }
}

/// Resolves when any of `live` is ended by its phone; never, with none.
async fn any_stopped<N: Notifier>(live: &[N]) {
    if live.is_empty() {
        return std::future::pending().await;
    }
    futures_util::future::select_all(live.iter().map(|session| session.stopped())).await;
}

/// Sends what is in `outbox` to the phones whose sessions arrive on
/// `sessions`: a chunk at a time, each cut to what `chunk_size` says when
/// its frame begins, none while a control characteristic has something
/// waiting, and no faster than `rate` bytes a second. With nobody
/// subscribed there is nobody to stream to, and every open stream is
/// ended. Ends when `sessions` closes.
pub(crate) async fn run<N: Notifier>(
    mut sessions: mpsc::UnboundedReceiver<N>,
    outbox: Arc<Outbox>,
    control: Arc<Control>,
    chunk_size: impl Fn() -> usize + Send,
    rate: Option<u32>,
) {
    let mut live: Vec<N> = Vec::new();
    let mut pacer = Pacer::new(rate);
    loop {
        tokio::select! {
            session = sessions.recv() => {
                let Some(session) = session else { return };
                live.push(session);
                continue;
            }
            () = any_stopped(&live) => {
                live.retain(|session| !session.is_stopped());
                if live.is_empty() {
                    outbox.drop_all();
                }
                continue;
            }
            () = outbox.something() => {}
        }
        // The turn is waited for before the frame is taken: until then a
        // newer one can still take its place.
        control.clear().await;
        pacer.ready().await;
        let Some(frame) = outbox.take() else { continue };
        let Ok(chunks) = chunks(&frame, chunk_size()) else {
            continue;
        };
        for (index, chunk) in chunks.into_iter().enumerate() {
            if index > 0 {
                control.clear().await;
                pacer.ready().await;
            }
            // A phone that subscribed and then asked is heard, whichever of
            // the two this task learned of first.
            while let Ok(session) = sessions.try_recv() {
                live.push(session);
            }
            let len = chunk.len();
            if !send_chunk(&mut live, chunk).await {
                outbox.drop_all();
                break;
            }
            pacer.sent(len);
        }
    }
}

#[cfg(test)]
mod tests;
