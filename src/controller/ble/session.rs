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

//! The link between phones and the player: everything the radio stack
//! hands over or takes back, with no stack types in it. A write is a chunk
//! of some peer's frame; a frame is a request, answered by a response cut
//! back into chunks. The status goes the same way, a frame of its own.
//!
//! # Tasking
//!
//! The order of a frame's chunks matters, and the order of two peers'
//! requests does not, so the two halves of a write are handled apart.
//! [`Link::submit`] reassembles synchronously, on the caller, in the order
//! the stack delivers the writes. A completed frame is queued to a worker
//! task of its peer's own, which answers a peer's requests one at a time in
//! the order they arrived and sends the response to the pump that notifies
//! the phones. A slow call therefore holds up only the peer that made it:
//! other peers have workers of their own, the status stream is polled by a
//! task of its own, and the write that carried the request returns at once
//! instead of holding the radio's write response until the player answers.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Instant;

use futures_util::future::BoxFuture;
use prost::Message;
use tokio::sync::{mpsc, Notify};

use crate::proto::ble::v1::{Request, Status};

use super::codec::{chunks, Reassembler};
use super::dispatch::{failure, Dispatcher};
use super::status::StatusFeed;

/// A central's Bluetooth address, which is all the stack tells us of it.
pub(crate) type PeerId = [u8; 6];

/// The ATT MTU every link starts at, before a phone negotiates more.
pub(crate) const DEFAULT_MTU: u16 = 23;

/// How many peers the link tracks at once. A phone not heard from for the
/// longest makes room for a new one, so a stream of addresses cannot grow
/// the table.
pub(crate) const MAX_PEERS: usize = 8;

/// The bytes ATT keeps of each packet: an opcode and a handle.
const ATT_OVERHEAD: usize = 3;
/// An attribute value never exceeds this, whatever the MTU.
const MAX_CHUNK: usize = 512;
/// The smallest chunk [`chunks`] can cut any frame into: a header, a
/// three-byte length and a byte of the frame.
const MIN_CHUNK: usize = 5;

/// Requests a peer may have waiting behind the one being answered. A phone
/// pipelining more than this is not waiting for its answers, and the
/// excess is dropped.
const QUEUE_DEPTH: usize = 8;

/// A bound on responses waiting for the notification pump.
pub(crate) const PUMP_DEPTH: usize = 64;

/// The gRPC code of a response too large to send.
const RESOURCE_EXHAUSTED: tonic::Code = tonic::Code::ResourceExhausted;

/// Formats an address as BlueZ and the phones show it.
pub(crate) fn format_address(address: PeerId) -> String {
    address
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Reads an address in the form [`format_address`] writes, in either case.
pub(crate) fn parse_address(text: &str) -> Option<PeerId> {
    let mut address = [0u8; 6];
    let mut parts = text.split(':');
    for byte in &mut address {
        let part = parts.next()?;
        if part.len() != 2 || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        *byte = u8::from_str_radix(part, 16).ok()?;
    }
    parts.next().is_none().then_some(address)
}

/// Answers a request frame with a response frame.
pub(crate) trait Handler: Send + Sync + 'static {
    fn handle<'a>(&'a self, frame: &'a [u8]) -> BoxFuture<'a, Vec<u8>>;
}

impl Handler for Dispatcher {
    fn handle<'a>(&'a self, frame: &'a [u8]) -> BoxFuture<'a, Vec<u8>> {
        Box::pin(Dispatcher::handle(self, frame))
    }
}

