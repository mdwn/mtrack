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

use futures_util::FutureExt;
use tokio::sync::watch;

use super::*;
use crate::controller::ble::codec::Reassembler;
use crate::controller::ble::dispatch::Dispatcher;
use crate::controller::ble::notify;
use crate::controller::ble::session::{Handler, Link, PUMP_DEPTH};
use crate::player::Player;
use crate::proto::ble::v1::Request;
use crate::proto::player::v1::{
    stage_update::Update, AssetChunk, GetLightingAssetRequest, StageUpdate, WatchStageRequest,
};

const CANCELLED: u32 = Code::Cancelled as u32;
const ALREADY_EXISTS: u32 = Code::AlreadyExists as u32;
const RESOURCE_EXHAUSTED: u32 = Code::ResourceExhausted as u32;
const PERMISSION_DENIED: u32 = Code::PermissionDenied as u32;

/// A characteristic of the fake radio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum On {
    Response,
    Status,
    Stream,
}

/// What the fake radio saw, in the order it saw it, across characteristics.
#[derive(Default)]
struct Seen {
    /// Chunks a pump began to send.
    tried: Vec<On>,
    /// Chunks that went out.
    sent: Vec<(On, Vec<u8>)>,
}

/// A notification session on the fake radio. While its characteristic is
/// held, a chunk given to it waits: a radio that is busy.
struct Air {
    on: On,
    seen: Arc<parking_lot::Mutex<Seen>>,
    held: watch::Receiver<bool>,
    ended: watch::Receiver<bool>,
}

impl Notifier for Air {
    fn is_stopped(&self) -> bool {
        *self.ended.borrow()
    }

    fn notify(&mut self, value: Vec<u8>) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            self.seen.lock().tried.push(self.on);
            let _ = self.held.wait_for(|held| !held).await;
            self.seen.lock().sent.push((self.on, value));
            true
        })
    }

    fn stopped(&self) -> BoxFuture<'static, ()> {
        let mut ended = self.ended.clone();
        Box::pin(async move {
            if ended.wait_for(|ended| *ended).await.is_err() {
                std::future::pending::<()>().await;
            }
        })
    }
}

/// A phone's side of the three notify characteristics, with all three
/// pumps running as the radio glue runs them.
struct Radio {
    seen: Arc<parking_lot::Mutex<Seen>>,
    hold: [watch::Sender<bool>; 3],
    end: [watch::Sender<bool>; 3],
    control: Arc<Control>,
    outbox: Arc<Outbox>,
    /// The pumps end when these do.
    _sessions: [mpsc::UnboundedSender<Air>; 3],
}

impl Radio {
    /// Makes `on` a busy radio, or lets it go.
    fn hold(&self, on: On, held: bool) {
        self.hold[on as usize].send_replace(held);
    }

    /// The phone unsubscribes from `on`.
    fn unsubscribe(&self, on: On) {
        self.end[on as usize].send_replace(true);
    }

    /// How many chunks the pump of `on` has begun to send.
    fn tried(&self, on: On) -> usize {
        self.seen.lock().tried.iter().filter(|o| **o == on).count()
    }

    /// How many chunks have gone out on `on`.
    fn sent(&self, on: On) -> usize {
        self.seen.lock().sent.iter().filter(|s| s.0 == on).count()
    }

    /// The whole frames that have gone out on `on`, as responses.
    fn responses(&self, on: On) -> Vec<Response> {
        let mut reassembler = Reassembler::new();
        let seen = self.seen.lock();
        seen.sent
            .iter()
            .filter(|sent| sent.0 == on)
            .filter_map(|sent| reassembler.push(&sent.1).expect("a chunk"))
            .map(|frame| Response::decode(frame.as_slice()).expect("a response"))
            .collect()
    }

    /// Waits until `count` responses have gone out on `on`: all of them.
    async fn wait_for(&self, on: On, count: usize) -> Vec<Response> {
        until(&format!("{count} responses on {on:?}"), || {
            Some(self.responses(on)).filter(|responses| responses.len() >= count)
        })
        .await
    }
}

/// Waits until `found` finds something, within the test patience.
async fn until<T>(what: &str, mut found: impl FnMut() -> Option<T>) -> T {
    let deadline = std::time::Instant::now() + crate::testutil::PATIENCE;
    loop {
        if let Some(value) = found() {
            return value;
        }
        assert!(std::time::Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Long enough for something that should not happen to have happened.
async fn quiet() {
    tokio::time::sleep(Duration::from_millis(30)).await;
}

/// A link over `handler` with a phone subscribed to everything, its pumps
/// running unpaced.
fn rig<H: Handler>(handler: H) -> (Arc<Link<H>>, Radio) {
    let (responses, response_frames) = mpsc::channel(PUMP_DEPTH);
    let (status_out, status_frames) = mpsc::channel(PUMP_DEPTH);
    let link = Link::new(handler, responses, status_out);
    let seen: Arc<parking_lot::Mutex<Seen>> = Default::default();
    let hold = [(); 3].map(|()| watch::channel(false).0);
    let end = [(); 3].map(|()| watch::channel(false).0);
    let air = |on: On| {
        let (sessions, sessions_rx) = mpsc::unbounded_channel();
        let session = Air {
            on,
            seen: seen.clone(),
            held: hold[on as usize].subscribe(),
            ended: end[on as usize].subscribe(),
        };
        assert!(sessions.send(session).is_ok());
        (sessions, sessions_rx)
    };
    let (response_sessions, response_rx) = air(On::Response);
    let (status_sessions, status_rx) = air(On::Status);
    let (stream_sessions, stream_rx) = air(On::Stream);
    tokio::spawn(notify::run(
        response_rx,
        response_frames,
        None,
        Some(link.control()),
    ));
    tokio::spawn(notify::run(
        status_rx,
        status_frames,
        None,
        Some(link.control()),
    ));
    let sizes = Arc::downgrade(&link);
    tokio::spawn(run(
        stream_rx,
        link.outbox(),
        link.control(),
        move || sizes.upgrade().map_or(20, |link| link.chunk_size()),
        None,
    ));
    let radio = Radio {
        seen,
        hold,
        end,
        control: link.control(),
        outbox: link.outbox(),
        _sessions: [response_sessions, status_sessions, stream_sessions],
    };
    (link, radio)
}

fn peer(n: u8) -> PeerId {
    [0xAA, 0, 0, 0, 0, n]
}

/// Writes `request` to the request characteristic, as `peer`.
fn write<H: Handler>(link: &Arc<Link<H>>, peer: PeerId, request: Request) {
    for chunk in chunks(&request.encode_to_vec(), 20).unwrap() {
        link.submit(peer, 23, &chunk);
    }
}

fn call(id: u32, method: &str, payload: Vec<u8>) -> Request {
    Request {
        id,
        method: method.to_string(),
        payload,
        cancel: false,
    }
}

fn cancel(id: u32) -> Request {
    Request {
        id,
        cancel: true,
        ..Default::default()
    }
}

/// A handler whose one stream, `Watch`, says what the test feeds it, and
/// which echoes every other call. A message beginning with `F` is
/// replaceable.
#[derive(Clone, Default)]
struct Script {
    /// The feed of each stream opened, in the order they were.
    opened: Arc<parking_lot::Mutex<Vec<mpsc::UnboundedSender<Step>>>>,
    /// What the next open is refused with.
    refuse: Arc<parking_lot::Mutex<Option<Status>>>,
}

impl Script {
    /// The feed of the `n`th stream opened, once it is.
    async fn feed(&self, n: usize) -> mpsc::UnboundedSender<Step> {
        until("the stream opened", || self.opened.lock().get(n).cloned()).await
    }
}

struct Fed(mpsc::UnboundedReceiver<Step>);

impl Messages for Fed {
    fn next(&mut self) -> BoxFuture<'_, Step> {
        Box::pin(async move {
            match self.0.recv().await {
                Some(step) => step,
                None => std::future::pending().await,
            }
        })
    }
}

impl Handler for Script {
    fn handle<'a>(&'a self, frame: &'a [u8]) -> BoxFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let request = Request::decode(frame).expect("a request");
            Response {
                id: request.id,
                payload: request.payload,
                ..Default::default()
            }
            .encode_to_vec()
        })
    }

    fn stream(&self, method: &str) -> Option<Replaceable> {
        (method == "Watch").then_some(|message| message.first() == Some(&b'F'))
    }

    fn open<'a>(
        &'a self,
        _method: &'a str,
        _payload: &'a [u8],
    ) -> BoxFuture<'a, Result<Box<dyn Messages>, Status>> {
        Box::pin(async move {
            if let Some(status) = self.refuse.lock().take() {
                return Err(status);
            }
            let (feed, fed) = mpsc::unbounded_channel();
            self.opened.lock().push(feed);
            Ok(Box::new(Fed(fed)) as Box<dyn Messages>)
        })
    }
}

fn message(text: &str) -> Step {
    Step::Message(text.as_bytes().to_vec())
}

/// Feeds a stream its next step.
fn say(feed: &mpsc::UnboundedSender<Step>, step: Step) {
    assert!(feed.send(step).is_ok(), "the stream is gone");
}

/// The payloads of the responses, as text; a last response as its code.
fn told(responses: &[Response]) -> Vec<String> {
    responses
        .iter()
        .map(|response| match response.more {
            true => String::from_utf8(response.payload.clone()).expect("text"),
            false => format!("end {}", response.code),
        })
        .collect()
}

fn slot() -> (Outbox, Arc<Slot>) {
    let outbox = Outbox::default();
    let slot = outbox
        .open(peer(1), 1, &AtomicBool::new(true))
        .expect("a slot");
    (outbox, slot)
}