/// What the link knows of one phone.
struct Peer {
    id: PeerId,
    /// The ATT MTU it last wrote or read with.
    mtu: u16,
    reassembler: Reassembler,
    /// When it was last heard from, as a count, for evicting the oldest.
    touched: u64,
    /// Frames waiting for its worker, once it has one.
    queue: Option<mpsc::Sender<Vec<u8>>>,
    /// Cleared when the peer goes, so its worker drops what is queued.
    alive: Arc<AtomicBool>,
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

#[derive(Default)]
struct Peers {
    peers: Vec<Peer>,
    clock: u64,
}

impl Peers {
    /// The peer `id`, added if it is new, which makes room if it must.
    /// Heard from just now.
    fn touch(&mut self, id: PeerId, mtu: Option<u16>) -> &mut Peer {
        self.clock += 1;
        let clock = self.clock;
        let index = match self.peers.iter().position(|p| p.id == id) {
            Some(index) => index,
            None => {
                if self.peers.len() >= MAX_PEERS {
                    if let Some(oldest) =
                        (0..self.peers.len()).min_by_key(|&i| self.peers[i].touched)
                    {
                        self.peers.swap_remove(oldest);
                    }
                }
                self.peers.push(Peer {
                    id,
                    mtu: DEFAULT_MTU,
                    reassembler: Reassembler::new(),
                    touched: clock,
                    queue: None,
                    alive: Arc::new(AtomicBool::new(true)),
                });
                self.peers.len() - 1
            }
        };
        let peer = &mut self.peers[index];
        peer.touched = clock;
        if let Some(mtu) = mtu {
            peer.mtu = mtu;
        }
        peer
    }
}

/// The phones' side of the player: its requests in, its responses and its
/// status out.
pub(crate) struct Link<H: Handler> {
    handler: H,
    peers: parking_lot::Mutex<Peers>,
    status: parking_lot::Mutex<StatusFeed>,
    /// Response frames, as chunks, for the pump that notifies them.
    responses: mpsc::Sender<Vec<Vec<u8>>>,
    /// Status frames, as chunks, for the pump that notifies them.
    status_out: mpsc::Sender<Vec<Vec<u8>>>,
    /// Told after each request is answered: the status poll looks at once,
    /// so a phone sees what its call changed without waiting for a tick.
    status_wanted: Arc<Notify>,
}

impl<H: Handler> Link<H> {
    pub(crate) fn new(
        handler: H,
        responses: mpsc::Sender<Vec<Vec<u8>>>,
        status_out: mpsc::Sender<Vec<Vec<u8>>>,
    ) -> Arc<Link<H>> {
        Arc::new(Link {
            handler,
            peers: parking_lot::Mutex::new(Peers::default()),
            status: parking_lot::Mutex::new(StatusFeed::default()),
            responses,
            status_out,
            status_wanted: Arc::new(Notify::new()),
        })
    }

    /// Told after each answered request; see [`super::status::poll`].
    pub(crate) fn status_wanted(&self) -> Arc<Notify> {
        self.status_wanted.clone()
    }

    /// A phone connected. It counts toward the chunk size from now on, at
    /// the default MTU until it says otherwise.
    pub(crate) fn on_connect(&self, peer: PeerId) {
        self.peers.lock().touch(peer, None);
    }

    /// A phone disconnected: its state, a partly received frame included, is
    /// dropped, and so is any request of its still waiting.
    pub(crate) fn on_disconnect(&self, peer: PeerId) {
        self.peers.lock().peers.retain(|p| p.id != peer);
    }

    /// How many phones the link holds state for.
    #[cfg(test)]
    pub(crate) fn peer_count(&self) -> usize {
        self.peers.lock().peers.len()
    }

    /// The bytes of each chunk sent: what the smallest MTU of the phones
    /// known leaves of an ATT packet. A notification goes to every
    /// subscriber, so it must fit the smallest.
    pub(crate) fn chunk_size(&self) -> usize {
        let mtu = self
            .peers
            .lock()
            .peers
            .iter()
            .map(|p| p.mtu)
            .min()
            .unwrap_or(DEFAULT_MTU);
        usize::from(mtu)
            .saturating_sub(ATT_OVERHEAD)
            .clamp(MIN_CHUNK, MAX_CHUNK)
    }