fn frame(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

// The buffer's rule, with nothing running.

#[tokio::test]
async fn a_newer_replaceable_message_takes_the_place_of_the_one_waiting() {
    let (outbox, slot) = slot();
    slot.replace(frame("F1"));
    slot.replace(frame("F2"));
    slot.replace(frame("F3"));
    assert_eq!(outbox.take(), Some(frame("F3")));
    assert_eq!(outbox.take(), None);
    // One taken for sending is gone from the buffer: the next waits its turn.
    slot.replace(frame("F4"));
    assert_eq!(outbox.take(), Some(frame("F4")));
}

#[tokio::test]
async fn a_kept_message_discards_the_replaceable_one_before_it_and_is_never_dropped() {
    let (outbox, slot) = slot();
    slot.replace(frame("F1"));
    assert!(slot.keep(frame("L1")).await);
    slot.replace(frame("F2"));
    slot.replace(frame("F3"));

    // A second kept message waits for the first to go, and nothing grows.
    let mut second = Box::pin(slot.keep(frame("L2")));
    assert!(second.as_mut().now_or_never().is_none());
    assert_eq!(slot.state.lock().waiting.len(), 2);

    assert_eq!(outbox.take(), Some(frame("L1")));
    // F3 was read against L1, and L2 is newer than it.
    assert!(second.await);
    slot.replace(frame("F4"));
    slot.replace(frame("F5"));
    assert_eq!(outbox.take(), Some(frame("L2")));
    assert_eq!(outbox.take(), Some(frame("F5")));
    assert_eq!(outbox.take(), None);
}

#[tokio::test]
async fn a_stream_is_open_until_its_last_response_is_taken() {
    let (outbox, slot) = slot();
    slot.replace(frame("F1"));
    slot.finish(frame("end"));
    assert_eq!(outbox.open_count(), 1);
    assert_eq!(outbox.take(), Some(frame("F1")));
    assert_eq!(outbox.open_count(), 1);
    assert_eq!(outbox.take(), Some(frame("end")));
    assert_eq!(outbox.open_count(), 0);
}

#[tokio::test]
async fn streams_take_turns() {
    let outbox = Outbox::default();
    let alive = AtomicBool::new(true);
    let a = outbox.open(peer(1), 1, &alive).expect("a slot");
    let b = outbox.open(peer(2), 1, &alive).expect("a slot");
    assert!(a.keep(frame("a1")).await);
    assert!(b.keep(frame("b1")).await);
    assert_eq!(outbox.take(), Some(frame("a1")));
    assert!(a.keep(frame("a2")).await);
    // The first stream has more, and it is the second one's turn.
    assert_eq!(outbox.take(), Some(frame("b1")));
    assert_eq!(outbox.take(), Some(frame("a2")));
}

#[tokio::test]
async fn a_stream_is_not_opened_for_a_phone_that_is_gone() {
    let outbox = Outbox::default();
    let gone = AtomicBool::new(false);
    assert_eq!(outbox.open(peer(1), 1, &gone).err(), Some(Refused::Gone));
    assert!(Refused::Gone.status(1).is_none());
    outbox.close();
    let alive = AtomicBool::new(true);
    assert_eq!(outbox.open(peer(1), 1, &alive).err(), Some(Refused::Gone));
}

#[tokio::test]
async fn the_pacer_holds_the_pump_to_its_rate() {
    // 100 bytes at 5,000 a second is 20 ms.
    let mut pacer = Pacer::new(Some(5_000));
    let started = Instant::now();
    for _ in 0..10 {
        pacer.ready().await;
        pacer.sent(10);
    }
    pacer.ready().await;
    let took = started.elapsed();
    assert!(took >= Duration::from_millis(14), "{took:?}");

    // With no rate nothing waits.
    let mut pacer = Pacer::new(None);
    pacer.sent(1_000_000);
    assert!(pacer.ready().now_or_never().is_some());
}

#[tokio::test]
async fn the_stream_waits_only_while_a_control_frame_does() {
    let control = Control::default();
    assert!(control.clear().now_or_never().is_some());
    control.enter();
    control.enter();
    let mut clear = Box::pin(control.clear());
    assert!(clear.as_mut().now_or_never().is_none());
    control.leave();
    assert!(clear.as_mut().now_or_never().is_none());
    control.leave();
    assert!(clear.now_or_never().is_some());
}

// The envelope.

/// The messages as a client built before streams has them.
mod old {
    #[derive(Clone, PartialEq, prost::Message)]
    pub(super) struct Request {
        #[prost(uint32, tag = "1")]
        pub id: u32,
        #[prost(string, tag = "2")]
        pub method: String,
        #[prost(bytes = "vec", tag = "3")]
        pub payload: Vec<u8>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub(super) struct Response {
        #[prost(uint32, tag = "1")]
        pub id: u32,
        #[prost(uint32, tag = "2")]
        pub code: u32,
        #[prost(string, tag = "3")]
        pub message: String,
        #[prost(bytes = "vec", tag = "4")]
        pub payload: Vec<u8>,
    }
}

#[test]
fn an_old_client_and_this_player_read_each_other_as_before() {
    let asked = old::Request {
        id: 7,
        method: "Play".into(),
        payload: vec![1, 2, 3],
    };
    let read = Request::decode(asked.encode_to_vec().as_slice()).unwrap();
    assert!(!read.cancel);
    assert_eq!((read.id, read.method.as_str()), (7, "Play"));
    // A request that cancels nothing is, byte for byte, the old one.
    assert_eq!(read.encode_to_vec(), asked.encode_to_vec());

    // And a unary answer is, byte for byte, the old one.
    let answer = Response {
        id: 7,
        code: 5,
        message: "no".into(),
        payload: vec![9],
        more: false,
    };
    let old_answer = old::Response {
        id: 7,
        code: 5,
        message: "no".into(),
        payload: vec![9],
    };
    assert_eq!(answer.encode_to_vec(), old_answer.encode_to_vec());
    assert_eq!(
        Response::decode(old_answer.encode_to_vec().as_slice()).unwrap(),
        answer
    );
}

#[test]
fn more_cancel_and_the_last_response_round_trip() {
    let more = Response {
        id: 3,
        payload: vec![1],
        more: true,
        ..Default::default()
    };
    let read = Response::decode(more.encode_to_vec().as_slice()).unwrap();
    assert_eq!(read, more);
    assert!(read.more);

    let last = ending(3, &Status::cancelled("the stream was cancelled"));
    let read = Response::decode(last.as_slice()).unwrap();
    assert!(!read.more);
    assert_eq!((read.id, read.code), (3, CANCELLED));
    assert_eq!(read.message, "the stream was cancelled");
    assert!(read.payload.is_empty());
    // A stream that ends well ends with nothing but its id.
    let ok = Response::decode(ending(3, &Status::ok("")).as_slice()).unwrap();
    assert_eq!((ok.id, ok.code, ok.more), (3, 0, false));

    let asked = cancel(3);
    let read = Request::decode(asked.encode_to_vec().as_slice()).unwrap();
    assert!(read.cancel);
    assert_eq!(read.id, 3);
}

// A stream through the link, with a service the test feeds.

#[tokio::test]
async fn a_stream_arrives_on_the_stream_characteristic_with_more_on_all_but_the_last() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    write(&link, peer(1), call(5, "Watch", vec![]));
    let feed = script.feed(0).await;
    say(&feed, message("L1"));
    radio.wait_for(On::Stream, 1).await;
    say(&feed, message("F1"));
    radio.wait_for(On::Stream, 2).await;
    assert_eq!(radio.outbox.open_count(), 1);
    say(&feed, Step::End(Status::ok("")));

    let responses = radio.wait_for(On::Stream, 3).await;
    assert_eq!(told(&responses), ["L1", "F1", "end 0"]);
    assert!(responses.iter().all(|response| response.id == 5));
    assert!(responses[2].payload.is_empty());
    until("the stream closed", || {
        (radio.outbox.open_count() == 0).then_some(())
    })
    .await;
    // The service's side went when the stream ended.
    assert!(feed.is_closed());
    quiet().await;
    assert!(radio.responses(On::Response).is_empty());
}

#[tokio::test]
async fn a_stream_the_service_fails_or_refuses_ends_with_its_status() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    write(&link, peer(1), call(1, "Watch", vec![]));
    let feed = script.feed(0).await;
    say(&feed, message("L1"));
    say(&feed, Step::End(Status::internal("the rig fell over")));
    let responses = radio.wait_for(On::Stream, 2).await;
    assert_eq!(told(&responses), ["L1", "end 13"]);
    assert_eq!(responses[1].message, "the rig fell over");

    // Refused before it opens, as the lock refuses: the same place.
    *script.refuse.lock() = Some(Status::failed_precondition("locked"));
    write(&link, peer(1), call(2, "Watch", vec![]));
    let responses = radio.wait_for(On::Stream, 3).await;
    assert_eq!(
        (responses[2].id, responses[2].code, responses[2].more),
        (2, Code::FailedPrecondition as u32, false)
    );
    assert_eq!(radio.outbox.open_count(), 0);
    quiet().await;
    assert!(radio.responses(On::Response).is_empty());
}

#[tokio::test]
async fn a_slow_radio_gets_the_newest_frame_and_every_layout() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    write(&link, peer(1), call(1, "Watch", vec![]));
    let feed = script.feed(0).await;

    // The radio is busy with F1 from here on.
    radio.hold(On::Stream, true);
    say(&feed, message("F1"));
    until("the first chunk began", || {
        (radio.tried(On::Stream) == 1).then_some(())
    })
    .await;
    for (n, text) in ["F2", "F3", "L2", "F4", "F5", "F6"].iter().enumerate() {
        say(&feed, message(text));
        until("the reader took it", || {
            (radio.outbox.accepted() == n + 2).then_some(())
        })
        .await;
    }
    radio.hold(On::Stream, false);

    // F1 had begun and is finished; F2 and F3 were for the layout before L2
    // and are gone; of the frames for L2 only the newest is left.
    let responses = radio.wait_for(On::Stream, 3).await;
    assert_eq!(told(&responses), ["F1", "L2", "F6"]);
    quiet().await;
    assert_eq!(radio.responses(On::Stream).len(), 3);
}

#[tokio::test]
async fn layouts_wait_for_the_radio_instead_of_piling_up() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    write(&link, peer(1), call(1, "Watch", vec![]));
    let feed = script.feed(0).await;

    radio.hold(On::Stream, true);
    say(&feed, message("L1"));
    until("the first chunk began", || {
        (radio.tried(On::Stream) == 1).then_some(())
    })
    .await;
    for text in ["L2", "L3", "F1", "L4"] {
        say(&feed, message(text));
    }
    // L2 waits in the buffer; the reader holds L3 and has read no further.
    until("the reader took L2", || {
        (radio.outbox.accepted() == 2).then_some(())
    })
    .await;
    quiet().await;
    assert_eq!(radio.outbox.accepted(), 2);

    // A radio that keeps up from here on is sent all of it, in order.
    radio.hold(On::Stream, false);
    let responses = radio.wait_for(On::Stream, 5).await;
    assert_eq!(told(&responses), ["L1", "L2", "L3", "F1", "L4"]);
}

#[tokio::test]
async fn a_response_and_the_status_go_before_the_rest_of_a_stream_message() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    write(&link, peer(1), call(1, "Watch", vec![]));
    let feed = script.feed(0).await;

    // A layout of a hundred chunks, the radio busy with its first.
    radio.hold(On::Stream, true);
    let layout = format!("L{}", "x".repeat(2_000));
    say(&feed, message(&layout));
    until("the first chunk began", || {
        (radio.tried(On::Stream) == 1).then_some(())
    })
    .await;

    // A call is answered meanwhile, in several chunks, the radio no faster.
    radio.hold(On::Response, true);
    write(&link, peer(1), call(2, "Stop", vec![7; 60]));
    until("the response is waiting", || {
        (radio.tried(On::Response) == 1).then_some(())
    })
    .await;
    assert_eq!(radio.control.waiting(), 1);

    // The radio takes the chunk it had. The stream has a hundred more, and
    // sends none of them while the response waits.
    radio.hold(On::Stream, false);
    until("the chunk in flight went", || {
        (radio.sent(On::Stream) == 1).then_some(())
    })
    .await;
    quiet().await;
    assert_eq!(radio.tried(On::Stream), 1);

    // Nor while the status does: it goes out past the waiting stream.
    link.publish_status(std::time::Instant::now(), Default::default());
    until("the status went", || {
        (radio.sent(On::Status) == 1).then_some(())
    })
    .await;
    assert_eq!(radio.tried(On::Stream), 1);

    radio.hold(On::Response, false);
    let answer = radio.wait_for(On::Response, 1).await;
    assert_eq!((answer[0].id, answer[0].payload.len()), (2, 60));
    let stream = radio.wait_for(On::Stream, 1).await;
    assert_eq!(told(&stream), [layout]);
    assert_eq!(radio.control.waiting(), 0);

    // On the air: one chunk of the layout, the whole response, the rest.
    let seen = radio.seen.lock();
    let order: Vec<On> = seen
        .sent
        .iter()
        .map(|sent| sent.0)
        .filter(|on| *on != On::Status)
        .collect();
    let response_chunks = order.iter().filter(|on| **on == On::Response).count();
    assert!(response_chunks > 1);
    assert_eq!(order[0], On::Stream);
    assert!(order[1..=response_chunks]
        .iter()
        .all(|on| *on == On::Response));
    assert!(order[response_chunks + 1..]
        .iter()
        .all(|on| *on == On::Stream));
    assert!(order.len() > 100);
}