    /// Takes one chunk written by `peer`: the frame, when it completes one.
    /// A chunk that cannot be part of a frame is logged at debug and dropped
    /// with the frame it broke, and the peer starts over.
    fn push(&self, peer: PeerId, mtu: u16, chunk: &[u8]) -> Option<Vec<u8>> {
        let mut peers = self.peers.lock();
        match peers.touch(peer, Some(mtu)).reassembler.push(chunk) {
            Ok(frame) => frame,
            Err(e) => {
                tracing::debug!(peer = %format_address(peer), error = %e, "Bluetooth frame dropped");
                None
            }
        }
    }

    /// Answers one request frame with the chunks of its response.
    async fn respond(&self, frame: &[u8]) -> Vec<Vec<u8>> {
        let response = self.handler.handle(frame).await;
        // Whatever the call changed, the phone hears of it now.
        self.status_wanted.notify_one();
        let size = self.chunk_size();
        match chunks(&response, size) {
            Ok(chunks) => chunks,
            Err(e) => {
                // Too large for a frame: say so, in a frame that fits.
                let id = Request::decode(frame).map(|r| r.id).unwrap_or(0);
                let refusal = failure(id, RESOURCE_EXHAUSTED, format!("response not sent: {e}"));
                chunks(&refusal, size).unwrap_or_default()
            }
        }
    }

    /// Takes one chunk written by `peer` and answers it: the chunks to send
    /// back on the response characteristic, when the chunk completed a
    /// request. [`Link::submit`] is this with the answer queued instead of
    /// waited for, which is what the radio glue needs; this is the whole of
    /// the exchange in one call, for tests.
    #[cfg(test)]
    pub(crate) async fn on_write(&self, peer: PeerId, mtu: u16, chunk: &[u8]) -> Vec<Vec<u8>> {
        match self.push(peer, mtu, chunk) {
            Some(frame) => self.respond(&frame).await,
            None => Vec::new(),
        }
    }

    /// Takes one chunk written by `peer`. A request it completes is queued
    /// for the peer's worker, and its response goes to the response pump
    /// when the worker has it. Returns without waiting for either.
    pub(crate) fn submit(self: &Arc<Self>, peer: PeerId, mtu: u16, chunk: &[u8]) {
        let Some(frame) = self.push(peer, mtu, chunk) else {
            return;
        };
        let queue = {
            let mut peers = self.peers.lock();
            let entry = peers.touch(peer, None);
            if entry.queue.is_none() {
                let (queue, waiting) = mpsc::channel(QUEUE_DEPTH);
                tokio::spawn(work(
                    Arc::downgrade(self),
                    entry.alive.clone(),
                    waiting,
                    self.responses.clone(),
                ));
                entry.queue = Some(queue);
            }
            entry.queue.clone()
        };
        if let Some(queue) = queue {
            if queue.try_send(frame).is_err() {
                tracing::debug!(peer = %format_address(peer), "Bluetooth request dropped: too many waiting");
            }
        }
    }

    /// A phone read the status characteristic: the latest status, from
    /// `offset`, as a long read asks for it.
    pub(crate) fn status_read(&self, peer: PeerId, mtu: u16, offset: usize) -> Vec<u8> {
        self.peers.lock().touch(peer, Some(mtu));
        let status = self.status.lock();
        status.latest().get(offset..).unwrap_or_default().to_vec()
    }

    /// The latest status as chunks, for a phone that has just subscribed.
    pub(crate) fn status_chunks(&self) -> Vec<Vec<u8>> {
        let size = self.chunk_size();
        let status = self.status.lock();
        if status.latest().is_empty() {
            return Vec::new();
        }
        chunks(status.latest(), size).unwrap_or_default()
    }