#[tokio::test]
async fn a_unary_call_is_answered_on_response_as_ever() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    write(&link, peer(1), call(9, "Play", vec![1, 2, 3]));
    let answer = radio.wait_for(On::Response, 1).await;
    assert_eq!(
        (answer[0].id, answer[0].code, answer[0].more),
        (9, 0, false)
    );
    assert_eq!(answer[0].payload, [1, 2, 3]);
    until("the response is counted out", || {
        (radio.control.waiting() == 0).then_some(())
    })
    .await;
    assert_eq!(radio.sent(On::Stream), 0);
    assert!(script.opened.lock().is_empty());
}

#[tokio::test]
async fn a_cancel_ends_the_stream_in_the_player_and_then_for_the_phone() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    write(&link, peer(1), call(1, "Watch", vec![]));
    let feed = script.feed(0).await;

    // The radio is busy with a layout, and another waits behind it.
    radio.hold(On::Stream, true);
    let layout = format!("L{}", "x".repeat(100));
    say(&feed, message(&layout));
    until("the first chunk began", || {
        (radio.tried(On::Stream) == 1).then_some(())
    })
    .await;
    say(&feed, message("L2"));
    until("the reader took L2", || {
        (radio.outbox.accepted() == 2).then_some(())
    })
    .await;

    write(&link, peer(1), cancel(1));
    // The service's side is dropped at once, the radio still busy.
    until("the stream was dropped", || feed.is_closed().then_some(())).await;
    assert_eq!(radio.sent(On::Stream), 0);

    // What had begun is finished, what waited is not sent, and the last
    // response says why.
    radio.hold(On::Stream, false);
    let responses = radio.wait_for(On::Stream, 2).await;
    assert_eq!(told(&responses), [layout, format!("end {CANCELLED}")]);
    assert_eq!(responses[1].id, 1);
    assert!(responses[1].payload.is_empty());
    until("the stream closed", || {
        (radio.outbox.open_count() == 0).then_some(())
    })
    .await;

    // A second cancel finds nothing, and nothing answers a cancel.
    write(&link, peer(1), cancel(1));
    quiet().await;
    assert_eq!(radio.responses(On::Stream).len(), 2);
    assert!(radio.responses(On::Response).is_empty());
}

#[tokio::test]
async fn a_cancel_for_no_stream_of_the_phones_own_is_ignored() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    write(&link, peer(1), call(1, "Watch", vec![]));
    let feed = script.feed(0).await;

    // Not an open id, and another phone's id.
    write(&link, peer(1), cancel(2));
    write(&link, peer(2), cancel(1));
    // A call after each: both cancels have been served when it is answered.
    write(&link, peer(1), call(3, "Play", vec![]));
    write(&link, peer(2), call(4, "Play", vec![]));
    radio.wait_for(On::Response, 2).await;
    quiet().await;

    assert!(!feed.is_closed());
    assert_eq!(radio.outbox.open_count(), 1);
    assert_eq!(radio.sent(On::Stream), 0);
    assert_eq!(radio.responses(On::Response).len(), 2);
    say(&feed, message("F1"));
    assert_eq!(told(&radio.wait_for(On::Stream, 1).await), ["F1"]);
}

#[tokio::test]
async fn a_disconnect_ends_the_phones_streams_and_says_nothing() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    write(&link, peer(1), call(1, "Watch", vec![]));
    let gone = script.feed(0).await;
    write(&link, peer(2), call(1, "Watch", vec![]));
    let stays = script.feed(1).await;
    say(&gone, message("F1"));
    radio.wait_for(On::Stream, 1).await;

    link.on_disconnect(peer(1));
    until("the stream was dropped", || gone.is_closed().then_some(())).await;
    assert_eq!(radio.outbox.open_count(), 1);
    quiet().await;
    assert_eq!(told(&radio.responses(On::Stream)), ["F1"]);

    // The other phone's stream is its own.
    assert!(!stays.is_closed());
    say(&stays, message("F2"));
    assert_eq!(told(&radio.wait_for(On::Stream, 2).await), ["F1", "F2"]);
}

#[tokio::test]
async fn a_phone_has_two_streams_and_is_refused_a_third() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    write(&link, peer(1), call(1, "Watch", vec![]));
    write(&link, peer(1), call(2, "Watch", vec![]));
    write(&link, peer(1), call(3, "Watch", vec![]));

    let refused = radio.wait_for(On::Stream, 1).await;
    assert_eq!(
        (refused[0].id, refused[0].code, refused[0].more),
        (3, RESOURCE_EXHAUSTED, false)
    );
    assert_eq!(radio.outbox.open_count(), 2);
    assert_eq!(script.opened.lock().len(), 2, "the third never opened");

    // Another phone's streams are counted apart.
    write(&link, peer(2), call(3, "Watch", vec![]));
    script.feed(2).await;
    assert_eq!(radio.outbox.open_count(), 3);

    // One ends, and there is room again.
    write(&link, peer(1), cancel(1));
    radio.wait_for(On::Stream, 2).await;
    write(&link, peer(1), call(4, "Watch", vec![]));
    script.feed(3).await;
    quiet().await;
    assert_eq!(radio.responses(On::Stream).len(), 2);
    assert!(radio.responses(On::Response).is_empty());
}

#[tokio::test]
async fn an_id_that_is_open_is_refused_on_response_and_the_open_stream_carries_on() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    write(&link, peer(1), call(1, "Watch", vec![]));
    let feed = script.feed(0).await;
    write(&link, peer(1), call(1, "Watch", vec![]));

    // Refused where it cannot be read as the end of the stream that is open.
    let refused = radio.wait_for(On::Response, 1).await;
    assert_eq!(
        (refused[0].id, refused[0].code, refused[0].more),
        (1, ALREADY_EXISTS, false)
    );
    assert!(refused[0].payload.is_empty());
    until("the refusal is counted out", || {
        (radio.control.waiting() == 0).then_some(())
    })
    .await;
    quiet().await;
    assert_eq!(radio.sent(On::Stream), 0);
    assert_eq!(script.opened.lock().len(), 1);
    assert_eq!(radio.outbox.open_count(), 1);
    assert!(!feed.is_closed());

    // On `stream`, a response without `more` is the end of its stream and
    // nothing else: the open one says what it has, and then ends once.
    say(&feed, message("F1"));
    say(&feed, Step::End(Status::ok("")));
    let responses = radio.wait_for(On::Stream, 2).await;
    assert_eq!(told(&responses), ["F1", "end 0"]);

    // It is refused the same way while a cancel's last response is still
    // to go, and not once it has gone.
    write(&link, peer(1), call(2, "Watch", vec![]));
    let feed = script.feed(1).await;
    radio.hold(On::Stream, true);
    say(&feed, message("L1"));
    until("the first chunk began", || {
        (radio.tried(On::Stream) == 3).then_some(())
    })
    .await;
    write(&link, peer(1), cancel(2));
    write(&link, peer(1), call(2, "Watch", vec![]));
    let refused = radio.wait_for(On::Response, 2).await;
    assert_eq!((refused[1].id, refused[1].code), (2, ALREADY_EXISTS));
    radio.hold(On::Stream, false);
    let responses = radio.wait_for(On::Stream, 4).await;
    assert_eq!(told(&responses[2..]), ["L1", &format!("end {CANCELLED}")]);
    write(&link, peer(1), call(2, "Watch", vec![]));
    script.feed(2).await;

    // The same id is another phone's to use.
    write(&link, peer(2), call(2, "Watch", vec![]));
    script.feed(3).await;
    assert_eq!(radio.responses(On::Response).len(), 2);
}

#[tokio::test]
async fn a_message_too_large_for_a_frame_ends_the_stream() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    write(&link, peer(1), call(1, "Watch", vec![]));
    let feed = script.feed(0).await;
    say(&feed, Step::Message(vec![b'L'; MAX_FRAME]));
    let responses = radio.wait_for(On::Stream, 1).await;
    assert_eq!(
        (responses[0].code, responses[0].more),
        (RESOURCE_EXHAUSTED, false)
    );
    assert!(feed.is_closed());
}

#[tokio::test]
async fn streams_end_when_nobody_is_subscribed_to_stream() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    write(&link, peer(1), call(1, "Watch", vec![]));
    let feed = script.feed(0).await;
    say(&feed, message("F1"));
    radio.wait_for(On::Stream, 1).await;

    // Nothing is being sent when the phone unsubscribes.
    radio.unsubscribe(On::Stream);
    until("the stream was dropped", || feed.is_closed().then_some(())).await;
    assert_eq!(radio.outbox.open_count(), 0);

    // One opened with nobody listening ends at its first message.
    write(&link, peer(1), call(2, "Watch", vec![]));
    let feed = script.feed(1).await;
    say(&feed, message("F1"));
    until("the stream was dropped", || feed.is_closed().then_some(())).await;
    assert_eq!(radio.responses(On::Stream).len(), 1);
}

#[tokio::test]
async fn streams_end_when_the_controller_stops() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    write(&link, peer(1), call(1, "Watch", vec![]));
    let feed = script.feed(0).await;
    say(&feed, message("F1"));
    radio.wait_for(On::Stream, 1).await;

    drop(link);
    until("the stream was dropped", || feed.is_closed().then_some(())).await;
    assert_eq!(radio.outbox.open_count(), 0);
}

#[tokio::test]
async fn a_phone_pushed_out_of_the_table_loses_its_streams() {
    use crate::controller::ble::session::MAX_PEERS;

    let script = Script::default();
    let (link, radio) = rig(script.clone());
    write(&link, peer(1), call(1, "Watch", vec![]));
    let feed = script.feed(0).await;
    for n in 0..MAX_PEERS as u8 {
        link.on_connect([0xBB, 0, 0, 0, 0, n]);
    }
    until("the stream was dropped", || feed.is_closed().then_some(())).await;
    assert_eq!(radio.outbox.open_count(), 0);
}

// The stage stream, through the player's own service.

/// A snapshot of fixtures, each with channels and their values.
fn stage_of(fixtures: &[(&str, &[(&str, u8)])]) -> Arc<crate::state::StateSnapshot> {
    Arc::new(crate::state::StateSnapshot {
        fixtures: fixtures
            .iter()
            .map(|(name, channels)| crate::state::FixtureSnapshot {
                name: name.to_string(),
                channels: channels.iter().map(|(c, v)| (c.to_string(), *v)).collect(),
                cells: Default::default(),
                strobe_hz: None,
            })
            .collect(),
        ..Default::default()
    })
}

type StateTx = Arc<watch::Sender<Arc<crate::state::StateSnapshot>>>;

/// A player whose stage is one fixture with `red` at 1, and the sender its
/// sampler would push snapshots through.
fn stage_player() -> (Arc<Player>, StateTx) {
    let player = crate::testutil::player::memory_player(&["Song A"]).build();
    let (state_tx, _) = watch::channel(stage_of(&[("par", &[("red", 1)])]));
    player.set_state_tx(state_tx);
    let state_tx = player.state_tx().expect("the sender just set");
    (player, state_tx)
}