    /// Takes the player's status as just polled. It is what a read returns
    /// from now on, and goes to the status pump if it is due.
    pub(crate) fn publish_status(&self, now: Instant, status: Status) {
        let encoded = self.status.lock().offer(now, status);
        let Some(encoded) = encoded else {
            return;
        };
        let sent = chunks(&encoded, self.chunk_size())
            .ok()
            .is_some_and(|chunks| self.status_out.try_send(chunks).is_ok());
        if !sent {
            self.status.lock().unsend();
        }
    }
}

/// Answers one peer's requests, one at a time, in order.
async fn work<H: Handler>(
    link: Weak<Link<H>>,
    alive: Arc<AtomicBool>,
    mut waiting: mpsc::Receiver<Vec<u8>>,
    responses: mpsc::Sender<Vec<Vec<u8>>>,
) {
    while let Some(frame) = waiting.recv().await {
        // A phone that left no longer wants what it asked for.
        if !alive.load(Ordering::Acquire) {
            break;
        }
        let Some(link) = link.upgrade() else {
            break;
        };
        let answer = link.respond(&frame).await;
        drop(link);
        if !alive.load(Ordering::Acquire) || responses.send(answer).await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::controller::ble::codec::MAX_FRAME;
    use crate::proto::ble::v1::Response;

    /// Answers a request with its payload, after the delay its first byte
    /// asks for in milliseconds.
    struct Echo;

    impl Handler for Echo {
        fn handle<'a>(&'a self, frame: &'a [u8]) -> BoxFuture<'a, Vec<u8>> {
            Box::pin(async move {
                let request = Request::decode(frame).expect("a request");
                if let Some(&ms) = request.payload.first() {
                    tokio::time::sleep(Duration::from_millis(u64::from(ms))).await;
                }
                Response {
                    id: request.id,
                    code: 0,
                    message: String::new(),
                    payload: request.payload,
                }
                .encode_to_vec()
            })
        }
    }

    /// Answers every request with a payload of the given size.
    struct Big(usize);

    impl Handler for Big {
        fn handle<'a>(&'a self, frame: &'a [u8]) -> BoxFuture<'a, Vec<u8>> {
            Box::pin(async move {
                let request = Request::decode(frame).expect("a request");
                Response {
                    id: request.id,
                    code: 0,
                    message: String::new(),
                    payload: vec![7; self.0],
                }
                .encode_to_vec()
            })
        }
    }

    type Pumps = (mpsc::Receiver<Vec<Vec<u8>>>, mpsc::Receiver<Vec<Vec<u8>>>);

    fn link<H: Handler>(handler: H) -> (Arc<Link<H>>, Pumps) {
        let (responses, response_rx) = mpsc::channel(PUMP_DEPTH);
        let (status_out, status_rx) = mpsc::channel(PUMP_DEPTH);
        (
            Link::new(handler, responses, status_out),
            (response_rx, status_rx),
        )
    }

    fn peer(n: u8) -> PeerId {
        [0xAA, 0, 0, 0, 0, n]
    }

    fn request(id: u32, payload: Vec<u8>) -> Vec<u8> {
        Request {
            id,
            method: "Play".into(),
            payload,
        }
        .encode_to_vec()
    }

    fn reassemble(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut reassembler = Reassembler::new();
        let mut frame = None;
        for chunk in chunks {
            frame = reassembler.push(chunk).expect("chunk");
        }
        frame.expect("a whole frame")
    }

    fn response_id(chunks: &[Vec<u8>]) -> u32 {
        Response::decode(reassemble(chunks).as_slice())
            .expect("a response")
            .id
    }

    #[tokio::test]
    async fn a_request_in_one_chunk_is_answered_in_chunks() {
        let (link, _pumps) = link(Echo);
        let frame = request(9, vec![0; 60]);
        let written = chunks(&frame, 20).unwrap();
        assert!(written.len() > 1);

        let mut answer = Vec::new();
        for chunk in &written {
            answer = link.on_write(peer(1), 23, chunk).await;
        }
        let response = Response::decode(reassemble(&answer).as_slice()).unwrap();
        assert_eq!(response.id, 9);
        assert_eq!(response.payload.len(), 60);
        for chunk in &answer {
            assert!(chunk.len() <= 20, "chunk of {}", chunk.len());
        }
    }

    #[tokio::test]
    async fn an_answered_request_asks_for_the_status_at_once() {
        let (link, _pumps) = link(Echo);
        let wanted = link.status_wanted();
        let good = chunks(&request(5, vec![]), 20).unwrap();
        let answer = link.on_write(peer(1), 23, &good[0]).await;
        assert_eq!(response_id(&answer), 5);
        // The permit is already there: this does not wait for a tick.
        tokio::time::timeout(Duration::from_millis(200), wanted.notified())
            .await
            .expect("the status poll was not told");
    }

    #[tokio::test]
    async fn a_bad_chunk_resets_that_peer_and_answers_nothing() {
        let (link, _pumps) = link(Echo);
        // A continuation with no start, then a good request.
        assert!(link.on_write(peer(1), 23, &[0x00, 1, 2]).await.is_empty());
        assert!(link.on_write(peer(1), 23, &[]).await.is_empty());
        assert!(link.on_write(peer(1), 23, &[0x7F]).await.is_empty());

        let good = chunks(&request(3, vec![]), 20).unwrap();
        let answer = link.on_write(peer(1), 23, &good[0]).await;
        assert_eq!(response_id(&answer), 3);

        // A frame broken half way starts over with the next start.
        let long = chunks(&request(4, vec![0; 60]), 20).unwrap();
        assert!(link.on_write(peer(1), 23, &long[0]).await.is_empty());
        assert!(link.on_write(peer(1), 23, &[0x7F]).await.is_empty());
        assert!(link.on_write(peer(1), 23, &long[1]).await.is_empty());
    }

    #[tokio::test]
    async fn peers_reassemble_apart() {
        let (link, _pumps) = link(Echo);
        let a = chunks(&request(1, vec![0; 50]), 20).unwrap();
        let b = chunks(&request(2, vec![0; 50]), 20).unwrap();
        assert_eq!(a.len(), b.len());
        let mut answers = Vec::new();
        for (x, y) in a.iter().zip(&b) {
            answers.push(link.on_write(peer(1), 23, x).await);
            answers.push(link.on_write(peer(2), 23, y).await);
        }
        let done: Vec<u32> = answers
            .iter()
            .filter(|a| !a.is_empty())
            .map(|a| response_id(a))
            .collect();
        assert_eq!(done, vec![1, 2]);
    }

    #[tokio::test]
    async fn a_disconnect_drops_a_partial_frame() {
        let (link, _pumps) = link(Echo);
        let written = chunks(&request(1, vec![0; 50]), 20).unwrap();
        link.on_write(peer(1), 23, &written[0]).await;
        link.on_disconnect(peer(1));
        assert_eq!(link.peer_count(), 0);
        // The rest of it, on a new connection, is not a frame.
        assert!(link.on_write(peer(1), 23, &written[1]).await.is_empty());
    }

    #[tokio::test]
    async fn chunks_fit_the_smallest_mtu_known() {
        let (link, _pumps) = link(Echo);
        assert_eq!(link.chunk_size(), 20);

        link.on_write(peer(1), 517, &[]).await;
        assert_eq!(link.chunk_size(), 512);
        link.on_write(peer(2), 185, &[]).await;
        assert_eq!(link.chunk_size(), 182);
        // A phone that has only connected counts at the default.
        link.on_connect(peer(3));
        assert_eq!(link.chunk_size(), 20);
        link.on_disconnect(peer(3));
        assert_eq!(link.chunk_size(), 182);
        link.on_disconnect(peer(2));
        assert_eq!(link.chunk_size(), 512);
        link.on_disconnect(peer(1));
        assert_eq!(link.chunk_size(), 20);

        // Nothing makes it too small to cut a frame into.
        link.on_write(peer(4), 3, &[]).await;
        assert_eq!(link.chunk_size(), MIN_CHUNK);
        let frame = request(1, vec![5; 100]);
        assert!(chunks(&frame, link.chunk_size()).is_ok());
    }

    #[tokio::test]
    async fn the_peer_table_is_bounded_and_drops_the_one_heard_longest_ago() {
        let (link, _pumps) = link(Echo);
        for n in 0..MAX_PEERS as u8 {
            link.on_connect(peer(n));
        }
        // Peer 0 speaks, so peer 1 is the oldest.
        link.on_write(peer(0), 23, &[]).await;
        link.on_connect(peer(100));
        assert_eq!(link.peer_count(), MAX_PEERS);

        // Peer 1 lost its place: a half frame it sent is gone.
        let ids: Vec<PeerId> = link.peers.lock().peers.iter().map(|p| p.id).collect();
        assert!(!ids.contains(&peer(1)));
        assert!(ids.contains(&peer(0)));
        assert!(ids.contains(&peer(100)));

        // A flood of addresses leaves the table the same size.
        for n in 0..=255u8 {
            link.on_connect([0xBB, 0, 0, 0, 0, n]);
        }
        assert_eq!(link.peer_count(), MAX_PEERS);
    }

    #[tokio::test]
    async fn a_response_too_large_for_a_frame_is_refused_in_one_that_fits() {
        let (link, _pumps) = link(Big(MAX_FRAME + 1));
        let frame = chunks(&request(77, vec![]), 20).unwrap();
        let answer = link.on_write(peer(1), 23, &frame[0]).await;
        let response = Response::decode(reassemble(&answer).as_slice()).unwrap();
        assert_eq!(response.id, 77);
        assert_eq!(response.code, RESOURCE_EXHAUSTED as u32);
        assert!(response.payload.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_slow_call_holds_up_only_its_own_peer() {
        let (link, (mut responses, _status)) = link(Echo);
        // Peer 1 asks for something that takes 400 ms; peer 2 asks after.
        let slow = chunks(&request(1, vec![200, 0, 0]), 20).unwrap();
        let quick = chunks(&request(2, vec![]), 20).unwrap();
        let started = Instant::now();
        for chunk in &slow {
            link.submit(peer(1), 23, chunk);
        }
        for chunk in &quick {
            link.submit(peer(2), 23, chunk);
        }
        // Submitting did not wait for the slow call.
        assert!(started.elapsed() < Duration::from_millis(150));

        let first = responses.recv().await.expect("a response");
        assert_eq!(response_id(&first), 2);
        let second = responses.recv().await.expect("a response");
        assert_eq!(response_id(&second), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn one_peers_requests_are_answered_in_order() {
        let (link, (mut responses, _status)) = link(Echo);
        // The first takes longer than the rest: they wait for it.
        for (id, delay) in [(1u32, 60u8), (2, 0), (3, 0), (4, 10)] {
            let frame = request(id, vec![delay]);
            for chunk in chunks(&frame, 20).unwrap() {
                link.submit(peer(1), 23, &chunk);
            }
        }
        let mut ids = Vec::new();
        for _ in 0..4 {
            ids.push(response_id(&responses.recv().await.expect("a response")));
        }
        assert_eq!(ids, vec![1, 2, 3, 4]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_peer_that_left_gets_no_answer_to_what_it_left_waiting() {
        let (link, (mut responses, _status)) = link(Echo);
        for id in [1u32, 2] {
            for chunk in chunks(&request(id, vec![100]), 20).unwrap() {
                link.submit(peer(1), 23, &chunk);
            }
        }
        link.on_disconnect(peer(1));
        // Neither is answered: the first was in flight and is dropped when
        // done, the second never starts.
        let answered = tokio::time::timeout(Duration::from_millis(400), responses.recv()).await;
        assert!(answered.is_err(), "{answered:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_peer_with_too_many_requests_waiting_loses_the_excess() {
        let (link, (mut responses, _status)) = link(Echo);
        let total = QUEUE_DEPTH as u32 + 5;
        // The first holds the worker while the rest queue behind it.
        for id in 0..total {
            let delay = if id == 0 { 100 } else { 0 };
            for chunk in chunks(&request(id, vec![delay]), 20).unwrap() {
                link.submit(peer(1), 23, &chunk);
            }
        }
        let mut answered = 0;
        while tokio::time::timeout(Duration::from_millis(500), responses.recv())
            .await
            .is_ok()
        {
            answered += 1;
        }
        assert!(answered >= 1 && answered < total as usize, "{answered}");
    }

    #[test]
    fn addresses_format_and_parse() {
        let address = [0x00, 0x1A, 0x7D, 0xDA, 0x71, 0x13];
        assert_eq!(format_address(address), "00:1A:7D:DA:71:13");
        assert_eq!(parse_address("00:1A:7D:DA:71:13"), Some(address));
        assert_eq!(parse_address("00:1a:7d:da:71:13"), Some(address));
        for bad in [
            "",
            "00:1A:7D:DA:71",
            "00:1A:7D:DA:71:13:00",
            "00:1A:7D:DA:71:1",
            "00:1A:7D:DA:71:GG",
            "001A7DDA7113",
            "+0:1A:7D:DA:71:13",
        ] {
            assert_eq!(parse_address(bad), None, "{bad:?}");
        }
    }

    fn status(playing: bool, elapsed: i64) -> Status {
        Status {
            playlist_name: "Set".into(),
            song_name: "Song".into(),
            song_duration: None,
            playing,
            elapsed: Some(prost_types::Duration {
                seconds: elapsed,
                nanos: 0,
            }),
            position: None,
            auto_play: None,
            loop_section: None,
            gains_revision: None,
            ambient_mode: None,
            ambient_state: None,
        }
    }

    #[tokio::test]
    async fn status_reads_the_latest_poll_from_an_offset_and_pushes_when_due() {
        let (link, (_responses, mut pushed)) = link(Echo);
        let start = Instant::now();
        assert!(link.status_read(peer(1), 23, 0).is_empty());
        assert!(link.status_chunks().is_empty());

        link.publish_status(start, status(false, 0));
        let first = pushed.try_recv().expect("the first status is pushed");
        assert_eq!(
            Status::decode(reassemble(&first).as_slice()).unwrap(),
            status(false, 0)
        );

        // Nothing new: not pushed, but a read has the poll.
        link.publish_status(start + Duration::from_secs(1), status(false, 0));
        assert!(pushed.try_recv().is_err());

        link.publish_status(start + Duration::from_secs(2), status(true, 5));
        assert!(pushed.try_recv().is_ok());

        let whole = link.status_read(peer(1), 23, 0);
        assert_eq!(Status::decode(whole.as_slice()).unwrap(), status(true, 5));
        // A long read continues where the last stopped, and past the end it
        // is empty.
        assert_eq!(link.status_read(peer(1), 23, 4), whole[4..]);
        assert!(link.status_read(peer(1), 23, whole.len()).is_empty());
        assert!(link.status_read(peer(1), 23, whole.len() + 10).is_empty());
        assert_eq!(
            reassemble(&link.status_chunks()),
            whole,
            "a new subscriber is sent the latest"
        );
    }

    #[tokio::test]
    async fn a_status_the_pump_could_not_take_is_pushed_again() {
        let (responses, _response_rx) = mpsc::channel(PUMP_DEPTH);
        let (status_out, mut pushed) = mpsc::channel(1);
        let link = Link::new(Echo, responses, status_out);
        let start = Instant::now();

        link.publish_status(start, status(false, 0));
        // The pump has not taken the first, so this one finds the channel
        // full and is not lost: the next poll offers it again.
        link.publish_status(start + Duration::from_secs(1), status(true, 0));
        pushed.try_recv().expect("the first");
        link.publish_status(start + Duration::from_secs(1), status(true, 0));
        assert!(pushed.try_recv().is_ok());
    }

    #[tokio::test]
    async fn the_status_chunks_follow_the_smallest_mtu() {
        let (link, (_responses, mut pushed)) = link(Echo);
        let mut long = status(false, 0);
        long.song_name = "n".repeat(100);
        link.on_write(peer(1), 517, &[]).await;
        link.publish_status(Instant::now(), long.clone());
        assert_eq!(pushed.try_recv().unwrap().len(), 1);

        link.on_connect(peer(2));
        long.playing = true;
        link.publish_status(Instant::now(), long);
        let small = pushed.try_recv().unwrap();
        assert!(small.len() > 1);
        assert!(small.iter().all(|c| c.len() <= 20));
    }
}