fn watch_stage(id: u32) -> Request {
    call(
        id,
        "WatchStage",
        WatchStageRequest { max_rate: 0 }.encode_to_vec(),
    )
}

/// What the responses of a stage stream say: `layout 2`, `frame 2 [5, 1]`
/// (the layout it is read against, and its channels), or `end 1`.
fn stage(responses: &[Response]) -> Vec<String> {
    responses
        .iter()
        .map(|response| {
            if !response.more {
                return format!("end {}", response.code);
            }
            let update = StageUpdate::decode(response.payload.as_slice()).expect("an update");
            match update.update.expect("a layout or a frame") {
                Update::Layout(layout) => format!("layout {}", layout.id),
                Update::Frame(frame) => format!("frame {} {:?}", frame.layout, frame.channels),
            }
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_stage_streams_to_a_phone_and_a_cancel_ends_it() {
    let (player, state_tx) = stage_player();
    // Players start locked, and the stage is open while they are.
    assert!(player.is_locked());
    let watchers = state_tx.receiver_count();
    let (link, radio) = rig(Dispatcher::new(player));

    write(&link, peer(1), watch_stage(7));
    let responses = radio.wait_for(On::Stream, 2).await;
    assert_eq!(stage(&responses), ["layout 1", "frame 1 [1]"]);
    assert!(responses.iter().all(|r| r.id == 7 && r.more && r.code == 0));
    assert_eq!(state_tx.receiver_count(), watchers + 1);

    state_tx.send_replace(stage_of(&[("par", &[("red", 2)])]));
    let responses = radio.wait_for(On::Stream, 3).await;
    assert_eq!(stage(&responses[2..]), ["frame 1 [2]"]);

    write(&link, peer(1), cancel(7));
    let responses = radio.wait_for(On::Stream, 4).await;
    assert_eq!(stage(&responses[3..]), [format!("end {CANCELLED}")]);
    assert!(!responses[3].more);
    // The player's side of the stream is gone, not merely unheard.
    until("the service's stream was dropped", || {
        (state_tx.receiver_count() == watchers).then_some(())
    })
    .await;

    // Nothing more comes of a change, and nothing was said on `response`.
    state_tx.send_replace(stage_of(&[("par", &[("red", 3)])]));
    quiet().await;
    assert_eq!(radio.responses(On::Stream).len(), 4);
    assert!(radio.responses(On::Response).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_phone_sees_the_newest_stage_frame_and_never_one_for_an_older_layout() {
    let (player, state_tx) = stage_player();
    let (link, radio) = rig(Dispatcher::new(player));
    write(&link, peer(1), watch_stage(1));
    radio.wait_for(On::Stream, 2).await;
    let chunks_sent = radio.tried(On::Stream);
    let accepted = radio.outbox.accepted();
    assert_eq!(accepted, 2);

    // The radio is busy with the frame that says red is 2 from here on.
    radio.hold(On::Stream, true);
    let push = async |snapshot, count: usize| {
        state_tx.send_replace(snapshot);
        until("the reader took it", || {
            (radio.outbox.accepted() == accepted + count).then_some(())
        })
        .await;
    };
    push(stage_of(&[("par", &[("red", 2)])]), 1).await;
    until("its first chunk began", || {
        (radio.tried(On::Stream) == chunks_sent + 1).then_some(())
    })
    .await;
    push(stage_of(&[("par", &[("red", 3)])]), 2).await;
    push(stage_of(&[("par", &[("red", 4)])]), 3).await;
    // A fixture more is a new layout and the frame read against it.
    let wider = |red, dimmer| stage_of(&[("par", &[("red", red)]), ("bar", &[("dimmer", dimmer)])]);
    push(wider(4, 5), 5).await;
    push(wider(6, 5), 6).await;
    push(wider(7, 8), 7).await;
    radio.hold(On::Stream, false);

    let responses = radio.wait_for(On::Stream, 5).await;
    assert_eq!(
        stage(&responses[2..]),
        ["frame 1 [2]", "layout 2", "frame 2 [8, 7]"]
    );
    quiet().await;
    assert_eq!(radio.responses(On::Stream).len(), 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_disconnect_drops_the_stage_stream_in_the_player() {
    let (player, state_tx) = stage_player();
    let watchers = state_tx.receiver_count();
    let (link, radio) = rig(Dispatcher::new(player));
    write(&link, peer(1), watch_stage(1));
    radio.wait_for(On::Stream, 2).await;
    assert_eq!(state_tx.receiver_count(), watchers + 1);

    link.on_disconnect(peer(1));
    until("the service's stream was dropped", || {
        (state_tx.receiver_count() == watchers).then_some(())
    })
    .await;
    quiet().await;
    assert_eq!(radio.responses(On::Stream).len(), 2, "and nothing is said");
}

#[tokio::test(flavor = "multi_thread")]
async fn what_is_not_a_stream_is_answered_on_response_and_a_stream_never_is() {
    let (player, _state_tx) = stage_player();
    let (link, radio) = rig(Dispatcher::new(player));

    // Off the allowlist: refused where every refusal has always been.
    write(&link, peer(1), call(1, "UpdateAudio", vec![]));
    // A unary call.
    write(&link, peer(1), call(2, "Status", vec![]));
    // A frame that is no request at all.
    for chunk in chunks(&[0x12, 0xff, 0x01], 20).unwrap() {
        link.submit(peer(1), 23, &chunk);
    }
    let answers = radio.wait_for(On::Response, 3).await;
    assert_eq!((answers[0].id, answers[0].code), (1, PERMISSION_DENIED));
    assert_eq!((answers[1].id, answers[1].code), (2, 0));
    assert_eq!(
        (answers[2].id, answers[2].code),
        (0, Code::InvalidArgument as u32)
    );
    assert!(answers.iter().all(|answer| !answer.more));
    assert_eq!(radio.sent(On::Stream), 0);

    // A stream the service refuses, here for a request it cannot read, is
    // refused on `stream`.
    write(&link, peer(1), call(3, "WatchStage", vec![0xff]));
    let refused = radio.wait_for(On::Stream, 1).await;
    assert_eq!((refused[0].id, refused[0].more), (3, false));
    assert_ne!(refused[0].code, 0);
    assert_eq!(radio.outbox.open_count(), 0);
    quiet().await;
    assert_eq!(radio.responses(On::Response).len(), 3);
}

// A file in chunks, as `GetLightingAsset` sends one: none replaceable.

#[tokio::test]
async fn every_message_of_a_stream_without_replaceables_arrives_in_order_and_its_reader_waits() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    radio.hold(On::Stream, true);
    write(&link, peer(1), call(1, "Watch", vec![]));
    let feed = script.feed(0).await;
    let names: Vec<String> = (0..40).map(|n| format!("A{n:02}")).collect();
    for name in &names {
        say(&feed, message(name));
    }
    say(&feed, Step::End(Status::ok("")));

    // The radio is busy: one chunk is on it, one waits in the slot, and the
    // rest wait where they were, in the service's stream, not in a queue
    // that grows.
    quiet().await;
    assert!(radio.outbox.accepted() <= 2, "{}", radio.outbox.accepted());

    radio.hold(On::Stream, false);
    let responses = radio.wait_for(On::Stream, names.len() + 1).await;
    assert_eq!(told(&responses[..names.len()]), names);
    assert!(!responses[names.len()].more);
}

#[tokio::test]
async fn a_file_in_flight_does_not_hold_back_the_other_stream() {
    let script = Script::default();
    let (link, radio) = rig(script.clone());
    radio.hold(On::Stream, true);
    write(&link, peer(1), call(1, "Watch", vec![]));
    write(&link, peer(1), call(2, "Watch", vec![]));
    let file = script.feed(0).await;
    let frames = script.feed(1).await;
    for n in 0..8 {
        say(&file, message(&format!("A{n}")));
    }
    say(&frames, message("F1"));
    quiet().await;
    radio.hold(On::Stream, false);

    let responses = radio.wait_for(On::Stream, 9).await;
    let order: Vec<(u32, String)> = responses
        .iter()
        .map(|r| (r.id, String::from_utf8(r.payload.clone()).unwrap()))
        .collect();
    let at = order
        .iter()
        .position(|(id, _)| *id == 2)
        .expect("the frame");
    // The streams take turns a message at a time: the frame goes after the
    // chunk in flight and the one that follows, not after all eight.
    assert!(at <= 2, "{order:?}");
    let file_chunks: Vec<&str> = order
        .iter()
        .filter(|(id, _)| *id == 1)
        .map(|(_, text)| text.as_str())
        .collect();
    assert_eq!(
        file_chunks,
        ["A0", "A1", "A2", "A3", "A4", "A5", "A6", "A7"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lighting_asset_streams_whole_beside_the_stage() {
    let project = crate::testutil::player::playlist_project()
        .await
        .expect("a project");
    let file: Vec<u8> = (0..45_000u32).map(|n| (n % 253) as u8).collect();
    let store = project.dir.path().join("lighting/.cache/assets/abc");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("rig.json"), &file).unwrap();
    // The stage, with a sender, so that there is a frame to send.
    let (state_tx, _) = watch::channel(stage_of(&[("par", &[("red", 1)])]));
    project.player.set_state_tx(state_tx);
    // Locked: both are reads.
    project.player.set_locked(true);
    let (link, radio) = rig(Dispatcher::new(project.player.clone()));

    radio.hold(On::Stream, true);
    let asset = GetLightingAssetRequest {
        path: "abc/rig.json".to_string(),
        gzip: false,
    };
    write(
        &link,
        peer(1),
        call(1, "GetLightingAsset", asset.encode_to_vec()),
    );
    quiet().await;
    write(&link, peer(1), watch_stage(2));
    quiet().await;
    radio.hold(On::Stream, false);

    // 45,000 bytes in 8 KiB chunks is 6, then the last response; and the
    // stage's layout and frame.
    let responses = radio.wait_for(On::Stream, 9).await;
    let of = |id| responses.iter().filter(move |r| r.id == id);
    let chunks: Vec<AssetChunk> = of(1)
        .filter(|r| r.more)
        .map(|r| AssetChunk::decode(r.payload.as_slice()).expect("a chunk"))
        .collect();
    assert_eq!(chunks.len(), 6);
    assert_eq!(chunks[0].total_size, 45_000);
    let joined: Vec<u8> = chunks.into_iter().flat_map(|c| c.data).collect();
    assert_eq!(joined, file);
    let end = of(1).next_back().expect("an end");
    assert_eq!((end.more, end.code), (false, 0));
    assert_eq!(
        stage(&of(2).cloned().collect::<Vec<_>>()),
        ["layout 1", "frame 1 [1]"]
    );

    // The stage got its turn while the file was still going.
    let last_chunk = responses.iter().rposition(|r| r.id == 1 && r.more).unwrap();
    let layout = responses.iter().position(|r| r.id == 2).unwrap();
    assert!(layout < last_chunk, "the stage waited for the whole file");

    // A path outside the store is the service's refusal, on stream.
    let bad = GetLightingAssetRequest {
        path: "../x.json".to_string(),
        gzip: false,
    };
    write(
        &link,
        peer(1),
        call(3, "GetLightingAsset", bad.encode_to_vec()),
    );
    let refused = until("a refusal", || {
        radio.responses(On::Stream).into_iter().find(|r| r.id == 3)
    })
    .await;
    assert_eq!(refused.code, Code::InvalidArgument as u32);
}
