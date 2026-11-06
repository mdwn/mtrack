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
use super::*;

use std::{
    collections::HashSet,
    error::Error,
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

use midly::live::LiveEvent;

use crate::{clock::PlaybackClock, config, playsync::CancelHandle, songs::Song};

/// Runs a section loop jump publisher for a test playback, as
/// `play_files` runs one for a song without audio: the jumps for the
/// armed section (on `clock`, from `start_time`) go into the log given
/// back, until the flag given back is set.
fn publish_jumps_for_test(
    clock: &PlaybackClock,
    start_time: Duration,
    active_section: &Arc<parking_lot::RwLock<Option<crate::player::SectionBounds>>>,
    section_loop_break: &Arc<AtomicBool>,
) -> (Arc<crate::playsync::SectionJumpLog>, Arc<AtomicBool>) {
    let mut loop_control = crate::playsync::LoopControl::new();
    loop_control.active_section = active_section.clone();
    loop_control.section_loop_break = section_loop_break.clone();
    let log = loop_control.section_jumps.clone();
    let done = Arc::new(AtomicBool::new(false));
    let publisher = crate::section_loop::JumpPublisher {
        clock: clock.clone(),
        start_time,
        cancel: CancelHandle::new(),
        done: done.clone(),
        audio_gone: None,
        loop_control,
        song: None,
    };
    thread::spawn(move || crate::section_loop::publish_jumps(publisher));
    // The audio this stands in for plays on as long as the notes: the
    // log closes once the section is broken or cleared (the notes then
    // end with their own), or the test is done.
    {
        let log = log.clone();
        let done = done.clone();
        let active_section = active_section.clone();
        let section_loop_break = section_loop_break.clone();
        thread::spawn(move || {
            while !done.load(Ordering::Relaxed)
                && !section_loop_break.load(Ordering::Relaxed)
                && active_section.read().is_some()
            {
                thread::sleep(Duration::from_millis(5));
            }
            // A jump committed just before still falls due.
            thread::sleep(Duration::from_millis(50));
            log.close();
        });
    }
    (log, done)
}
use midly::num::u7;
use midly::MidiMessage;

/// Records what it is sent; or, made `failing`, fails every send.
struct MockSender {
    sent: parking_lot::Mutex<Vec<Vec<u8>>>,
    should_fail: bool,
}

impl MockSender {
    fn new() -> Self {
        MockSender {
            sent: parking_lot::Mutex::new(Vec::new()),
            should_fail: false,
        }
    }

    fn failing() -> Self {
        MockSender {
            sent: parking_lot::Mutex::new(Vec::new()),
            should_fail: true,
        }
    }
}

impl MidiSender for MockSender {
    fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
        if self.should_fail {
            return Err("mock send failure".into());
        }
        self.sent.lock().push(bytes.to_vec());
        Ok(())
    }
}

/// Sends nowhere.
struct Null;

impl MidiSender for Null {
    fn send(&mut self, _: &[u8]) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
}

/// Note-ons at `times_ms`, keys 60 up, on channel 0.
fn make_events(times_ms: &[u64]) -> crate::midi::playback::PrecomputedMidi {
    let events: Vec<crate::midi::playback::TimedMidiEvent> = times_ms
        .iter()
        .enumerate()
        .map(|(i, &t)| crate::midi::playback::TimedMidiEvent {
            time: Duration::from_millis(t),
            channel: 0,
            message: midly::MidiMessage::NoteOn {
                key: u7::new(60 + i as u8),
                vel: u7::new(100),
            },
        })
        .collect();
    crate::midi::playback::PrecomputedMidi::from_events(events)
}

fn start_bytes() -> Vec<u8> {
    realtime_bytes(midly::live::SystemRealtime::Start)
}

fn continue_bytes() -> Vec<u8> {
    realtime_bytes(midly::live::SystemRealtime::Continue)
}

fn stop_bytes() -> Vec<u8> {
    realtime_bytes(midly::live::SystemRealtime::Stop)
}

fn clock_bytes() -> Vec<u8> {
    realtime_bytes(midly::live::SystemRealtime::TimingClock)
}

mod display {
    use super::*;

    #[test]
    fn display_no_ports() {
        let device = Device::new_default("test-device".to_string());
        assert_eq!(format!("{}", device), "test-device ()");
    }

    #[test]
    fn display_input_only() {
        let device = Device::new_default("test-device".to_string());
        assert!(device.input_port.is_none());
        assert!(device.output_port.is_none());
        let display = format!("{}", device);
        assert!(display.contains("test-device"));
    }

    #[test]
    fn new_default_has_empty_fields() {
        let device = Device::new_default("my-midi".to_string());
        assert_eq!(device.name, "my-midi");
        assert_eq!(device.playback_delay, Duration::ZERO);
        assert!(!device.beat_clock_enabled);
        assert!(device.input_port.is_none());
        assert!(device.output_port.is_none());
        assert!(device.beat_clock_engine.is_none());
    }
}

mod serialize_midi_event_tests {
    use super::*;
    use crate::midi::playback::TimedMidiEvent;

    fn make_event(channel: u8, key: u8) -> TimedMidiEvent {
        TimedMidiEvent {
            time: Duration::from_millis(100),
            channel,
            message: MidiMessage::NoteOn {
                key: u7::new(key),
                vel: u7::new(100),
            },
        }
    }

    #[test]
    fn serializes_event_with_no_exclusions() {
        let event = make_event(0, 60);
        let exclude = HashSet::new();
        let mut buf = Vec::new();

        let result = serialize_midi_event(&event, &exclude, &mut buf);
        assert!(result.is_some());
        let bytes = result.unwrap();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn excludes_matching_channel() {
        let event = make_event(5, 60);
        let exclude = HashSet::from([5]);
        let mut buf = Vec::new();

        let result = serialize_midi_event(&event, &exclude, &mut buf);
        assert!(result.is_none());
    }

    #[test]
    fn passes_non_excluded_channel() {
        let event = make_event(3, 60);
        let exclude = HashSet::from([5, 9]);
        let mut buf = Vec::new();

        let result = serialize_midi_event(&event, &exclude, &mut buf);
        assert!(result.is_some());
    }

    #[test]
    fn serialized_bytes_are_valid_midi() {
        let event = make_event(0, 60);
        let exclude = HashSet::new();
        let mut buf = Vec::new();

        let bytes = serialize_midi_event(&event, &exclude, &mut buf).unwrap();
        // Standard MIDI note-on: status byte (0x90 | channel), key, velocity
        assert_eq!(bytes.len(), 3);
        assert_eq!(bytes[0], 0x90); // Note on, channel 0
        assert_eq!(bytes[1], 60); // Key
        assert_eq!(bytes[2], 100); // Velocity
    }

    #[test]
    fn different_channels_produce_correct_status_byte() {
        let exclude = HashSet::new();
        let mut buf = Vec::new();

        for ch in 0..16u8 {
            let event = make_event(ch, 60);
            let bytes = serialize_midi_event(&event, &exclude, &mut buf).unwrap();
            assert_eq!(bytes[0], 0x90 | ch);
        }
    }

    #[test]
    fn note_off_serialization() {
        let event = TimedMidiEvent {
            time: Duration::from_millis(200),
            channel: 0,
            message: MidiMessage::NoteOff {
                key: u7::new(60),
                vel: u7::new(64),
            },
        };
        let exclude = HashSet::new();
        let mut buf = Vec::new();

        let bytes = serialize_midi_event(&event, &exclude, &mut buf).unwrap();
        assert_eq!(bytes.len(), 3);
        assert_eq!(bytes[0], 0x80); // Note off, channel 0
        assert_eq!(bytes[1], 60);
        assert_eq!(bytes[2], 64);
    }

    #[test]
    fn empty_exclude_set_passes_all() {
        let exclude = HashSet::new();
        let mut buf = Vec::new();

        for ch in 0..16u8 {
            let event = make_event(ch, 60);
            assert!(serialize_midi_event(&event, &exclude, &mut buf).is_some());
        }
    }
}

mod validate_device_match_tests {
    use super::*;

    // Simple wrapper to provide Display for test strings.
    struct Named(String);
    impl fmt::Display for Named {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    #[test]
    fn single_match_succeeds() {
        let matches = vec![Named("device-1".to_string())];
        assert!(validate_device_match("device", &matches).is_ok());
    }

    #[test]
    fn no_matches_fails() {
        let matches: Vec<Named> = vec![];
        let err = validate_device_match("my-device", &matches).unwrap_err();
        assert!(err.to_string().contains("no device found"));
        assert!(err.to_string().contains("my-device"));
    }

    #[test]
    fn multiple_matches_fails() {
        let matches = vec![
            Named("midi-device-1".to_string()),
            Named("midi-device-2".to_string()),
        ];
        let err = validate_device_match("midi-device", &matches).unwrap_err();
        assert!(err.to_string().contains("too many devices"));
        assert!(err.to_string().contains("midi-device-1"));
        assert!(err.to_string().contains("midi-device-2"));
    }

    #[test]
    fn exactly_two_matches_fails() {
        let matches = vec![Named("a".to_string()), Named("b".to_string())];
        assert!(validate_device_match("x", &matches).is_err());
    }
}

#[test]
fn to_mock_returns_error() {
    let device = Device::new_default("test".to_string());
    let result = <Device as crate::midi::Device>::to_mock(&device);
    assert!(result.is_err());
    let err = result.err().unwrap();
    assert!(err.to_string().contains("not a mock"));
}

/// A device that cannot be opened afresh fails to reopen, for the
/// reload to open a new one, and is left as it was, its watch included.
#[test]
fn reopen_of_a_device_gone_fails_and_leaves_it() {
    use crate::midi::Device as _;

    let device = Reopenable::with_device(
        config::Midi::new("mtrack-test-no-such-midi-device", None),
        Device::new_default("mtrack-test-no-such-midi-device".to_string()),
    );
    let before = device.current();
    let (tx, _rx) = tokio::sync::mpsc::channel(10);
    device.watch_events(tx).unwrap();

    assert!(device.reopen().is_err());
    assert!(Arc::ptr_eq(&before, &device.current()));
    assert!(device.watch.lock().unwrap().is_some());
    device.stop_watch_events();
    assert!(device.watch.lock().unwrap().is_none());
}

/// A device whose beat clock holds `tempo`, without the engine's thread
/// or an output port.
fn device_holding(name: &str, tempo: Option<Duration>) -> Device {
    let (tx, _) = mpsc::channel();
    let mut device = Device::new_default(name.to_string());
    device.beat_clock_engine = Some(Arc::new(BeatClockEngine {
        tx,
        join: Mutex::new(None),
        tempo: Arc::new(Mutex::new(tempo)),
    }));
    device
}

/// A device opened afresh started its beat clock knowing no tempo, so
/// one free-running the tempo held between songs went silent until the
/// next song. The fresh one is opened holding it.
/// The fresh device starts its beat clock free-running the held tempo,
/// so the old one's is stopped before it opens: both at once would send
/// timing clocks to the same port.
#[test]
fn reopen_stops_the_old_beat_clock_before_the_fresh_one_runs() {
    let (tx, rx) = mpsc::channel();
    let mut old = Device::new_default("test".to_string());
    old.beat_clock_engine = Some(Arc::new(BeatClockEngine {
        tx,
        join: Mutex::new(None),
        tempo: Arc::new(Mutex::new(Some(Duration::from_millis(20)))),
    }));
    let device = Reopenable::with_device(config::Midi::new("test", None), old);
    let mut old_running_at_open = None;
    device
        .reopen_with(|tempo| {
            old_running_at_open = Some(!matches!(rx.try_recv(), Ok(BeatClockCommand::Shutdown)));
            Ok(device_holding("test", tempo))
        })
        .unwrap();
    assert_eq!(
        old_running_at_open,
        Some(false),
        "the fresh engine (holding {:?}) started while the old one still ran",
        device.current().held_tempo()
    );
}

/// A fresh device that cannot take the watch over, and its beat clock
/// thread's join, are dropped outside the watch lock.
#[test]
fn a_refused_fresh_device_is_dropped_outside_the_watch_lock() {
    use crate::midi::Device as _;
    let reopenable: Arc<std::sync::OnceLock<std::sync::Weak<Reopenable>>> = Arc::default();
    let free = Arc::new(Mutex::new(None));
    let device = Arc::new(Reopenable::with_device(
        config::Midi::new("test", None),
        Device::new_default("test".to_string()),
    ));
    reopenable.set(Arc::downgrade(&device)).unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(10);
    device.watch_events(tx).unwrap();
    let _ = device.reopen_with(|_| {
        let mut fresh = Device::new_default("test".to_string());
        fresh.refuse_watch = true;
        fresh.on_drop = Some(Box::new({
            let reopenable = reopenable.clone();
            let free = free.clone();
            move || {
                let device = reopenable.get().and_then(std::sync::Weak::upgrade);
                *free.lock().unwrap() = device.map(|d| d.watch.try_lock().is_ok());
            }
        }));
        Ok(fresh)
    });
    assert_eq!(
        *free.lock().unwrap(),
        Some(true),
        "dropped under the watch lock"
    );
}

/// A reopen whose fresh device cannot be opened has stopped the old
/// clock already, but the tempo it held stays readable through the
/// device (`Device::held_tempo`), for the device the reload opens in its
/// place to go on with.
#[test]
fn a_failed_reopen_keeps_the_held_tempo_for_the_device_opened_instead() {
    let (tx, rx) = mpsc::channel();
    let mut old = Device::new_default("test".to_string());
    old.beat_clock_engine = Some(Arc::new(BeatClockEngine {
        tx,
        join: Mutex::new(None),
        tempo: Arc::new(Mutex::new(Some(Duration::from_millis(20)))),
    }));
    let device = Reopenable::with_device(config::Midi::new("test", None), old);
    assert!(device.reopen_with(|_| Err("device gone".into())).is_err());
    assert!(
        matches!(rx.try_recv(), Ok(BeatClockCommand::Shutdown)),
        "the old clock was not stopped before the fresh device was tried"
    );
    assert_eq!(
        crate::midi::Device::held_tempo(&device),
        Some(Duration::from_millis(20))
    );
}

/// The playback delay counts from the playback clock's start, as the
/// notes count it, not from when the engine noticed the start (#600): an
/// engine told of a song well after its clock started still has its first
/// clock out a delay after that start.
#[test]
fn the_playback_delay_counts_from_the_clocks_start() {
    struct FirstClock {
        clock: PlaybackClock,
        at: Arc<Mutex<Option<Duration>>>,
    }
    impl MidiSender for FirstClock {
        fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
            if bytes == [0xF8] {
                let mut at = self.at.lock().unwrap();
                at.get_or_insert(self.clock.elapsed());
            }
            Ok(())
        }
    }
    let clock = PlaybackClock::wall();
    clock.start();
    // The engine hears of the song 600ms into it.
    std::thread::sleep(Duration::from_millis(600));
    let at = Arc::new(Mutex::new(None));
    let (tx, rx) = mpsc::channel();
    let cancel = CancelHandle::new();
    tx.send(BeatClockCommand::Play(BeatClockPlay {
        ticks: Arc::new((0..48).map(|k| Duration::from_millis(k * 20)).collect()),
        start_time: Duration::ZERO,
        playback_delay: Duration::from_secs(1),
        cancel: cancel.clone(),
        clock: clock.clone(),
        sections: None,
        transport: None,
    }))
    .unwrap();
    let join = std::thread::spawn({
        let at = at.clone();
        let clock = clock.clone();
        move || {
            let tempo = Mutex::new(None);
            beat_clock_engine_loop(&mut FirstClock { clock, at }, false, &tempo, &rx);
        }
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while at.lock().unwrap().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    cancel.cancel();
    drop(tx);
    join.join().unwrap();
    let first = at.lock().unwrap().expect("no clock was sent");
    assert!(first >= Duration::from_secs(1), "{first:?}");
    // Counted from the notice, it would be 1.6s in.
    assert!(first < Duration::from_millis(1_450), "{first:?}");
}

/// A reopen while a song's beat clock schedule still runs (a clock
/// outlasting the song's audio) waits for the old clock to stop, so the
/// fresh one is handed the tempo that schedule set, not the one before.
#[test]
fn a_reopen_mid_schedule_carries_the_schedule_tempo() {
    // Counts the timing clocks sent, so the test waits for the schedule
    // to have set its tempo rather than for a time that a slow runner
    // (CI runs these under coverage) need not have used to send two.
    struct Counting(Arc<std::sync::atomic::AtomicUsize>);
    impl MidiSender for Counting {
        fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
            if bytes == [0xF8] {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            Ok(())
        }
    }
    let sent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (tx, rx) = mpsc::channel();
    // The previous song left 20ms; this schedule ticks every 10ms.
    let tempo = Arc::new(Mutex::new(Some(Duration::from_millis(20))));
    let engine_tempo = tempo.clone();
    let join = std::thread::spawn({
        let sent = sent.clone();
        move || {
            beat_clock_engine_loop(&mut Counting(sent), true, &engine_tempo, &rx);
        }
    });
    let clock = PlaybackClock::wall();
    clock.start();
    let ticks: Vec<Duration> = (0..1000).map(|i| Duration::from_millis(10 * i)).collect();
    let cancel = CancelHandle::new();
    tx.send(BeatClockCommand::Play(BeatClockPlay {
        ticks: Arc::new(ticks),
        start_time: Duration::ZERO,
        playback_delay: Duration::ZERO,
        cancel: cancel.clone(),
        clock,
        sections: None,
        transport: None,
    }))
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while sent.load(std::sync::atomic::Ordering::SeqCst) < 3 {
        assert!(
            std::time::Instant::now() < deadline,
            "the schedule never ticked"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut old = Device::new_default("test".to_string());
    old.beat_clock_engine = Some(Arc::new(BeatClockEngine {
        tx,
        join: Mutex::new(Some(join)),
        tempo,
    }));
    let device = Reopenable::with_device(config::Midi::new("test", None), old);
    let mut handed = None;
    device
        .reopen_with(|held| {
            handed = Some(held);
            Ok(device_holding("test", held))
        })
        .unwrap();
    assert_eq!(
        handed,
        Some(Some(Duration::from_millis(10))),
        "the fresh clock was handed the tempo from before the running schedule"
    );
}

/// Two reopens racing take turns, so the second stops the clock of the
/// device the first installs, not of the one the first replaced, and the
/// port never carries two clocks.
#[test]
fn racing_reopens_stop_the_clock_actually_replaced() {
    let device = Arc::new(Reopenable::with_device(
        config::Midi::new("test", None),
        device_holding("old", Some(Duration::from_millis(20))),
    ));
    let (fresh1_tx, fresh1_rx) = mpsc::channel();
    let (opening_tx, opening_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let first = std::thread::spawn({
        let device = device.clone();
        move || {
            device
                .reopen_with(|held| {
                    opening_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    let mut fresh = Device::new_default("fresh1".to_string());
                    fresh.beat_clock_engine = Some(Arc::new(BeatClockEngine {
                        tx: fresh1_tx,
                        join: Mutex::new(None),
                        tempo: Arc::new(Mutex::new(held)),
                    }));
                    Ok(fresh)
                })
                .unwrap();
        }
    });
    // The first holds the watch lock, opening its fresh device.
    opening_rx.recv().unwrap();
    let second = std::thread::spawn({
        let device = device.clone();
        move || {
            let mut fresh1_running = None;
            device
                .reopen_with(|held| {
                    fresh1_running = Some(!matches!(
                        fresh1_rx.try_recv(),
                        Ok(BeatClockCommand::Shutdown)
                    ));
                    Ok(device_holding("fresh2", held))
                })
                .unwrap();
            fresh1_running
        }
    });
    // Let the second reach the watch lock, then let the first finish.
    std::thread::sleep(Duration::from_millis(100));
    release_tx.send(()).unwrap();
    first.join().unwrap();
    assert_eq!(
        second.join().unwrap(),
        Some(false),
        "the second reopen opened its device while the first one's clock still ran"
    );
}

/// A reopen that fails mid-schedule has stopped the old clock, and the
/// schedule has set its tempo by then: that, not the one before it, is
/// what the device the reload opens instead goes on with.
#[test]
fn the_fallback_tempo_is_read_after_the_old_clock_stops() {
    use crate::midi::Device as _;
    let (tx, rx) = mpsc::channel();
    let tempo = Arc::new(Mutex::new(Some(Duration::from_millis(20))));
    let engine_tempo = tempo.clone();
    let join = std::thread::spawn(move || {
        beat_clock_engine_loop(&mut Null, true, &engine_tempo, &rx);
    });
    let clock = PlaybackClock::wall();
    clock.start();
    let ticks: Vec<Duration> = (0..1000).map(|i| Duration::from_millis(10 * i)).collect();
    tx.send(BeatClockCommand::Play(BeatClockPlay {
        ticks: Arc::new(ticks),
        start_time: Duration::ZERO,
        playback_delay: Duration::ZERO,
        cancel: CancelHandle::new(),
        clock,
        sections: None,
        transport: None,
    }))
    .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let mut old = Device::new_default("test".to_string());
    old.beat_clock_engine = Some(Arc::new(BeatClockEngine {
        tx,
        join: Mutex::new(Some(join)),
        tempo,
    }));
    let device = Reopenable::with_device(config::Midi::new("test", None), old);
    // As the reload's MIDI phase does it: the reopen, then the tempo for
    // the device opened instead, once the old clock has stopped.
    assert!(device.reopen_with(|_| Err("device gone".into())).is_err());
    assert_eq!(
        device.held_tempo(),
        Some(Duration::from_millis(10)),
        "the fallback would be handed the tempo from before the running schedule"
    );
}

/// A schedule waiting on a go signal that never comes, and is never
/// cancelled, still hears a stop: stopping the clock for a reopen joins
/// its thread, and would otherwise never return.
#[test]
fn stop_returns_while_a_schedule_waits_for_its_start() {
    let (tx, rx) = mpsc::channel();
    let tempo = Arc::new(Mutex::new(Some(Duration::from_millis(20))));
    let engine_tempo = tempo.clone();
    let join = std::thread::spawn(move || {
        beat_clock_engine_loop(&mut Null, true, &engine_tempo, &rx);
    });
    let cancel = CancelHandle::new();
    tx.send(BeatClockCommand::Play(BeatClockPlay {
        ticks: Arc::new(vec![Duration::ZERO]),
        start_time: Duration::ZERO,
        playback_delay: Duration::ZERO,
        cancel: cancel.clone(),
        clock: PlaybackClock::wall(),
        sections: None,
        transport: None,
    }))
    .unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let engine = Arc::new(BeatClockEngine {
        tx,
        join: Mutex::new(Some(join)),
        tempo,
    });
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn({
        let engine = engine.clone();
        move || {
            engine.stop();
            let _ = done_tx.send(());
        }
    });
    let stopped = done_rx.recv_timeout(Duration::from_secs(1)).is_ok();
    cancel.cancel(); // let the thread go either way
    assert!(stopped, "stop() hung on a schedule waiting for its start");
}

#[test]
fn reopen_carries_the_held_tempo_over() {
    let held = Duration::from_millis(20);
    let device = Reopenable::with_device(
        config::Midi::new("test", None),
        device_holding("test", Some(held)),
    );
    let mut given = None;
    device
        .reopen_with(|tempo| {
            given = Some(tempo);
            Ok(device_holding("test", tempo))
        })
        .unwrap();
    assert_eq!(given, Some(Some(held)));
    assert_eq!(device.current().held_tempo(), Some(held));
}

/// The device a reopen replaces was dropped under the device and watch
/// locks, so its beat clock's thread was joined holding both. It is let
/// go of once they are released.
#[test]
fn reopen_drops_the_old_device_outside_its_locks() {
    let reopenable: Arc<std::sync::OnceLock<std::sync::Weak<Reopenable>>> = Arc::default();
    let free = Arc::new(Mutex::new(None));
    let mut old = Device::new_default("test".to_string());
    old.on_drop = Some(Box::new({
        let reopenable = reopenable.clone();
        let free = free.clone();
        move || {
            let device = reopenable.get().and_then(std::sync::Weak::upgrade);
            *free.lock().unwrap() = device
                .map(|device| device.device.try_write().is_ok() && device.watch.try_lock().is_ok());
        }
    }));
    let device = Arc::new(Reopenable::with_device(
        config::Midi::new("test", None),
        old,
    ));
    reopenable.set(Arc::downgrade(&device)).unwrap();

    device
        .reopen_with(|_| Ok(Device::new_default("test".to_string())))
        .unwrap();
    assert_eq!(*free.lock().unwrap(), Some(true), "dropped under its locks");
}

/// A reopen whose fresh device could not take over the watch returned
/// the error with the sender still kept, so the controller read as
/// running while nothing watched. The sender is dropped, and the
/// controller hears its channel close.
#[test]
fn reopen_that_cannot_watch_lets_the_watch_go() {
    use crate::midi::Device as _;

    let device = Reopenable::with_device(
        config::Midi::new("test", None),
        Device::new_default("test".to_string()),
    );
    let before = device.current();
    let (tx, mut rx) = tokio::sync::mpsc::channel(10);
    device.watch_events(tx).unwrap();

    let result = device.reopen_with(|_| {
        let mut fresh = Device::new_default("test".to_string());
        fresh.refuse_watch = true;
        Ok(fresh)
    });
    assert!(result.is_err());
    assert!(Arc::ptr_eq(&before, &device.current()));
    assert!(device.watch.lock().unwrap().is_none(), "the watch was kept");
    assert_eq!(
        rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
    );
}

#[test]
fn stop_watch_events_without_connection() {
    let device = Device::new_default("test".to_string());
    // Should not panic when no connection exists
    <Device as crate::midi::Device>::stop_watch_events(&device);
}

mod emit_tests {
    use super::*;

    #[test]
    fn emit_none_returns_ok() {
        let device = Device::new_default("test".to_string());
        let result = <Device as crate::midi::Device>::emit(&device, None);
        assert!(result.is_ok());
    }

    #[test]
    fn emit_without_output_port_returns_ok() {
        let device = Device::new_default("test".to_string());
        assert!(device.output_port.is_none());
        let event = LiveEvent::Midi {
            channel: 0.into(),
            message: midly::MidiMessage::NoteOn {
                key: u7::new(60),
                vel: u7::new(100),
            },
        };
        // Should return Ok (with a warning logged)
        let result = <Device as crate::midi::Device>::emit(&device, Some(event));
        assert!(result.is_ok());
    }
}

mod play_from_tests {
    use super::*;
    use crate::playsync::CancelHandle;

    fn make_song() -> (tempfile::TempDir, Arc<Song>) {
        let tmp_dir = tempfile::tempdir().unwrap();
        let wav_path = tmp_dir.path().join("test.wav");
        crate::testutil::write_wav(wav_path.clone(), vec![vec![1_i32; 44100]], 44100).unwrap();

        let song_config = config::Song::new(
            "test",
            None,
            None,
            None,
            None,
            None,
            vec![config::Track::new(
                "test".to_string(),
                wav_path.file_name().unwrap().to_str().unwrap(),
                Some(1),
            )],
            std::collections::HashMap::new(),
            Vec::new(),
        );
        let song = Arc::new(crate::songs::Song::new(tmp_dir.path(), &song_config).unwrap());
        (tmp_dir, song)
    }

    #[test]
    fn play_from_without_output_port_returns_ok() {
        let device = Device::new_default("test".to_string());
        let (_tmp_dir, song) = make_song();
        let cancel = CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let clock = PlaybackClock::wall();
        let result = <Device as crate::midi::Device>::play_from(
            &device,
            song,
            crate::playsync::PlaybackSync {
                cancel_handle: cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                clock,
                start_time: Duration::ZERO,
                loop_control: crate::playsync::LoopControl::new(),
            },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn play_from_without_midi_playback_returns_ok() {
        // Song has audio but no MIDI sheet
        let device = Device::new_default("test".to_string());
        let (_tmp_dir, song) = make_song();
        assert!(song.midi_playback().is_none());

        let cancel = CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let clock = PlaybackClock::wall();
        // The no-MIDI-sheet check happens before the output-port check
        let result = <Device as crate::midi::Device>::play_from(
            &device,
            song,
            crate::playsync::PlaybackSync {
                cancel_handle: cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                clock,
                start_time: Duration::ZERO,
                loop_control: crate::playsync::LoopControl::new(),
            },
        );
        assert!(result.is_ok());
    }

    /// A song with a tempo and no MIDI file still drives the beat clock:
    /// its schedule goes to the engine, and the song is ready at once.
    #[test]
    fn play_from_hands_a_tempo_only_songs_clock_to_the_engine() {
        let (tx, rx) = mpsc::channel();
        let mut device = Device::new_default("test".to_string());
        device.beat_clock_enabled = true;
        device.beat_clock_engine = Some(Arc::new(BeatClockEngine {
            tx,
            join: Mutex::new(None),
            tempo: Arc::new(Mutex::new(None)),
        }));
        let song = Arc::new(crate::songs::Song::new_for_test_with_tempo(
            "tempo only",
            &["click"],
            Duration::from_secs(1),
            120.0,
        ));
        assert!(song.midi_playback().is_none());

        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
        let result = <Device as crate::midi::Device>::play_from(
            &device,
            song,
            crate::playsync::PlaybackSync {
                cancel_handle: CancelHandle::new(),
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                clock: PlaybackClock::wall(),
                start_time: Duration::from_millis(250),
                loop_control: crate::playsync::LoopControl::new(),
            },
        );
        assert!(result.is_ok(), "{result:?}");
        assert!(ready_rx.try_recv().is_ok(), "the song was never ready");
        let Ok(BeatClockCommand::Play(play)) = rx.try_recv() else {
            panic!("no schedule reached the engine");
        };
        assert_eq!(play.ticks.len(), 48);
        assert_eq!(
            play.ticks[1] - play.ticks[0],
            Duration::from_nanos(20_833_333)
        );
        assert_eq!(play.start_time, Duration::from_millis(250));
    }

    /// Without the beat clock, a song with no MIDI file has nothing for
    /// the MIDI device to do.
    #[test]
    fn play_from_ignores_a_tempo_only_song_without_a_beat_clock() {
        let device = Device::new_default("test".to_string());
        let song = Arc::new(crate::songs::Song::new_for_test_with_tempo(
            "tempo only",
            &["click"],
            Duration::from_secs(1),
            120.0,
        ));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
        let result = <Device as crate::midi::Device>::play_from(
            &device,
            song,
            crate::playsync::PlaybackSync {
                cancel_handle: CancelHandle::new(),
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                clock: PlaybackClock::wall(),
                start_time: Duration::ZERO,
                loop_control: crate::playsync::LoopControl::new(),
            },
        );
        assert!(result.is_ok());
        assert!(ready_rx.try_recv().is_ok());
    }

    #[test]
    fn play_from_without_midi_playback_sends_ready() {
        let device = Device::new_default("test".to_string());
        let (_tmp_dir, song) = make_song();
        assert!(song.midi_playback().is_none());

        let cancel = CancelHandle::new();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
        let clock = PlaybackClock::wall();
        let result = <Device as crate::midi::Device>::play_from(
            &device,
            song,
            crate::playsync::PlaybackSync {
                cancel_handle: cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                clock,
                start_time: Duration::ZERO,
                loop_control: crate::playsync::LoopControl::new(),
            },
        );
        assert!(result.is_ok());
        assert!(
            ready_rx.try_recv().is_ok(),
            "play_from must send ready signal even when the song has no MIDI playback"
        );
    }
}

mod play_precomputed_tests {
    use super::*;
    use crate::midi::playback::{PrecomputedMidi, TimedMidiEvent};
    use crate::playsync::CancelHandle;
    /// Helper: run play_streams with common defaults for tests.
    fn run_play(
        midi: &PrecomputedMidi,
        start: Duration,
        sender: &mut MockSender,
        cancel: &CancelHandle,
        exclude: &HashSet<u8>,
        clock: &PlaybackClock,
    ) {
        // The notes keep to the clock: running.
        if !clock.has_started() {
            clock.start();
        }
        // No loops: the log is closed from the start.
        let log = Arc::new(crate::playsync::SectionJumpLog::default());
        log.close();
        let time = RealTime {
            wall: clock.clone(),
            reference: None,
        };
        play_streams(
            &NoteStreams {
                precomputed: midi,
                start_time: start,
                cancel_handle: cancel,
                loop_break: &AtomicBool::new(false),
                active_section: &parking_lot::RwLock::new(None),
                exclude_channels: exclude,
                time: &time,
                delay: Duration::ZERO,
                transport: None,
            },
            &mut crate::playsync::JumpCursor::new(log),
            sender,
        );
    }

    #[test]
    fn plays_all_events() {
        let midi = make_events(&[0, 0, 0]);
        let cancel = CancelHandle::new();
        let exclude = HashSet::new();
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();

        run_play(
            &midi,
            Duration::ZERO,
            &mut sender,
            &cancel,
            &exclude,
            &clock,
        );

        let sent = sender.sent.lock();
        assert_eq!(sent.len(), 3);
    }

    #[test]
    fn respects_start_time() {
        // Events at 0ms, 100ms, 200ms. Start from 100ms → skip first event.
        let midi = make_events(&[0, 100, 200]);
        let cancel = CancelHandle::new();
        let exclude = HashSet::new();
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();

        run_play(
            &midi,
            Duration::from_millis(100),
            &mut sender,
            &cancel,
            &exclude,
            &clock,
        );

        let sent = sender.sent.lock();
        assert_eq!(sent.len(), 2);
    }

    #[test]
    fn excludes_channels() {
        let events = vec![
            TimedMidiEvent {
                time: Duration::ZERO,
                channel: 0,
                message: midly::MidiMessage::NoteOn {
                    key: u7::new(60),
                    vel: u7::new(100),
                },
            },
            TimedMidiEvent {
                time: Duration::ZERO,
                channel: 5,
                message: midly::MidiMessage::NoteOn {
                    key: u7::new(62),
                    vel: u7::new(100),
                },
            },
            TimedMidiEvent {
                time: Duration::ZERO,
                channel: 0,
                message: midly::MidiMessage::NoteOn {
                    key: u7::new(64),
                    vel: u7::new(100),
                },
            },
        ];
        let midi = PrecomputedMidi::from_events(events);
        let cancel = CancelHandle::new();
        let exclude = HashSet::from([5]);
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();

        run_play(
            &midi,
            Duration::ZERO,
            &mut sender,
            &cancel,
            &exclude,
            &clock,
        );

        let sent = sender.sent.lock();
        assert_eq!(sent.len(), 2); // Channel 5 excluded
    }

    #[test]
    fn stops_on_cancel() {
        // Create events spread over time — cancel before they all play
        let midi = make_events(&[0, 500, 1000]);
        let cancel = CancelHandle::new();
        let exclude = HashSet::new();
        let mut sender = MockSender::new();

        // Cancel immediately
        cancel.cancel();

        let clock = PlaybackClock::wall();
        run_play(
            &midi,
            Duration::ZERO,
            &mut sender,
            &cancel,
            &exclude,
            &clock,
        );

        let sent = sender.sent.lock();
        assert_eq!(sent.len(), 0);
    }

    #[test]
    fn empty_events() {
        let midi = PrecomputedMidi::from_events(Vec::new());
        let cancel = CancelHandle::new();
        let exclude = HashSet::new();
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();

        run_play(
            &midi,
            Duration::ZERO,
            &mut sender,
            &cancel,
            &exclude,
            &clock,
        );

        let sent = sender.sent.lock();
        assert!(sent.is_empty());
    }

    #[test]
    fn send_failure_continues() {
        let midi = make_events(&[0, 0, 0]);
        let cancel = CancelHandle::new();
        let exclude = HashSet::new();
        let mut sender = MockSender::failing();
        let clock = PlaybackClock::wall();

        // Should not panic — errors are logged but playback continues
        run_play(
            &midi,
            Duration::ZERO,
            &mut sender,
            &cancel,
            &exclude,
            &clock,
        );
    }

    #[test]
    fn serialized_bytes_are_correct() {
        let events = vec![TimedMidiEvent {
            time: Duration::ZERO,
            channel: 3,
            message: midly::MidiMessage::NoteOn {
                key: u7::new(72),
                vel: u7::new(64),
            },
        }];
        let midi = PrecomputedMidi::from_events(events);
        let cancel = CancelHandle::new();
        let exclude = HashSet::new();
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();

        run_play(
            &midi,
            Duration::ZERO,
            &mut sender,
            &cancel,
            &exclude,
            &clock,
        );

        let sent = sender.sent.lock();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], vec![0x93, 72, 64]); // NoteOn ch3, key 72, vel 64
    }
}

mod watch_events_tests {
    use super::*;

    #[test]
    fn watch_events_without_input_port_returns_ok() {
        let device = Device::new_default("test".to_string());
        assert!(device.input_port.is_none());
        let (tx, _rx) = tokio::sync::mpsc::channel(10);
        // Should return Ok (with a warning) when no input port
        let result = <Device as crate::midi::Device>::watch_events(&device, tx);
        assert!(result.is_ok());
    }
}

mod run_playback_tests {
    use super::*;
    use crate::midi::playback::PrecomputedMidi;
    use crate::playsync::CancelHandle;
    /// Records when on the clock each note was sent, and which; the
    /// note-offs apart.
    struct TimedSender {
        clock: PlaybackClock,
        sent: Vec<(f64, u8)>,
        offs: Vec<(f64, u8)>,
    }

    impl TimedSender {
        fn new(clock: &PlaybackClock) -> Self {
            Self {
                clock: clock.clone(),
                sent: Vec::new(),
                offs: Vec::new(),
            }
        }
    }

    impl MidiSender for TimedSender {
        fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
            let at = self.clock.elapsed().as_secs_f64();
            if bytes[0] & 0xF0 == 0x80 {
                self.offs.push((at, bytes[1]));
            } else {
                self.sent.push((at, bytes[1]));
            }
            Ok(())
        }
    }

    /// A looping song's passes as the audio commits them: a jump at each
    /// of `at_ms`, from `from_ms` back to the top, with `overlap_ms`. The
    /// log is closed: no more come.
    fn passes(
        at_ms: &[u64],
        from_ms: u64,
        overlap_ms: u64,
    ) -> Arc<crate::playsync::SectionJumpLog> {
        let log = Arc::new(crate::playsync::SectionJumpLog::default());
        for &at in at_ms {
            log.push(crate::section_loop::SectionJump {
                at: Duration::from_millis(at),
                from: Duration::from_millis(from_ms),
                to: Duration::ZERO,
                overlap: Duration::from_millis(overlap_ms),
                pass: true,
            });
        }
        log.close();
        log
    }

    /// A crossfading loop's passes overlap: at each pass jump in the log,
    /// the pass it leaves plays on for the overlap, its notes in time
    /// with the new pass's, older pass first at the same moment.
    #[test]
    fn crossfading_loop_overlaps_its_passes() {
        // A 1 s song with a 0.4 s overlap: a pass every 0.6 s, notes
        // 60, 61, 62 at 0, 0.3 and 0.9 s into each.
        let midi = make_events(&[0, 300, 900]);
        let cancel = CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let exclude = HashSet::new();
        let clock = PlaybackClock::wall();
        clock.start();
        let mut timed = TimedSender::new(&clock);
        // The loop is broken as the last note expected goes out (1.5 s),
        // by the sender, not by a thread watching the clock: a thread a
        // loaded machine held up past the next pass's first note (1.8 s)
        // let that note out too.
        struct BreakAfter<'a> {
            timed: &'a mut TimedSender,
            notes: usize,
            loop_break: Arc<AtomicBool>,
        }
        impl MidiSender for BreakAfter<'_> {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                self.timed.send(bytes)?;
                if self.timed.sent.len() == self.notes {
                    self.loop_break.store(true, Ordering::Relaxed);
                }
                Ok(())
            }
        }
        let loop_break = Arc::new(AtomicBool::new(false));
        let mut sender = BreakAfter {
            timed: &mut timed,
            notes: 8,
            loop_break: loop_break.clone(),
        };

        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: &midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel_handle: &cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: Arc::new(AtomicBool::new(false)),
                exclude_channels: &exclude,
                clock: &clock,
                loop_break: loop_break.clone(),
                active_section: Arc::new(parking_lot::RwLock::new(None)),
                section_jumps: passes(&[600, 1_200], 600, 400),
                transport: None,
            },
        );

        // Pass 0 at 0, pass 1 at 0.6, pass 2 at 1.2: the first pass's
        // last note (0.9) lands with the second's second (0.6 + 0.3).
        let expected = [
            (0.0, 60),
            (0.3, 61),
            (0.6, 60),
            (0.9, 62),
            (0.9, 61),
            (1.2, 60),
            (1.5, 62),
            (1.5, 61),
        ];
        let sent = &timed.sent;
        assert_eq!(sent.len(), expected.len(), "{sent:?}");
        // Never early, and late by less than a third of the 0.3 s between
        // notes: a loaded machine's thread waking late cannot put a note
        // nearer another's time than its own.
        for ((at, note), (want_at, want_note)) in sent.iter().zip(expected) {
            assert!(*at - want_at > -0.03 && *at - want_at < 0.1, "{sent:?}");
            assert_eq!(*note, want_note, "{sent:?}");
        }
    }

    /// A plain loop's passes (no overlap) start as the log says: on the
    /// song's length, not when its MIDI runs out.
    #[test]
    fn a_loop_restarts_on_the_songs_length_not_its_midis() {
        // A 0.5 s song whose MIDI is done by 0.1 s: each pass starts on
        // the half second, as the audio's does, rather than as soon as
        // the last note has played.
        let midi = make_events(&[0, 100]);
        let cancel = CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let exclude = HashSet::new();
        let clock = PlaybackClock::wall();
        clock.start();
        let mut sender = TimedSender::new(&clock);
        let loop_break = Arc::new(AtomicBool::new(false));
        let breaker = {
            let loop_break = loop_break.clone();
            let clock = clock.clone();
            std::thread::spawn(move || {
                while clock.elapsed() < Duration::from_millis(1_250) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                loop_break.store(true, Ordering::Relaxed);
            })
        };

        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: &midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel_handle: &cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: Arc::new(AtomicBool::new(false)),
                exclude_channels: &exclude,
                clock: &clock,
                loop_break: loop_break.clone(),
                active_section: Arc::new(parking_lot::RwLock::new(None)),
                section_jumps: passes(&[500, 1_000], 500, 0),
                transport: None,
            },
        );
        breaker.join().unwrap();

        let expected = [
            (0.0, 60),
            (0.1, 61),
            (0.5, 60),
            (0.6, 61),
            (1.0, 60),
            (1.1, 61),
        ];
        let sent = &sender.sent;
        assert_eq!(sent.len(), expected.len(), "{sent:?}");
        // Timed from the first note: wall-clock playback on a loaded
        // runner (CI runs these under coverage) starts tens of
        // milliseconds late. Restarting on the MIDI's length instead
        // would put the second and third passes 400 ms early.
        let first = sent[0].0;
        for ((at, note), (want_at, want_note)) in sent.iter().zip(expected) {
            assert!((at - first - want_at).abs() < 0.1, "{sent:?}");
            assert_eq!(*note, want_note, "{sent:?}");
        }
    }

    /// A looping song's pass plays all of its own notes: one on the
    /// pass's end (a MIDI-only song's length is its last event, and a
    /// DAW export's last note-off lands on the audio's end) or past it
    /// (a release tail) still goes out, as `play_looping_passes` and the
    /// restart loop sent it; dropping it leaves the note hanging every
    /// pass.
    #[test]
    fn a_pass_plays_its_notes_on_and_past_its_end() {
        // A 200 ms song looping without a crossfade, its MIDI 60 at 0,
        // 61 at 100, 62 on the end (200) and 63 past it (250).
        let midi = make_events(&[0, 100, 200, 250]);
        let cancel = CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let exclude = HashSet::new();
        let clock = PlaybackClock::wall();
        clock.start();
        let mut sender = TimedSender::new(&clock);
        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: &midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel_handle: &cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: Arc::new(AtomicBool::new(false)),
                exclude_channels: &exclude,
                clock: &clock,
                loop_break: Arc::new(AtomicBool::new(false)),
                active_section: Arc::new(parking_lot::RwLock::new(None)),
                section_jumps: passes(&[200], 200, 0),
                transport: None,
            },
        );
        let notes: Vec<u8> = sender.sent.iter().map(|(_, note)| *note).collect();
        // Each of the two passes sends 62 and 63 once.
        assert_eq!(notes.iter().filter(|n| **n == 62).count(), 2, "{notes:?}");
        assert_eq!(notes.iter().filter(|n| **n == 63).count(), 2, "{notes:?}");
    }

    /// A section's jump cuts the run of notes it leaves: the notes that
    /// run has on are let go there, rather than left hanging.
    #[test]
    fn a_section_jump_lets_go_of_the_notes_it_cuts() {
        // Notes 60 at 0 and 61 at 100, both held; the section 0-300
        // jumps back once, at 300, and the song ends at 400.
        let midi = make_events(&[0, 100]);
        let cancel = CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let exclude = HashSet::new();
        let clock = PlaybackClock::wall();
        clock.start();
        let mut sender = TimedSender::new(&clock);
        let log = Arc::new(crate::playsync::SectionJumpLog::default());
        log.push(crate::section_loop::SectionJump {
            at: Duration::from_millis(300),
            from: Duration::from_millis(300),
            to: Duration::ZERO,
            overlap: Duration::ZERO,
            pass: false,
        });
        log.close();
        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: &midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel_handle: &cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: Arc::new(AtomicBool::new(false)),
                exclude_channels: &exclude,
                clock: &clock,
                loop_break: Arc::new(AtomicBool::new(false)),
                active_section: Arc::new(parking_lot::RwLock::new(None)),
                section_jumps: log,
                transport: None,
            },
        );
        let notes: Vec<u8> = sender.sent.iter().map(|(_, n)| *n).collect();
        assert_eq!(notes, vec![60, 61, 60, 61]);
        let mut offs: Vec<u8> = sender.offs.iter().map(|(_, n)| *n).collect();
        offs.sort();
        assert_eq!(offs, vec![60, 61], "let go at the cut, once each");
        for (at, _) in &sender.offs {
            assert!(*at >= 0.29, "not before the cut: {at}");
        }
    }

    fn review_run(
        midi: &PrecomputedMidi,
        jumps: &[(u64, u64, u64, bool)],
        exclude: &HashSet<u8>,
    ) -> Vec<Vec<u8>> {
        let cancel = CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let clock = PlaybackClock::wall();
        clock.start();
        let mut sender = MockSender::new();
        let log = Arc::new(crate::playsync::SectionJumpLog::default());
        for &(at, from, to, pass) in jumps {
            log.push(crate::section_loop::SectionJump {
                at: Duration::from_millis(at),
                from: Duration::from_millis(from),
                to: Duration::from_millis(to),
                overlap: Duration::ZERO,
                pass,
            });
        }
        log.close();
        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel_handle: &cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: Arc::new(AtomicBool::new(false)),
                exclude_channels: exclude,
                clock: &clock,
                loop_break: Arc::new(AtomicBool::new(false)),
                active_section: Arc::new(parking_lot::RwLock::new(None)),
                section_jumps: log,
                transport: None,
            },
        );
        sender.sent.into_inner()
    }

    /// REVIEW: the cut's note-off goes before the new stream's note-on of
    /// the same key at the seam.
    #[test]
    fn review_cut_off_precedes_restart_on() {
        let midi = make_events(&[0]);
        let sent = review_run(&midi, &[(300, 300, 0, false)], &HashSet::new());
        let shape: Vec<(u8, u8)> = sent.iter().map(|b| (b[0] & 0xF0, b[1])).collect();
        assert_eq!(shape, vec![(0x90, 60), (0x80, 60), (0x90, 60)]);
    }

    /// REVIEW: notes on an excluded channel are never sent, so never let go.
    #[test]
    fn review_cut_skips_excluded_channels() {
        use crate::midi::playback::TimedMidiEvent;
        let midi = PrecomputedMidi::from_events(vec![
            TimedMidiEvent {
                time: Duration::ZERO,
                channel: 9,
                message: midly::MidiMessage::NoteOn {
                    key: u7::new(36),
                    vel: u7::new(100),
                },
            },
            TimedMidiEvent {
                time: Duration::ZERO,
                channel: 2,
                message: midly::MidiMessage::NoteOn {
                    key: u7::new(60),
                    vel: u7::new(100),
                },
            },
            TimedMidiEvent {
                time: Duration::from_millis(50),
                channel: 2,
                message: midly::MidiMessage::NoteOn {
                    key: u7::new(61),
                    vel: u7::new(0),
                },
            },
        ]);
        let sent = review_run(&midi, &[(200, 200, 0, false)], &HashSet::from([9]));
        let offs: Vec<Vec<u8>> = sent
            .iter()
            .filter(|b| b[0] & 0xF0 == 0x80)
            .cloned()
            .collect();
        assert_eq!(offs, vec![vec![0x82, 60, 0]]);
    }

    /// REVIEW: a section jump right after a pass jump, the older pass
    /// still playing its tail: the tail plays on, the section's stream is cut.
    #[test]
    fn review_section_after_pass_with_tail() {
        // 200ms song: 60@0, 61@150 (held over the seam), 62@260 (tail past end).
        let midi = make_events(&[0, 150, 260]);
        // pass at 200 -> 0; then a section 0-100 of the new pass jumps at 300 (song 100).
        let sent = review_run(
            &midi,
            &[(200, 200, 0, true), (300, 100, 0, false)],
            &HashSet::new(),
        );
        let shape: Vec<(u8, u8)> = sent.iter().map(|b| (b[0] & 0xF0, b[1])).collect();
        // pass 1: 60, 61; pass 2: 60 @200; pass1 tail 62 @260; cut at 300: off 60; restart 60 @300 ...
        assert_eq!(
            shape.iter().filter(|s| **s == (0x80, 60)).count(),
            1,
            "{shape:?}"
        );
        assert_eq!(
            shape.iter().filter(|s| **s == (0x80, 61)).count(),
            0,
            "{shape:?}"
        );
    }

    #[test]
    fn an_armed_section_loops_its_midi_in_a_looping_song() {
        // A 1 s looping song with the section 0-0.3 s armed: its notes at
        // 0 and 0.1 s repeat every 0.3 s. The first play stops at the
        // note past the section (0.5 s), long before the clock reaches
        // the section end; MIDI used to take that as the end of the song.
        let midi = make_events(&[0, 100, 500]);
        let cancel = CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let exclude = HashSet::new();
        let clock = PlaybackClock::wall();
        clock.start();
        let mut sender = TimedSender::new(&clock);
        let loop_break = Arc::new(AtomicBool::new(false));
        let breaker = {
            let loop_break = loop_break.clone();
            let clock = clock.clone();
            std::thread::spawn(move || {
                while clock.elapsed() < Duration::from_millis(750) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                loop_break.store(true, Ordering::Relaxed);
            })
        };

        let active_section = Arc::new(parking_lot::RwLock::new(Some(
            crate::player::SectionBounds {
                name: "intro".to_string(),
                start_time: Duration::ZERO,
                end_time: Duration::from_millis(300),
            },
        )));
        let section_loop_break = Arc::new(AtomicBool::new(false));
        let (log, done) =
            publish_jumps_for_test(&clock, Duration::ZERO, &active_section, &section_loop_break);
        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: &midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel_handle: &cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: Arc::new(AtomicBool::new(false)),
                exclude_channels: &exclude,
                clock: &clock,
                loop_break: loop_break.clone(),
                active_section,
                section_jumps: log,
                transport: None,
            },
        );
        breaker.join().unwrap();
        done.store(true, Ordering::Relaxed);

        let notes: Vec<u8> = sender.sent.iter().map(|(_, note)| *note).collect();
        assert_eq!(notes, vec![60, 61, 60, 61, 60, 61], "{:?}", sender.sent);
        // The repeats come on the section's grid, from the clock's start:
        // 0.3 s and 0.6 s, where looping on the first play's end would put
        // them at 0.5 s and 1.0 s. A loaded runner (CI runs under
        // coverage) can send any note late, the first included, so the
        // bounds only rule that out; timing them from the first note
        // failed when it alone was late.
        let at = |i: usize| sender.sent[i].0;
        assert!(at(2) >= 0.28 && at(2) < 0.48, "{:?}", sender.sent);
        assert!(at(4) >= 0.58 && at(4) < 0.95, "{:?}", sender.sent);
    }

    /// Once a section loop is broken (as `stop_section_loop` breaks it:
    /// the break set, the section cleared), the notes play on from the
    /// section's end, as the audio and beat clock do; they used to stop
    /// there for good.
    #[test]
    fn notes_play_on_after_a_section_loop_is_broken() {
        // Notes at 0, 100 and 400ms; the section 0-300ms loops once
        // (300ms), is broken at 350ms, and the song goes on: the note at
        // 400ms comes at 700ms.
        let midi = make_events(&[0, 100, 400]);
        let cancel = CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let exclude = HashSet::new();
        let clock = PlaybackClock::wall();
        clock.start();
        let mut sender = TimedSender::new(&clock);
        let active_section = Arc::new(parking_lot::RwLock::new(Some(
            crate::player::SectionBounds {
                name: "intro".to_string(),
                start_time: Duration::ZERO,
                end_time: Duration::from_millis(300),
            },
        )));
        let section_loop_break = Arc::new(AtomicBool::new(false));
        let breaker = {
            let active_section = active_section.clone();
            let section_loop_break = section_loop_break.clone();
            let clock = clock.clone();
            std::thread::spawn(move || {
                while clock.elapsed() < Duration::from_millis(350) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                section_loop_break.store(true, Ordering::Relaxed);
                *active_section.write() = None;
            })
        };
        let (log, done) =
            publish_jumps_for_test(&clock, Duration::ZERO, &active_section, &section_loop_break);
        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: &midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel_handle: &cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: Arc::new(AtomicBool::new(false)),
                exclude_channels: &exclude,
                clock: &clock,
                loop_break: Arc::new(AtomicBool::new(false)),
                active_section,
                section_jumps: log,
                transport: None,
            },
        );
        breaker.join().unwrap();
        done.store(true, Ordering::Relaxed);
        let notes: Vec<u8> = sender.sent.iter().map(|(_, note)| *note).collect();
        assert_eq!(notes, vec![60, 61, 60, 61, 62], "{:?}", sender.sent);
    }

    /// Runs `run_playback` over notes at `at_ms` (60, 61, ...) with
    /// `section` armed from the top, while `driver` changes the section
    /// loop state on the clock; returns the notes sent.
    fn play_with_sections(
        at_ms: &[u64],
        section: (u64, u64),
        driver: impl FnOnce(
                PlaybackClock,
                Arc<parking_lot::RwLock<Option<crate::player::SectionBounds>>>,
                Arc<AtomicBool>,
            ) + Send
            + 'static,
    ) -> Vec<(f64, u8)> {
        let midi = make_events(at_ms);
        let cancel = CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let exclude = HashSet::new();
        let clock = PlaybackClock::wall();
        clock.start();
        let mut sender = TimedSender::new(&clock);
        let active_section = Arc::new(parking_lot::RwLock::new(Some(
            crate::player::SectionBounds {
                name: "a".to_string(),
                start_time: Duration::from_millis(section.0),
                end_time: Duration::from_millis(section.1),
            },
        )));
        let section_loop_break = Arc::new(AtomicBool::new(false));
        let driver = {
            let clock = clock.clone();
            let active_section = active_section.clone();
            let section_loop_break = section_loop_break.clone();
            std::thread::spawn(move || driver(clock, active_section, section_loop_break))
        };
        let (log, done) =
            publish_jumps_for_test(&clock, Duration::ZERO, &active_section, &section_loop_break);
        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: &midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel_handle: &cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: Arc::new(AtomicBool::new(false)),
                exclude_channels: &exclude,
                clock: &clock,
                loop_break: Arc::new(AtomicBool::new(false)),
                active_section,
                section_jumps: log,
                transport: None,
            },
        );
        driver.join().unwrap();
        done.store(true, Ordering::Relaxed);
        sender.sent
    }

    /// While the log is open the notes follow it past their last note: a
    /// section armed after it, its jump committed then, plays them again
    /// (#606); they end once the log closes. They used to end with their
    /// last note.
    #[test]
    fn notes_replay_after_their_last_note_while_the_log_is_open() {
        let midi = make_events(&[0, 50]);
        let cancel = CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let exclude = HashSet::new();
        let clock = PlaybackClock::wall();
        clock.start();
        let mut sender = TimedSender::new(&clock);
        let log = Arc::new(crate::playsync::SectionJumpLog::default());
        let audio = {
            let log = log.clone();
            let clock = clock.clone();
            std::thread::spawn(move || {
                // The section 0-300ms armed at 200ms, past the last note:
                // its jump at its end.
                wait_until(&clock, 200);
                log.push(crate::section_loop::SectionJump {
                    at: Duration::from_millis(300),
                    from: Duration::from_millis(300),
                    to: Duration::ZERO,
                    overlap: Duration::ZERO,
                    pass: false,
                });
                // Broken, and the song over: the log closes.
                wait_until(&clock, 450);
                log.close();
            })
        };
        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: &midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel_handle: &cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: Arc::new(AtomicBool::new(false)),
                exclude_channels: &exclude,
                clock: &clock,
                loop_break: Arc::new(AtomicBool::new(false)),
                active_section: Arc::new(parking_lot::RwLock::new(None)),
                section_jumps: log.clone(),
                transport: None,
            },
        );
        audio.join().unwrap();
        assert!(log.is_closed(), "the notes ended before the log closed");
        let notes: Vec<u8> = sender.sent.iter().map(|(_, note)| *note).collect();
        assert_eq!(notes, vec![60, 61, 60, 61], "{:?}", sender.sent);
    }

    /// A section armed in a song looping with a crossfade loops on its
    /// own seam, the notes with it, whether it reaches into the
    /// crossfade's tail or ends on the song's end: the whole-song pass
    /// never starts under it (it used to be refused in such a song). The
    /// jumps come from the playback's publisher, deciding as the audio
    /// does.
    #[test]
    fn a_section_in_a_crossfading_song_loops_without_a_pass_under_it() {
        // A 1s song crossfading over its last 0.4s (passes would start
        // at 0.6s); notes at 0, 0.45, 0.55 and 0.65s.
        for (start, end) in [(400, 700), (500, 1_000)] {
            let midi = make_events(&[0, 450, 550, 650]);
            let cancel = CancelHandle::new();
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let exclude = HashSet::new();
            let clock = PlaybackClock::wall();
            clock.start();
            let mut sender = TimedSender::new(&clock);
            let loop_control = crate::playsync::LoopControl::new();
            *loop_control.active_section.write() = Some(crate::player::SectionBounds {
                name: "a".to_string(),
                start_time: Duration::from_millis(start),
                end_time: Duration::from_millis(end),
            });
            let log = loop_control.section_jumps.clone();
            let loop_break = loop_control.loop_break.clone();
            let done = Arc::new(AtomicBool::new(false));
            let song = Arc::new(crate::songs::Song::new_for_test_crossfading(
                "Crossfading",
                Duration::from_secs(1),
                Duration::from_millis(400),
            ));
            let publisher = crate::section_loop::JumpPublisher {
                clock: clock.clone(),
                start_time: Duration::ZERO,
                cancel: cancel.clone(),
                done: done.clone(),
                audio_gone: None,
                loop_control: loop_control.clone(),
                song: Some(song),
            };
            let publisher = thread::spawn(move || crate::section_loop::publish_jumps(publisher));
            let breaker = {
                let loop_break = loop_break.clone();
                let clock = clock.clone();
                thread::spawn(move || {
                    wait_until(&clock, 2_300);
                    loop_break.store(true, Ordering::Relaxed);
                })
            };
            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel,
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: Arc::new(AtomicBool::new(false)),
                    exclude_channels: &exclude,
                    clock: &clock,
                    loop_break,
                    active_section: loop_control.active_section.clone(),
                    section_jumps: log.clone(),
                    transport: None,
                },
            );
            breaker.join().unwrap();
            done.store(true, Ordering::Relaxed);
            publisher.join().unwrap();
            let mut cursor = crate::playsync::JumpCursor::new(log);
            let mut jumps = 0;
            while let Some(jump) = cursor.due(Duration::MAX) {
                jumps += 1;
                assert_eq!(
                    (jump.to, jump.overlap),
                    (Duration::from_millis(start), Duration::ZERO),
                    "section {start}-{end}ms: a pass under it: {jump:?}"
                );
            }
            assert!(jumps >= 2, "section {start}-{end}ms: {jumps} jumps");
            let notes: Vec<u8> = sender.sent.iter().map(|(_, note)| *note).collect();
            // The top's note once, never again (no pass); the section's
            // notes round and round.
            assert_eq!(
                notes.iter().filter(|note| **note == 60).count(),
                1,
                "section {start}-{end}ms: {notes:?}"
            );
            assert!(
                notes.iter().filter(|note| **note == 62).count() >= 3,
                "section {start}-{end}ms: {notes:?}"
            );
        }
    }

    fn wait_until(clock: &PlaybackClock, ms: u64) {
        while clock.elapsed() < Duration::from_millis(ms) {
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Broken before it ever went round (as `stop_section_loop` breaks
    /// it), the section's end is played through, as the audio plays it:
    /// the notes past it still come.
    #[test]
    fn notes_play_on_after_a_section_broken_before_it_loops() {
        // Notes at 0, 100 and 400ms; section 0-300ms armed, broken at
        // 200ms: the note at 400ms is still due.
        let sent = play_with_sections(&[0, 100, 400], (0, 300), |clock, active, brk| {
            wait_until(&clock, 200);
            brk.store(true, Ordering::Relaxed);
            *active.write() = None;
        });
        let notes: Vec<u8> = sent.iter().map(|(_, note)| *note).collect();
        assert_eq!(notes, vec![60, 61, 62], "{sent:?}");
    }

    /// Another section armed while one loops (no break): the audio plays
    /// the rest of the song up to the new section's end; so do the notes.
    #[test]
    fn notes_play_on_into_another_section_armed_while_one_loops() {
        // A = 0-300ms, B = 400-600ms. A loops at 300ms; at 350ms B is
        // armed. The song plays on from A's end (clock 600ms) through
        // B, whose notes (400, 500) come at 700 and 800ms, and B goes
        // round at 900ms (notes again at 900, 1000). Broken at 950ms;
        // the note at 700ms in the song comes at 1200ms.
        let sent = play_with_sections(&[0, 100, 400, 500, 700], (0, 300), |clock, active, brk| {
            wait_until(&clock, 350);
            *active.write() = Some(crate::player::SectionBounds {
                name: "b".to_string(),
                start_time: Duration::from_millis(400),
                end_time: Duration::from_millis(600),
            });
            wait_until(&clock, 950);
            brk.store(true, Ordering::Relaxed);
            *active.write() = None;
        });
        let notes: Vec<u8> = sent.iter().map(|(_, note)| *note).collect();
        assert_eq!(notes, vec![60, 61, 60, 61, 62, 63, 62, 63, 64], "{sent:?}");
    }

    #[test]
    fn normal_playback_sets_finished() {
        let midi = make_events(&[0, 0]);
        let cancel = CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let finished = Arc::new(AtomicBool::new(false));
        let exclude = HashSet::new();
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();
        clock.start();

        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: &midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel_handle: &cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: finished.clone(),
                exclude_channels: &exclude,
                clock: &clock,
                loop_break: Arc::new(AtomicBool::new(false)),
                active_section: Arc::new(parking_lot::RwLock::new(None)),
                section_jumps: passes(&[], 0, 0),
                transport: None,
            },
        );

        assert!(finished.load(Ordering::Relaxed));
        assert_eq!(sender.sent.lock().len(), 2);
    }

    #[test]
    fn cancel_before_barrier_sets_finished() {
        let midi = make_events(&[0]);
        let cancel = CancelHandle::new();
        cancel.cancel();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let finished = Arc::new(AtomicBool::new(false));
        let exclude = HashSet::new();
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();
        clock.start();

        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: &midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel_handle: &cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: finished.clone(),
                exclude_channels: &exclude,
                clock: &clock,
                loop_break: Arc::new(AtomicBool::new(false)),
                active_section: Arc::new(parking_lot::RwLock::new(None)),
                section_jumps: passes(&[], 0, 0),
                transport: None,
            },
        );

        assert!(finished.load(Ordering::Relaxed));
        // No events should be sent since we cancelled before playback
        assert!(sender.sent.lock().is_empty());
    }

    #[test]
    fn cancel_during_delay_sets_finished() {
        let midi = make_events(&[0]);
        let cancel = CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let finished = Arc::new(AtomicBool::new(false));
        let exclude = HashSet::new();
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();
        clock.start();

        // Use a long delay but cancel from another thread
        let cancel_clone = cancel.clone();
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            cancel_clone.cancel();
        });

        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: &midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::from_secs(10), // Very long delay
                cancel_handle: &cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: finished.clone(),
                exclude_channels: &exclude,
                clock: &clock,
                loop_break: Arc::new(AtomicBool::new(false)),
                active_section: Arc::new(parking_lot::RwLock::new(None)),
                section_jumps: passes(&[], 0, 0),
                transport: None,
            },
        );

        handle.join().unwrap();
        assert!(finished.load(Ordering::Relaxed));
        // Should have been cancelled during delay, no events sent
        assert!(sender.sent.lock().is_empty());
    }

    #[test]
    fn short_delay_completes_and_plays() {
        let midi = make_events(&[0, 0, 0]);
        let cancel = CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let finished = Arc::new(AtomicBool::new(false));
        let exclude = HashSet::new();
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();
        clock.start();

        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: &midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::from_millis(10), // Short delay
                cancel_handle: &cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: finished.clone(),
                exclude_channels: &exclude,
                clock: &clock,
                loop_break: Arc::new(AtomicBool::new(false)),
                active_section: Arc::new(parking_lot::RwLock::new(None)),
                section_jumps: passes(&[], 0, 0),
                transport: None,
            },
        );

        assert!(finished.load(Ordering::Relaxed));
        assert_eq!(sender.sent.lock().len(), 3);
    }

    #[test]
    fn respects_exclude_channels() {
        use crate::midi::playback::TimedMidiEvent;
        let events = vec![
            TimedMidiEvent {
                time: Duration::ZERO,
                channel: 0,
                message: midly::MidiMessage::NoteOn {
                    key: u7::new(60),
                    vel: u7::new(100),
                },
            },
            TimedMidiEvent {
                time: Duration::ZERO,
                channel: 9, // Excluded
                message: midly::MidiMessage::NoteOn {
                    key: u7::new(62),
                    vel: u7::new(100),
                },
            },
        ];
        let midi = PrecomputedMidi::from_events(events);
        let cancel = CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let finished = Arc::new(AtomicBool::new(false));
        let exclude = HashSet::from([9]);
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();
        clock.start();

        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: &midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel_handle: &cancel,
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: finished.clone(),
                exclude_channels: &exclude,
                clock: &clock,
                loop_break: Arc::new(AtomicBool::new(false)),
                active_section: Arc::new(parking_lot::RwLock::new(None)),
                section_jumps: passes(&[], 0, 0),
                transport: None,
            },
        );

        assert_eq!(sender.sent.lock().len(), 1);
    }

    #[test]
    fn ready_channel_synchronization_works() {
        // run_playback sends on ready_tx, then waits for the clock to start.
        // The test thread receives on ready_rx, then starts the clock.
        let midi = make_events(&[0]);
        let cancel = CancelHandle::new();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
        let finished = Arc::new(AtomicBool::new(false));
        let exclude = HashSet::new();
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();

        let finished_clone = finished.clone();
        let cancel_clone = cancel.clone();
        let clock_clone = clock.clone();

        let handle = thread::spawn(move || {
            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel_clone,
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: finished_clone,
                    exclude_channels: &exclude,
                    clock: &clock_clone,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_jumps: passes(&[], 0, 0),
                    transport: None,
                },
            );
            sender
        });

        // Wait for run_playback to signal readiness
        ready_rx.recv().unwrap();
        assert!(!finished.load(Ordering::Relaxed));

        // Start the clock to let playback proceed
        clock.start();

        let sender = handle.join().unwrap();
        assert!(finished.load(Ordering::Relaxed));
        assert_eq!(sender.sent.lock().len(), 1);
    }

    /// Held by a test timing notes against a simulated audio device on
    /// real threads: one at a time, their feeders' spinning not taking
    /// each other's cores.
    static AUDIO_TIMING: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

    /// When an audio device's callbacks come, in seconds from the start,
    /// for one taking `frames` a time at 48kHz and running at `rate`
    /// against the wall clock: on time, or each up to `jitter` seconds
    /// early or late, one in ten held back to come with the next
    /// (`pairs`), and the first `first_late` late.
    fn callbacks(
        rate: f64,
        frames: u64,
        jitter: f64,
        pairs: bool,
        first_late: f64,
        until: f64,
    ) -> Vec<f64> {
        let period = frames as f64 / 48_000.0 / rate;
        // xorshift: the same arrivals every run.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut random = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut times = Vec::new();
        let mut last: f64 = 0.0;
        for k in 1.. {
            let mut at = k as f64 * period;
            if at > until {
                break;
            }
            at += (random() * 2.0 - 1.0) * jitter;
            if pairs && random() < 0.1 {
                at = (k + 1) as f64 * period;
            }
            if k == 1 {
                at += first_late;
            }
            last = last.max(at.max(0.0));
            times.push(last);
        }
        times
    }

    /// Plays notes at `times` (ms) with a 20ms playback delay on an audio
    /// clock fed `frames` at each of `callbacks` at 48kHz, following
    /// `jumps`; the clock's start and when each note went out.
    fn play_on_audio(
        callbacks: Vec<f64>,
        frames: u64,
        times: &[u64],
        transport: Option<Arc<AtomicBool>>,
        jumps: Arc<crate::playsync::SectionJumpLog>,
    ) -> (std::time::Instant, Vec<std::time::Instant>) {
        play_on_audio_after(20, callbacks, frames, times, transport, jumps)
    }

    /// The callbacks from `at` (seconds) on held back `lost`: the audio
    /// stalled there that long.
    fn stalled(callbacks: Vec<f64>, at: f64, lost: f64) -> Vec<f64> {
        callbacks
            .into_iter()
            .map(|time| if time >= at { time + lost } else { time })
            .collect()
    }

    /// As [`play_on_audio`], with a playback delay of `delay_ms`.
    fn play_on_audio_after(
        delay_ms: u64,
        callbacks: Vec<f64>,
        frames: u64,
        times: &[u64],
        transport: Option<Arc<AtomicBool>>,
        jumps: Arc<crate::playsync::SectionJumpLog>,
    ) -> (std::time::Instant, Vec<std::time::Instant>) {
        let _one_at_a_time = AUDIO_TIMING.lock();
        struct Stamped(Vec<std::time::Instant>);
        impl MidiSender for Stamped {
            fn send(&mut self, _: &[u8]) -> Result<(), Box<dyn Error>> {
                self.0.push(std::time::Instant::now());
                Ok(())
            }
        }
        let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let clock = PlaybackClock::from_sample_counter(counter.clone(), 48_000);
        let done = Arc::new(AtomicBool::new(false));
        let started = Arc::new(std::sync::Barrier::new(2));
        let feeder = thread::spawn({
            let counter = counter.clone();
            let done = done.clone();
            let started = started.clone();
            move || {
                started.wait();
                let start = std::time::Instant::now();
                for at in callbacks {
                    if done.load(Ordering::Relaxed) {
                        break;
                    }
                    let at = Duration::from_secs_f64(at);
                    spin_sleep::sleep(at.saturating_sub(start.elapsed()));
                    counter.fetch_add(frames, Ordering::Relaxed);
                }
            }
        });
        let midi = make_events(times);
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let mut sender = Stamped(Vec::new());
        clock.start();
        started.wait();
        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: &midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::from_millis(delay_ms),
                cancel_handle: &CancelHandle::new(),
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: Arc::new(AtomicBool::new(false)),
                exclude_channels: &HashSet::new(),
                clock: &clock,
                loop_break: Arc::new(AtomicBool::new(false)),
                active_section: Arc::new(parking_lot::RwLock::new(None)),
                section_jumps: jumps,
                transport,
            },
        );
        done.store(true, Ordering::Relaxed);
        feeder.join().unwrap();
        (clock.started_at().unwrap(), sender.0)
    }

    /// How far `sent` is from `due`, either way.
    fn off(sent: std::time::Instant, due: std::time::Instant) -> Duration {
        if sent >= due {
            sent - due
        } else {
            due - sent
        }
    }

    /// A time of a test's own, for the notes' clock: the wall clock
    /// moves only as the notes sleep (each sleep a little long, as a
    /// scheduler makes it: 0.1-0.3ms), and an audio device's counter
    /// moves at given times. Nothing sleeps for real, so a loaded machine
    /// changes nothing.
    struct VirtualTime {
        /// The wall clock, in seconds (as f64 bits).
        now: std::sync::atomic::AtomicU64,
        /// When the counter moves, and to what: seconds of audio.
        moves: Vec<(f64, f64)>,
        /// For each sleep's lateness (xorshift).
        seed: std::sync::atomic::AtomicU64,
        /// How often the counter has been read.
        reads: std::sync::atomic::AtomicUsize,
    }

    impl VirtualTime {
        /// A device taking `frames` at 48kHz at each of `callbacks`
        /// (seconds from the start).
        fn new(callbacks: &[f64], frames: u64) -> Arc<Self> {
            Arc::new(VirtualTime {
                now: std::sync::atomic::AtomicU64::new(0f64.to_bits()),
                moves: callbacks
                    .iter()
                    .enumerate()
                    .map(|(k, at)| (*at, ((k as u64 + 1) * frames) as f64 / 48_000.0))
                    .collect(),
                seed: std::sync::atomic::AtomicU64::new(0x9E37_79B9_7F4A_7C15),
                reads: std::sync::atomic::AtomicUsize::new(0),
            })
        }
    }

    impl NoteTime for VirtualTime {
        fn wall(&self) -> f64 {
            f64::from_bits(self.now.load(Ordering::Relaxed))
        }

        fn reading(&self) -> Option<f64> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            let wall = self.wall();
            let moved = self.moves.partition_point(|(at, _)| *at <= wall);
            Some(moved.checked_sub(1).map_or(0.0, |k| self.moves[k].1))
        }

        fn sleep(&self, length: Duration, _precise: bool) {
            let mut seed = self.seed.load(Ordering::Relaxed);
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            self.seed.store(seed, Ordering::Relaxed);
            let late = 0.0001 + 0.0002 * ((seed >> 11) as f64 / (1u64 << 53) as f64);
            let now = self.wall() + length.as_secs_f64() + late;
            assert!(now < 60.0, "the notes never finished");
            self.now.store(now.to_bits(), Ordering::Relaxed);
        }
    }

    /// Plays notes at `times` (ms) after a `delay_ms` playback delay on a
    /// device taking `frames` at each of `callbacks`, in virtual time,
    /// following `jumps`; when each note went out (seconds from the
    /// start), and how often the counter was read.
    fn play_virtual(
        callbacks: &[f64],
        frames: u64,
        times: &[u64],
        delay_ms: u64,
        jumps: Arc<crate::playsync::SectionJumpLog>,
    ) -> (Vec<f64>, usize) {
        let replays = crate::playsync::JumpCursor::new(jumps.clone())
            .peek()
            .is_some();
        let (sent, reads) = play_virtual_messages(callbacks, frames, times, delay_ms, jumps);
        if !replays {
            assert_eq!(sent.len(), times.len(), "notes missing");
        }
        (sent.into_iter().map(|(at, _)| at).collect(), reads)
    }

    /// As [`play_virtual`], each message with when it went out.
    fn play_virtual_messages(
        callbacks: &[f64],
        frames: u64,
        times: &[u64],
        delay_ms: u64,
        jumps: Arc<crate::playsync::SectionJumpLog>,
    ) -> (Vec<(f64, Vec<u8>)>, usize) {
        struct Sender(Arc<VirtualTime>, Vec<(f64, Vec<u8>)>);
        impl MidiSender for Sender {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                self.1.push((self.0.wall(), bytes.to_vec()));
                Ok(())
            }
        }
        let time = VirtualTime::new(callbacks, frames);
        let midi = make_events(times);
        let mut sender = Sender(time.clone(), Vec::new());
        play_streams(
            &NoteStreams {
                precomputed: &midi,
                start_time: Duration::ZERO,
                cancel_handle: &CancelHandle::new(),
                loop_break: &AtomicBool::new(false),
                active_section: &parking_lot::RwLock::new(None),
                exclude_channels: &HashSet::new(),
                time: &*time,
                delay: Duration::from_millis(delay_ms),
                transport: None,
            },
            &mut crate::playsync::JumpCursor::new(jumps),
            &mut sender,
        );
        (sender.1, time.reads.load(Ordering::Relaxed))
    }

    /// Note times every `every_ms` from 0 to `until_ms`.
    fn every(every_ms: u64, until_ms: u64) -> Vec<u64> {
        (0..=until_ms / every_ms).map(|k| k * every_ms).collect()
    }

    /// The playback clock reads zero until the first audio buffer after
    /// the start, then moves a buffer at a time (here 2048 frames,
    /// 42.7ms). The notes are not a buffer behind, taking the first
    /// buffer for their zero, nor go out only as it moves.
    #[test]
    fn notes_keep_time_between_audio_buffers() {
        let times = [0, 10, 20, 30, 60, 100];
        let feed = callbacks(1.0, 2048, 0.0, false, 0.0, 1.0);
        let (sent, _) = play_virtual(&feed, 2048, &times, 20, passes(&[], 0, 0));
        for (sent, ms) in sent.iter().zip(times) {
            let off = sent - (20 + ms) as f64 / 1000.0;
            assert!(off.abs() < 0.0005, "{ms}ms note {:.2}ms off", off * 1000.0);
        }
    }

    /// A sound card as most are -- callbacks within a millisecond of
    /// their time, its clock the computer's -- with periods of 256, 1024
    /// or 2048 frames: the first note goes out with the beat clock's
    /// Start, every note within 3ms of the beat clock's grid, evenly
    /// spaced to 2ms.
    #[test]
    fn notes_keep_to_a_sound_cards_clock_on_the_beat_clocks_grid() {
        let times = every(100, 3_000);
        for frames in [256, 1024, 2048] {
            let feed = callbacks(1.0, frames, 0.001, false, 0.0, 3.5);
            let (sent, _) = play_virtual(&feed, frames, &times, 20, passes(&[], 0, 0));
            let first = sent[0] - 0.02;
            assert!(
                first.abs() < 0.0005,
                "{frames}: first note {:.2}ms off",
                first * 1000.0
            );
            for (sent, ms) in sent.iter().zip(&times) {
                let off = sent - (20 + ms) as f64 / 1000.0;
                assert!(
                    off.abs() < 0.003,
                    "{frames}: {ms}ms note {:.2}ms off the grid",
                    off * 1000.0
                );
            }
            for (k, pair) in sent.windows(2).enumerate() {
                let jitter = pair[1] - pair[0] - 0.1;
                assert!(
                    jitter.abs() < 0.002,
                    "{frames}: note {} {:.2}ms off its spacing",
                    k + 1,
                    jitter * 1000.0
                );
            }
        }
    }

    /// Where the audio is at `wall`, and when it reaches `audio`, with
    /// each of `held` (when the audio held, how long) taken into account.
    fn held_audio(held: &[(f64, f64)]) -> (impl Fn(f64) -> f64 + '_, impl Fn(f64) -> f64 + '_) {
        let truth = move |audio: f64| {
            let mut wall = audio;
            for &(at, lost) in held {
                if wall >= at {
                    wall += lost;
                }
            }
            wall
        };
        let position = move |wall: f64| {
            let mut lost_before = 0.0;
            for &(at, lost) in held {
                let at = at + lost_before;
                if wall >= at + lost {
                    lost_before += lost;
                } else if wall >= at {
                    return at - lost_before;
                }
            }
            wall - lost_before
        };
        (truth, position)
    }

    /// Xruns and a stall on a sound card (a period lost at 0.6s, three
    /// at 2s, 300ms at 3.5s): the notes never run more than a period
    /// ahead of the audio, are never more than 3ms late after it (and
    /// the device's jitter), and are back within 3ms of it within a
    /// second.
    #[test]
    fn notes_go_on_with_the_audio_after_an_xrun_or_a_stall() {
        for frames in [256, 1024, 2048] {
            let period = frames as f64 / 48_000.0;
            let stalls = [(0.6, period), (2.0, 3.0 * period), (3.5, 0.3)];
            let mut feed = callbacks(1.0, frames, 0.001, false, 0.0, 6.0);
            for (at, lost) in stalls {
                feed = stalled(feed, at, lost);
            }
            // The audio holds at the first period's end at or after
            // each: what was written before it plays out.
            let held: Vec<(f64, f64)> = stalls
                .iter()
                .map(|&(at, lost)| ((at / period).ceil() * period, lost))
                .collect();
            let (truth, position) = held_audio(&held);
            let times = every(50, 5_000);
            let (sent, _) = play_virtual(&feed, frames, &times, 20, passes(&[], 0, 0));
            assert_eq!(sent.len(), times.len());
            for (sent, ms) in sent.iter().zip(&times) {
                let audio = (20 + ms) as f64 / 1000.0;
                let ahead = audio - position(*sent);
                assert!(
                    ahead <= period + 0.002,
                    "{frames}: {ms}ms note {:.1}ms ahead of the audio",
                    ahead * 1000.0
                );
                // 3ms, and the device's own jitter (1ms), which the
                // audio's place here (on the nominal period grid) does
                // not know.
                let late = sent - truth(audio);
                assert!(
                    late < 0.004,
                    "{frames}: {ms}ms note {:.1}ms late",
                    late * 1000.0
                );
                let due = truth(audio);
                if held
                    .iter()
                    .all(|&(at, lost)| due < at || due > at + lost + 1.0)
                {
                    assert!(
                        late.abs() < 0.003,
                        "{frames}: {ms}ms note {:.1}ms off the audio",
                        late * 1000.0
                    );
                }
            }
        }
    }

    /// An xrun (a period) or a stall (300ms) in the first half second,
    /// before the notes' clock has measured the audio against the beat
    /// clock's zero: the notes run on the wall clock, held once the
    /// counter has not moved for three periods (30ms at least), so they
    /// run no further than that ahead of the audio; then they are not
    /// late after it, nor bunched as the measurement comes, and are
    /// back on the audio within a second.
    #[test]
    fn an_early_xrun_or_stall_is_bounded_and_leaves_no_lasting_offset() {
        for frames in [256, 1024] {
            let period = frames as f64 / 48_000.0;
            let margin = (3.0 * period).max(0.03);
            for lost in [period, 0.3] {
                let feed = stalled(callbacks(1.0, frames, 0.001, false, 0.0, 4.0), 0.3, lost);
                let held = [((0.3 / period).ceil() * period, lost)];
                let (truth, position) = held_audio(&held);
                let times = every(50, 3_000);
                let (sent, _) = play_virtual(&feed, frames, &times, 20, passes(&[], 0, 0));
                for (sent, ms) in sent.iter().zip(&times) {
                    let audio = (20 + ms) as f64 / 1000.0;
                    let ahead = audio - position(*sent);
                    assert!(
                        ahead <= margin + 0.002,
                        "{frames}, {lost}s: {ms}ms note {:.1}ms ahead of the audio",
                        ahead * 1000.0
                    );
                    let due = truth(audio);
                    let late = sent - due;
                    // 3ms, and the device's jitter (see the test above).
                    assert!(
                        late < 0.004,
                        "{frames}, {lost}s: {ms}ms note {:.1}ms late",
                        late * 1000.0
                    );
                    if due > held[0].0 + lost + 1.0 {
                        assert!(
                            late.abs() < 0.003,
                            "{frames}, {lost}s: {ms}ms note {:.1}ms off",
                            late * 1000.0
                        );
                    }
                }
                // Not bunched once the audio goes on, through the
                // measurement (0.5s in) and on.
                for (pair, ms) in sent.windows(2).zip(&times) {
                    if truth((20 + ms) as f64 / 1000.0) < held[0].0 + lost {
                        continue;
                    }
                    assert!(
                        pair[1] - pair[0] > 0.05 - 0.003,
                        "{frames}, {lost}s: notes at {ms}ms bunched: {:.1}ms",
                        (pair[1] - pair[0]) * 1000.0
                    );
                }
            }
        }
    }

    /// A hiccup (80ms) and a stall (300ms) on a device 10% slow or fast
    /// (512 frames, callbacks within a millisecond): the notes hold
    /// through each, go on with the audio once it does (within 3ms, and
    /// its jitter, past 100ms after it), the stall not baked into the
    /// rest of the song, and no notes bunched.
    #[test]
    fn notes_hold_through_a_stall_on_a_device_off_the_computers_clock() {
        let times = every(100, 4_000);
        for rate in [0.9, 1.1] {
            let period = 512.0 / 48_000.0 / rate;
            let stalls = [(1.5, 0.08), (2.5, 0.3)];
            let mut feed = callbacks(rate, 512, 0.001, false, 0.0, 6.0);
            for (at, lost) in stalls {
                feed = stalled(feed, at, lost);
            }
            let held: Vec<(f64, f64)> = stalls
                .iter()
                .map(|&(at, lost)| ((at / period).ceil() * period, lost))
                .collect();
            let (truth, position) = held_audio(&held);
            let (sent, _) = play_virtual(&feed, 512, &times, 20, passes(&[], 0, 0));
            for (sent, ms) in sent.iter().zip(&times) {
                // In the device's time, the wall's at its rate.
                let audio = (20 + ms) as f64 / 1000.0 / rate;
                // Until the offset is measured (0.5s in) the notes go on
                // the wall clock, which this device is not on.
                if audio < 0.7 {
                    continue;
                }
                let ahead = audio - position(*sent);
                assert!(
                    ahead <= period + 0.002,
                    "{rate}x: {ms}ms note {:.1}ms ahead of the audio",
                    ahead * 1000.0
                );
                let due = truth(audio);
                if held
                    .iter()
                    .any(|&(at, lost)| due > at && due < at + lost + 0.1)
                {
                    continue;
                }
                let off = sent - due;
                assert!(
                    off.abs() < 0.004,
                    "{rate}x: {ms}ms note {:.1}ms off the audio",
                    off * 1000.0
                );
            }
            // From the notes' clock having measured the audio (until then
            // they go on the wall clock, which this device is not on).
            let spacing = 0.1 / rate;
            for (k, pair) in sent.windows(2).enumerate().skip(7) {
                assert!(
                    pair[1] - pair[0] + 0.003 >= spacing,
                    "{rate}x: notes {k} and {} bunched: {:.1}ms",
                    k + 1,
                    (pair[1] - pair[0]) * 1000.0
                );
            }
        }
    }

    /// A device far from any sound card (one resampling, 9-10% slow or
    /// fast, its callbacks up to a period early or late and one in ten
    /// held back to come with the next): the notes follow it at its own
    /// rate within a few periods, and do not drift off it. They read
    /// the counter about each move due and as they wake, not every
    /// millisecond. And the first note goes out with the beat clock's
    /// Start, here 200ms in: the notes go on the wall clock until they
    /// have measured the audio (they were held a couple of periods
    /// behind Start, bounded by the moves they had seen).
    #[test]
    fn notes_follow_a_jittery_audio_clock_running_slow_or_fast() {
        let times = every(250, 4_500);
        for rate in [0.9, 0.91, 1.1] {
            let period = 512.0 / 48_000.0 / rate;
            let feed = callbacks(rate, 512, period, true, 0.0, 6.0);
            let (sent, reads) = play_virtual(&feed, 512, &times, 20, passes(&[], 0, 0));
            let errors: Vec<(u64, f64)> = sent
                .iter()
                .zip(&times)
                .map(|(sent, ms)| (*ms, sent - (20 + ms) as f64 / 1000.0 / rate))
                .collect();
            for &(ms, error) in errors.iter().filter(|(ms, _)| *ms >= 1_500) {
                assert!(
                    error.abs() < 4.0 * period,
                    "{rate}x: {ms}ms note {:.1}ms off the audio",
                    error * 1000.0
                );
            }
            let mean = |from: u64, to: u64| {
                let span: Vec<f64> = errors
                    .iter()
                    .filter(|(ms, _)| (from..to).contains(ms))
                    .map(|(_, error)| *error)
                    .collect();
                span.iter().sum::<f64>() / span.len() as f64
            };
            let drift = mean(3_500, 4_600) - mean(1_500, 2_600);
            assert!(
                drift.abs() < 0.003,
                "{rate}x: drifted {:.1}ms",
                drift * 1000.0
            );
            // 4.5s of notes: reading every millisecond would be 4,500.
            assert!(reads < 1_500, "{rate}x: {reads} reads");

            let (sent, _) = play_virtual(&feed, 512, &[0, 100], 200, passes(&[], 0, 0));
            let first = sent[0] - 0.2;
            assert!(
                first.abs() < 0.0005,
                "{rate}x: first note {:.1}ms off Start",
                first * 1000.0
            );
        }
    }

    /// The first callback coming 30ms late does not put the notes off
    /// for the song.
    #[test]
    fn a_late_first_callback_does_not_put_the_notes_off() {
        let times = every(250, 3_000);
        let feed = callbacks(1.0, 512, 0.001, false, 0.03, 4.0);
        let (sent, _) = play_virtual(&feed, 512, &times, 20, passes(&[], 0, 0));
        for (sent, ms) in sent.iter().zip(&times) {
            let off = sent - (20 + ms) as f64 / 1000.0;
            assert!(off.abs() < 0.003, "{ms}ms note {:.1}ms off", off * 1000.0);
        }
    }

    /// A jump after a quiet stretch with no notes (here 3.1s after the
    /// last) plays on time: the notes' clock kept to the audio (a
    /// device 10% slow) through it.
    #[test]
    fn a_jump_after_a_quiet_stretch_plays_on_time() {
        let rate = 0.9;
        let log = Arc::new(crate::playsync::SectionJumpLog::default());
        log.push(crate::section_loop::SectionJump {
            at: Duration::from_millis(3_200),
            from: Duration::from_millis(3_200),
            to: Duration::ZERO,
            overlap: Duration::ZERO,
            pass: true,
        });
        log.close();
        let feed = callbacks(rate, 512, 0.001, false, 0.0, 5.0);
        let (sent, _) = play_virtual(&feed, 512, &[0, 100], 20, log);
        assert_eq!(sent.len(), 4);
        // The jump's notes, when the audio reaches 3.2s and 3.3s.
        for (sent, ms) in sent[2..].iter().zip([3_200, 3_300]) {
            let off = sent - (20 + ms) as f64 / 1000.0 / rate;
            assert!(off.abs() < 0.003, "{ms}ms note {:.1}ms off", off * 1000.0);
        }
    }

    /// A crossfading loop's passes, in virtual time: each pass's notes
    /// keep to the audio from its jump, the pass it leaves playing on for
    /// the overlap in time with it, older pass first at the same moment.
    /// Pins the times [`crossfading_loop_overlaps_its_passes`] checks on
    /// real threads only to a loaded machine's bounds.
    #[test]
    fn overlapping_passes_play_their_notes_on_time() {
        // A 1 s song with a 0.4 s overlap: a pass every 0.6 s, notes 60,
        // 61, 62 at 0, 0.3 and 0.9 s into each; the last pass plays out.
        let feed = callbacks(1.0, 512, 0.001, false, 0.0, 3.0);
        let (sent, _) = play_virtual_messages(
            &feed,
            512,
            &[0, 300, 900],
            20,
            passes(&[600, 1_200], 600, 400),
        );
        let notes: Vec<(f64, u8)> = sent
            .iter()
            .filter(|(_, bytes)| bytes[0] & 0xF0 == 0x90)
            .map(|(at, bytes)| (*at, bytes[1]))
            .collect();
        let expected = [
            (0, 60),
            (300, 61),
            (600, 60),
            (900, 62),
            (900, 61),
            (1_200, 60),
            (1_500, 62),
            (1_500, 61),
            (2_100, 62),
        ];
        assert_eq!(notes.len(), expected.len(), "{notes:?}");
        for ((at, note), (ms, want)) in notes.iter().zip(expected) {
            let off = at - (20 + ms) as f64 / 1000.0;
            assert!(
                off.abs() < 0.003,
                "{ms}ms note {:.1}ms off: {notes:?}",
                off * 1000.0
            );
            assert_eq!(*note, want, "{notes:?}");
        }
    }

    /// A long playback delay on a device 10% fast: the delay is waited
    /// on the audio's clock, so the first notes come when the audio
    /// reaches it, on time, not in a bunch.
    #[test]
    fn a_long_delay_on_a_fast_device_starts_on_the_audio() {
        let rate = 1.1;
        let times = every(100, 1_000);
        let feed = callbacks(rate, 1024, 0.001, false, 0.0, 4.0);
        let (sent, _) = play_virtual(&feed, 1024, &times, 2_000, passes(&[], 0, 0));
        for (sent, ms) in sent.iter().zip(&times) {
            let off = sent - (2_000 + ms) as f64 / 1000.0 / rate;
            assert!(off.abs() < 0.003, "{ms}ms note {:.1}ms off", off * 1000.0);
        }
    }

    /// On real threads, a real audio clock fed a buffer at a time: the
    /// notes go out in order, near their time and not bunched. (A smoke
    /// test, bounds generous for a loaded machine; the timing itself is
    /// tested in virtual time above.)
    #[test]
    fn notes_play_on_a_real_audio_clock() {
        let times = every(100, 1_500);
        let feed = callbacks(1.0, 1024, 0.001, false, 0.0, 2.5);
        let (started, sent) = play_on_audio(feed, 1024, &times, None, passes(&[], 0, 0));
        assert_eq!(sent.len(), times.len());
        // Bounds a loaded machine's late thread (this one's, or the
        // feeder's holding the audio back) stays inside: up to 60ms off,
        // so notes 100ms apart stay over 40ms apart, not bunched.
        let slack = Duration::from_millis(60);
        for (sent, ms) in sent.iter().zip(&times) {
            let off = off(*sent, started + Duration::from_millis(20 + ms));
            assert!(off < slack, "{ms}ms note {off:?} off");
        }
        for pair in sent.windows(2) {
            assert!(
                pair[1] - pair[0] > Duration::from_millis(100) - slack,
                "notes bunched"
            );
        }
    }

    /// With no audio the playback clock is the wall clock, which the
    /// notes keep to without reading it between.
    #[test]
    fn notes_on_a_wall_clock_do_not_read_it_between() {
        let midi = make_events(&[0, 50, 100]);
        let clock = PlaybackClock::wall();
        clock.start();
        NOTE_CLOCK_READS.with(|reads| reads.set(0));
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let mut sender = MockSender::new();
        run_playback(
            &mut sender,
            PlaybackContext {
                precomputed: &midi,
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel_handle: &CancelHandle::new(),
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                finished: Arc::new(AtomicBool::new(false)),
                exclude_channels: &HashSet::new(),
                clock: &clock,
                loop_break: Arc::new(AtomicBool::new(false)),
                active_section: Arc::new(parking_lot::RwLock::new(None)),
                section_jumps: passes(&[], 0, 0),
                transport: None,
            },
        );
        assert_eq!(sender.sent.lock().len(), 3);
        assert_eq!(NOTE_CLOCK_READS.with(|reads| reads.get()), 0);
    }

    /// The beat clock's Start and the first note, each on its own thread
    /// as on a rig (the beat clock engine announcing its Start to the
    /// notes), with a 200ms playback delay, on a sound card and on a
    /// jittery device 9% slow: the note goes out just after Start. Before
    /// it had measured the audio, the notes' clock took the jittery
    /// device's moves for the audio held up, and bounded itself by them,
    /// and the note went out a couple of periods after Start.
    #[test]
    fn the_first_note_follows_the_beat_clocks_start() {
        type Log = Arc<parking_lot::Mutex<Vec<(Vec<u8>, std::time::Instant)>>>;
        struct Stamped(Log);
        impl MidiSender for Stamped {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                self.0
                    .lock()
                    .push((bytes.to_vec(), std::time::Instant::now()));
                Ok(())
            }
        }
        let _one_at_a_time = AUDIO_TIMING.lock();
        let delay = Duration::from_millis(200);
        for rate in [1.0, 0.91] {
            let period = 512.0 / 48_000.0 / rate;
            let jitter = if rate == 1.0 { 0.001 } else { period };
            let feed = callbacks(rate, 512, jitter, rate != 1.0, 0.0, 1.5);
            let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let clock = PlaybackClock::from_sample_counter(counter.clone(), 48_000);
            let done = Arc::new(AtomicBool::new(false));
            let started = Arc::new(std::sync::Barrier::new(2));
            let feeder = thread::spawn({
                let (counter, done, started) = (counter.clone(), done.clone(), started.clone());
                move || {
                    started.wait();
                    let start = std::time::Instant::now();
                    for at in feed {
                        if done.load(Ordering::Relaxed) {
                            break;
                        }
                        let at = Duration::from_secs_f64(at);
                        spin_sleep::sleep(at.saturating_sub(start.elapsed()));
                        counter.fetch_add(512, Ordering::Relaxed);
                    }
                }
            });
            // The beat clock, as the device's engine plays it.
            let transport = Arc::new(AtomicBool::new(false));
            let cancel = CancelHandle::new();
            let clocks = Arc::new(parking_lot::Mutex::new(Vec::new()));
            let (tx, rx) = mpsc::channel();
            tx.send(BeatClockCommand::Play(BeatClockPlay {
                ticks: Arc::new((0..60).map(|k| Duration::from_millis(k * 20)).collect()),
                start_time: Duration::ZERO,
                playback_delay: delay,
                cancel: cancel.clone(),
                clock: clock.clone(),
                sections: None,
                transport: Some(transport.clone()),
            }))
            .unwrap();
            let engine = thread::spawn({
                let clocks = clocks.clone();
                move || {
                    let tempo = Mutex::new(None);
                    beat_clock_engine_loop(&mut Stamped(clocks), false, &tempo, &rx);
                }
            });
            let notes = Arc::new(parking_lot::Mutex::new(Vec::new()));
            let midi = make_events(&[0, 100]);
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            clock.start();
            started.wait();
            run_playback(
                &mut Stamped(notes.clone()),
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: delay,
                    cancel_handle: &cancel,
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: Arc::new(AtomicBool::new(false)),
                    exclude_channels: &HashSet::new(),
                    clock: &clock,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_jumps: passes(&[], 0, 0),
                    transport: Some(transport),
                },
            );
            cancel.cancel();
            drop(tx);
            engine.join().unwrap();
            done.store(true, Ordering::Relaxed);
            feeder.join().unwrap();
            let start = clocks
                .lock()
                .iter()
                .find(|(bytes, _)| bytes.as_slice() == [0xFA])
                .map(|(_, at)| *at)
                .expect("no Start");
            let note = notes.lock().first().map(|(_, at)| *at).expect("no note");
            // Generous for a loaded machine, whose threads can wake tens of
            // milliseconds late: the note's time itself (it was 26ms late)
            // is tested in virtual time, in
            // notes_follow_a_jittery_audio_clock_running_slow_or_fast. The
            // order cannot flake: the note waits for Start to have gone.
            assert!(note >= start, "{rate}x: the note went before Start");
            assert!(
                note - start < Duration::from_millis(100),
                "{rate}x: the note went {:?} after Start",
                note - start
            );
        }
    }

    /// A note due with the beat clock's Start goes out after it: the
    /// notes wait for the beat clock to send it (here 8ms late).
    #[test]
    fn notes_wait_for_the_beat_clocks_start() {
        let transport = Arc::new(AtomicBool::new(false));
        let set_at = Arc::new(parking_lot::Mutex::new(None));
        let setter = thread::spawn({
            let transport = transport.clone();
            let set_at = set_at.clone();
            move || {
                // The notes are due 20ms in: Start comes 8ms after.
                std::thread::sleep(Duration::from_millis(28));
                *set_at.lock() = Some(std::time::Instant::now());
                transport.store(true, Ordering::Release);
            }
        });
        let callbacks = callbacks(1.0, 1024, 0.0, false, 0.0, 1.0);
        let (_, sent) = play_on_audio(
            callbacks,
            1024,
            &[0, 0, 5],
            Some(transport),
            passes(&[], 0, 0),
        );
        assert_eq!(sent.len(), 3);
        setter.join().unwrap();
        let set_at = set_at.lock().expect("never set");
        assert!(sent.iter().all(|sent| *sent >= set_at));
    }

    /// The beat clock's sender says so once it has sent Start or
    /// Continue, not before.
    #[test]
    fn the_beat_clock_announces_its_start() {
        for transport_byte in [0xFA, 0xFB] {
            let transport = AtomicBool::new(false);
            let mut null = Null;
            let mut sender = Announcing {
                inner: &mut null,
                transport: Some(&transport),
            };
            sender.send(&[0xF2, 3, 0]).unwrap();
            sender.send(&[0xFC]).unwrap();
            assert!(!transport.load(Ordering::Acquire));
            sender.send(&[transport_byte]).unwrap();
            assert!(transport.load(Ordering::Acquire));
        }
    }
}

mod run_beat_clock_tests {
    use super::*;
    use crate::playsync::CancelHandle;
    use midly::live::SystemRealtime;

    /// Records each message with when on `clock` it went out.
    struct StampedSender {
        clock: PlaybackClock,
        sent: parking_lot::Mutex<Vec<(Vec<u8>, Duration)>>,
    }

    impl StampedSender {
        fn new(clock: &PlaybackClock) -> Self {
            StampedSender {
                clock: clock.clone(),
                sent: parking_lot::Mutex::new(Vec::new()),
            }
        }
    }

    impl MidiSender for StampedSender {
        fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
            self.sent
                .lock()
                .push((bytes.to_vec(), self.clock.elapsed()));
            Ok(())
        }
    }

    /// Records each clock as the song time it was due at, and breaks
    /// the loop once it has sent `break_after` of them.
    struct LoopingSender {
        clock: PlaybackClock,
        clocks: Vec<Duration>,
        break_after: usize,
        loop_break: Arc<AtomicBool>,
    }

    impl MidiSender for LoopingSender {
        fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
            if bytes == clock_bytes().as_slice() {
                // A clock that ignored the break would go round for ever.
                assert!(
                    self.clocks.len() <= self.break_after,
                    "the loop break was not heard"
                );
                self.clocks.push(self.clock.elapsed());
                if self.clocks.len() == self.break_after {
                    self.loop_break.store(true, Ordering::Relaxed);
                }
            }
            Ok(())
        }
    }

    fn every_10ms(until_ms: u64) -> Vec<Duration> {
        (0..until_ms / 10)
            .map(|i| Duration::from_millis(i * 10))
            .collect()
    }

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    /// Start and the first clock are a millisecond apart, as the MIDI spec
    /// asks, though the first is due at once.
    #[test]
    fn the_first_clock_follows_start_by_a_millisecond() {
        struct Timed {
            started: Option<std::time::Instant>,
            first_clock: Option<Duration>,
        }
        impl MidiSender for Timed {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                if bytes == realtime_bytes(SystemRealtime::Start).as_slice() {
                    self.started = Some(std::time::Instant::now());
                } else if bytes == clock_bytes().as_slice() && self.first_clock.is_none() {
                    self.first_clock = self.started.map(|at| at.elapsed());
                }
                Ok(())
            }
        }
        let mut sender = Timed {
            started: None,
            first_clock: None,
        };
        let clock = PlaybackClock::wall();
        clock.start();
        let (_tx, rx) = mpsc::channel();
        run_beat_clock(
            &mut sender,
            &[Duration::ZERO, ms(10)],
            Duration::ZERO,
            None,
            None,
            &CancelHandle::new(),
            &clock,
            &rx,
        );
        assert!(
            sender.first_clock.unwrap() >= START_TO_CLOCK,
            "{:?}",
            sender.first_clock
        );
    }

    /// Plays `ticks` following the pass jumps of a looping song given in
    /// a closed log; the clocks that went out, on the playback clock.
    fn play_passes(ticks: &[Duration], jumps: &[(u64, u64)], break_after: usize) -> Vec<Duration> {
        play_passes_on(ticks, jumps, break_after, true)
    }

    /// As [`play_passes`], in virtual time (`virtual_time`: a clock that
    /// moves only as the schedule waits, so every pulse goes out exactly
    /// when due, however loaded the machine) or on the wall clock.
    fn play_passes_on(
        ticks: &[Duration],
        jumps: &[(u64, u64)],
        break_after: usize,
        virtual_time: bool,
    ) -> Vec<Duration> {
        let mut _pauses = None;
        let clock = if virtual_time {
            // Nanoseconds.
            let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let advance = counter.clone();
            _pauses = Some(TestPauses::set(move |length| {
                advance.fetch_add(length.as_nanos() as u64, Ordering::Relaxed);
            }));
            PlaybackClock::from_sample_counter(counter, 1_000_000_000)
        } else {
            PlaybackClock::wall()
        };
        clock.start();
        let log = Arc::new(crate::playsync::SectionJumpLog::default());
        for &(at, overlap) in jumps {
            log.push(crate::section_loop::SectionJump {
                at: ms(at),
                from: ms(400),
                to: Duration::ZERO,
                overlap: ms(overlap),
                pass: false,
            });
        }
        log.close();
        let loop_break = Arc::new(AtomicBool::new(false));
        let sections = BeatClockSections {
            active_section: Default::default(),
            section_loop_break: Default::default(),
            jumps: log,
            loop_break: loop_break.clone(),
            loops: true,
            seams: config::BeatClockSeams::RunThrough,
        };
        let mut sender = LoopingSender {
            clock: clock.clone(),
            clocks: Vec::new(),
            break_after,
            loop_break,
        };
        let (_tx, rx) = mpsc::channel();
        run_beat_clock(
            &mut sender,
            ticks,
            Duration::ZERO,
            Some(&sections),
            None,
            &CancelHandle::new(),
            &clock,
            &rx,
        );
        sender.clocks
    }

    /// A looping song's passes are jumps in the log, which the clock
    /// follows as it does a section's: back to the top at each, its
    /// pulses switching to the new pass there, the overlap no matter.
    /// (It used to keep a pass grid of its own.)
    #[test]
    fn the_clock_follows_a_looping_songs_passes_from_the_log() {
        // 500ms of clocks; passes start 100ms before the end (at 400ms
        // and 800ms on the clock), crossfading over the last 100ms.
        let clocks = play_passes(&every_10ms(500), &[(400, 100), (800, 100)], 1_000);
        // 0-390ms, twice, then the last pass to its end: the seam is a
        // pulse, not two at once, by the count.
        assert_eq!(clocks.len(), 40 + 40 + 50, "{clocks:?}");
        // Each pass's first pulse goes out at its jump, a whole pulse after
        // the last pass's last. In virtual time (see play_passes), so a
        // loaded machine cannot put the last pulse late, into the gap.
        let near = |a: Duration, b: Duration| a.abs_diff(b) < Duration::from_micros(1);
        assert!(near(clocks[40], ms(400)), "{clocks:?}");
        assert!(near(clocks[40] - clocks[39], ms(10)), "{clocks:?}");
        assert!(near(clocks[80], ms(800)), "{clocks:?}");
        assert!(near(clocks[80] - clocks[79], ms(10)), "{clocks:?}");
    }

    /// DIAGNOSTIC (no assertion; prints timing): a plain loop's pass jump
    /// lies past the last pulse (pulses stop short of the song's end), so
    /// the clock reaches it through the out-of-ticks wait, which polls the
    /// wall clock every 5ms instead of sleeping until the jump's `at`.
    #[test]
    #[ignore]
    fn diagnostic_first_pulse_of_a_plain_loops_pass() {
        // 120 BPM: a pulse every 20.833ms; a 500ms song, 24 pulses.
        let ticks: Vec<Duration> = (0..24)
            .map(|i| Duration::from_nanos(i * 500_000_000 / 24))
            .collect();
        let jumps: Vec<(u64, u64)> = (1..=10).map(|k| (k * 500, 0)).collect();
        let clocks = play_passes_on(&ticks, &jumps, 24 * 11, false);
        let late: Vec<Duration> = (1..=10)
            .map(|k| clocks[24 * k].saturating_sub(ms(500 * k as u64)))
            .collect();
        let mid: Vec<Duration> = (1..=10)
            .map(|k| clocks[24 * k + 12].saturating_sub(ms(500 * k as u64 + 250)))
            .collect();
        eprintln!("seam lateness {late:?}\nmid-pass lateness {mid:?}");
    }

    /// A broken loop ends the clock with a Stop, as the player ends the
    /// song with it.
    #[test]
    fn a_looping_clock_runs_across_seams_until_the_loop_is_broken() {
        let clocks = play_passes(&every_10ms(500), &[(400, 0), (800, 0)], 55);
        assert_eq!(clocks.len(), 55);
    }

    /// A loop of whole pulses that comes out a hair long puts no pulse
    /// against the next pass's first.
    #[test]
    fn a_pass_a_hair_long_keeps_every_seam_a_pulse() {
        // Ticks every 10ms to 400ms inclusive: the pass jumps at 400ms
        // from 400ms, a hair after its last pulse.
        let mut ticks = every_10ms(400);
        ticks.push(Duration::from_micros(399_990));
        let clocks = play_passes(&ticks, &[(400, 0)], 1_000);
        assert_eq!(clocks.len(), 40 + 41, "{clocks:?}");
    }

    fn section(start: u64, end: u64) -> (BeatClockSections, Arc<AtomicBool>) {
        let section_loop_break = Arc::new(AtomicBool::new(false));
        (
            BeatClockSections {
                active_section: Arc::new(parking_lot::RwLock::new(Some(
                    crate::player::SectionBounds {
                        name: "loop".to_string(),
                        start_time: ms(start),
                        end_time: ms(end),
                    },
                ))),
                section_loop_break: section_loop_break.clone(),
                jumps: Default::default(),
                loop_break: Default::default(),
                loops: false,
                seams: config::BeatClockSeams::RunThrough,
            },
            section_loop_break,
        )
    }

    /// Plays `ticks` from `start` with a section loop, breaking it once
    /// `break_after` clocks have gone out; how many went out in all.
    fn play_section(
        ticks: &[Duration],
        start: u64,
        sections: &BeatClockSections,
        loop_break: Arc<AtomicBool>,
        break_after: usize,
    ) -> usize {
        /// Breaks the section loop at the `break_after`th clock; a clock
        /// that never stopped looping fails rather than hangs.
        struct SectionSender {
            sent: usize,
            break_after: usize,
            section_loop_break: Arc<AtomicBool>,
        }
        impl MidiSender for SectionSender {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                if bytes == clock_bytes().as_slice() {
                    self.sent += 1;
                    assert!(self.sent < 1_000, "the section break was not heard");
                    if self.sent == self.break_after {
                        self.section_loop_break.store(true, Ordering::Relaxed);
                    }
                }
                Ok(())
            }
        }
        let clock = PlaybackClock::wall();
        clock.start();
        // The jumps a test gives stand in for the audio's; otherwise the
        // playback's own publisher commits them.
        let given = crate::playsync::JumpCursor::new(sections.jumps.clone())
            .peek()
            .is_some();
        let (log, done) = publish_jumps_for_test(
            &clock,
            ms(start),
            &sections.active_section,
            &sections.section_loop_break,
        );
        let mut sections = sections.clone();
        if !given {
            sections.jumps = log;
        }
        let sections = &sections;
        let mut sender = SectionSender {
            sent: 0,
            break_after,
            section_loop_break: loop_break,
        };
        let (_tx, rx) = mpsc::channel();
        run_beat_clock(
            &mut sender,
            ticks,
            ms(start),
            Some(sections),
            None,
            &CancelHandle::new(),
            &clock,
            &rx,
        );
        done.store(true, Ordering::Relaxed);
        sender.sent
    }

    /// A section loop takes the clock back to the section's start at its
    /// end, as it takes the notes and audio, until it is broken; then the
    /// clock plays on from the section's end (#604).
    #[test]
    fn a_section_loop_takes_the_clock_back_until_it_is_broken() {
        // 10ms clocks to 500ms; the section is 100-300ms. 30 clocks to
        // its end, then round from 100ms; broken at the 40th clock (at
        // 190ms, the second time round: well before the next jump is
        // committed, 30ms ahead of it, as the audio commits), then on
        // from there to the end: 30 more.
        let (sections, section_loop_break) = section(100, 300);
        let sent = play_section(&every_10ms(500), 0, &sections, section_loop_break, 40);
        assert_eq!(sent, 30 + 10 + 30);
    }

    /// A section armed after the clock has passed its end goes back at
    /// once, as the notes do.
    #[test]
    fn a_section_armed_past_its_end_goes_back_at_once() {
        // Started at 350ms, past the 100-300ms section: round from 100ms
        // straight away, 20 clocks a time round; broken at the 25th.
        let (sections, section_loop_break) = section(100, 300);
        let sent = play_section(&every_10ms(500), 350, &sections, section_loop_break, 25);
        // The 25th is at 140ms the second time round; then on to 490ms.
        // The jump falls when the publisher sees the section armed, a
        // poll after the start, so the 360ms clock (the start's MIDI
        // beat) may go out first. (The player refuses a section armed
        // past its end; this is the jump at once that a late arm still
        // gets.)
        assert!(sent == 25 + 35 || sent == 1 + 25 + 35, "{sent}");
    }

    /// A seek within the pass under way goes to its first tick at or after
    /// the position, however early in the song.
    #[test]
    fn pass_ticks_seek_goes_back_within_the_pass() {
        let ticks = every_10ms(500);
        let mut schedule = PassTicks::new(&ticks, ms(250));
        assert_eq!(schedule.next(), Some(ms(250)));
        schedule.seek(ms(95));
        assert_eq!(schedule.next(), Some(ms(100)));
    }

    /// The drift steering follows a playback clock running fast or slow
    /// against the wall, read through its buffer-sized steps, and holds
    /// still for one keeping time with it.
    #[test]
    fn the_schedule_is_steered_onto_the_playback_clocks_drift() {
        // Two minutes of readings every 20ms; the playback clock moves in
        // 5.33ms steps (256 frames at 48kHz), so a reading lags by up to
        // that much.
        let step = 256.0 / 48_000.0;
        for ppm in [-100.0, 0.0, 50.0, 100.0] {
            let mut discipline = Discipline::default();
            let mut previous = 0.0;
            for k in 0..6_000u64 {
                let wall = k as f64 * 0.02;
                let true_ahead = wall * ppm * 1e-6;
                let reading = ((wall + true_ahead) / step).floor() * step;
                discipline.observe(Duration::from_secs_f64(wall), reading - wall);
                // No tick moves more than the most a window allows.
                assert!((discipline.correction - previous).abs() <= Discipline::MAX_STEP + 1e-12);
                previous = discipline.correction;
            }
            let drift = 120.0 * ppm * 1e-6;
            assert!(
                (discipline.correction - drift).abs() < 0.002,
                "{ppm} ppm: corrected {:.4}s for a drift of {drift:.4}s",
                discipline.correction
            );
        }
    }

    /// With audio, the clock goes back where and when the audio has
    /// committed to, not by a section reading of its own: here the armed
    /// section is 100-300ms, but the audio's one jump goes back from 300
    /// to 150ms, and that is the one made.
    #[test]
    fn a_clock_with_audio_follows_the_jumps_the_audio_commits_to() {
        let (mut sections, section_loop_break) = section(100, 300);
        let jumps = Arc::new(crate::playsync::SectionJumpLog::default());
        jumps.push(crate::section_loop::SectionJump {
            at: ms(300),
            from: ms(300),
            to: ms(150),
            overlap: Duration::ZERO,
            pass: false,
        });
        sections.jumps = jumps.clone();
        // Broken once the audio's jump has been taken: the section's own
        // end would loop again at 500ms, but the audio made no such jump.
        let sent = play_section(&every_10ms(500), 0, &sections, section_loop_break, 31);
        // 30 to 300ms, back to 150ms, and on to the end: 35 more (a
        // jump of its own would have gone to 100ms: 40).
        assert_eq!(sent, 30 + 35);
    }

    /// A section with no pulse in it (shorter than a pulse, between two)
    /// does not spin the clock thread for ever, its jump finding the same
    /// tick again and again, never sending, never hearing a cancel.
    #[test]
    fn a_section_with_no_pulse_does_not_spin() {
        let (sections, section_loop_break) = section(103, 107);
        let cancel = CancelHandle::new();
        cancel.cancel();
        let handle = std::thread::spawn({
            let sections = sections.clone();
            let cancel = cancel.clone();
            move || {
                let clock = PlaybackClock::wall();
                clock.start();
                let mut sender = MockSender::new();
                let (_tx, rx) = mpsc::channel();
                // Started at 110ms, past the section's end.
                run_beat_clock(
                    &mut sender,
                    &every_10ms(500),
                    ms(110),
                    Some(&sections),
                    None,
                    &cancel,
                    &clock,
                    &rx,
                );
            }
        });
        // A cancelled schedule returns at once; give it ample time.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while !handle.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let returned = handle.is_finished();
        // Free a spinning thread either way, so nothing hangs.
        section_loop_break.store(true, Ordering::Relaxed);
        handle.join().unwrap();
        assert!(returned, "the clock spun on a section with no pulse in it");
    }

    /// The last section, ending at or past the last pulse (its end on
    /// the song's end), loops in the audio, and the clock waits for its
    /// jump rather than running out of ticks and sending Stop.
    #[test]
    fn a_section_ending_past_the_last_pulse_still_loops() {
        // Pulses 0-490ms; the section is 300-500ms. 50 to the end, back
        // to 300 (20 a time round); broken at the 60th (390ms the second
        // time round), then on to 490ms: 10 more.
        let (sections, section_loop_break) = section(300, 500);
        let sent = play_section(&every_10ms(500), 0, &sections, section_loop_break, 60);
        assert_eq!(sent, 70);
    }

    /// a section with no pulse in it at all, past the last one
    /// (a clock that stops before the song does), can never be followed,
    /// yet the clock out of ticks waits for its jump for as long as the
    /// section stays armed: no Stop at the clock's end, and the engine
    /// cannot go on to hold the tempo. Here the section is broken after
    /// 1.5s; the clock (ticks to 490ms) should have stopped long before.
    #[test]
    fn a_section_past_the_last_pulse_does_not_hold_the_clock() {
        let (sections, section_loop_break) = section(600, 800);
        let clock = PlaybackClock::wall();
        clock.start();
        let breaker = std::thread::spawn({
            let section_loop_break = section_loop_break.clone();
            move || {
                std::thread::sleep(Duration::from_millis(1_500));
                section_loop_break.store(true, Ordering::Relaxed);
            }
        });
        let mut sender = MockSender::new();
        let (_tx, rx) = mpsc::channel();
        run_beat_clock(
            &mut sender,
            &every_10ms(500),
            Duration::ZERO,
            Some(&sections),
            None,
            &CancelHandle::new(),
            &clock,
            &rx,
        );
        let returned_at = clock.elapsed();
        let still_armed = !section_loop_break.load(Ordering::Relaxed);
        breaker.join().unwrap();
        assert!(
            still_armed,
            "the clock sent Stop only once the loop was broken, at {returned_at:?}"
        );
    }

    /// A section ending on the song's end, past its last pulse (as one in
    /// a looping song may, which no pass interrupts: the section takes
    /// precedence), goes back at its end with no silence and no spin.
    /// The clock used to keep passes of its own, which had rolled into
    /// the next when the jump came.
    #[test]
    fn a_section_ending_on_a_looping_songs_end_goes_back_in_its_pass() {
        let (sections, section_loop_break) = section(300, 500);
        let sent = play_section(&every_10ms(500), 0, &sections, section_loop_break, 52);
        // 0-490ms, round from 300ms (the break at the 52nd, 310ms), and
        // on to the end.
        assert_eq!(sent, 50 + 2 + 18);
    }

    /// A start at the top of the song noticed late sends Start and the
    /// pulses already due at once: gear counts its position from Start,
    /// so every pulse of the schedule goes out.
    #[test]
    fn a_late_start_keeps_the_pulse_count() {
        let clock = PlaybackClock::wall();
        clock.start();
        std::thread::sleep(Duration::from_millis(200));
        let mut sender = MockSender::new();
        let (_tx, rx) = mpsc::channel();
        let ticks: Vec<Duration> = (0..30).map(|k| ms(k * 20)).collect();
        run_beat_clock(
            &mut sender,
            &ticks,
            Duration::ZERO,
            None,
            None,
            &CancelHandle::new(),
            &clock,
            &rx,
        );
        let sent = sender.sent.lock();
        assert_eq!(sent[0], start_bytes());
        let clocks = sent.iter().filter(|bytes| **bytes == clock_bytes()).count();
        assert_eq!(clocks, ticks.len(), "pulses dropped at a late start");
    }

    /// A start part way in noticed a MIDI beat or more late puts the gear
    /// at the next MIDI beat still to come with a Song Position Pointer
    /// and Continue, rather than sending the pulses already due in a
    /// burst: the position and the pulses after it add up to the
    /// schedule's.
    #[test]
    fn a_late_start_part_way_in_goes_on_at_the_next_midi_beat() {
        let clock = PlaybackClock::wall();
        clock.start();
        std::thread::sleep(Duration::from_millis(200));
        let mut sender = StampedSender::new(&clock);
        let (_tx, rx) = mpsc::channel();
        let ticks: Vec<Duration> = (0..30).map(|k| ms(k * 20)).collect();
        // From 120ms, pulse 6, the 2nd MIDI beat.
        run_beat_clock(
            &mut sender,
            &ticks,
            ms(120),
            None,
            None,
            &CancelHandle::new(),
            &clock,
            &rx,
        );
        let sent = sender.sent.lock();
        let messages: Vec<&[u8]> = sent.iter().map(|(bytes, _)| bytes.as_slice()).collect();
        // 200ms late, the next MIDI beat to come is the 4th (pulse 18, at
        // 360ms in the song, due 240ms in); anything later is a heavily
        // loaded machine.
        let [0xF2, lsb, msb] = *messages[0] else {
            panic!("no Song Position Pointer first: {messages:?}");
        };
        let position = usize::from(lsb) | usize::from(msb) << 7;
        assert!(position >= 3, "{position}");
        assert_eq!(messages[1], continue_bytes().as_slice());
        let clocks: Vec<Duration> = sent
            .iter()
            .filter(|(bytes, _)| *bytes == clock_bytes())
            .map(|(_, at)| *at)
            .collect();
        assert_eq!(
            position * PULSES_PER_MIDI_BEAT + clocks.len(),
            ticks.len(),
            "the gear's count is off the song's"
        );
        // On time from there: no burst.
        assert!(
            clocks[0] >= ticks[position * PULSES_PER_MIDI_BEAT] - ms(120),
            "{:?}",
            clocks[0]
        );
        assert_eq!(*messages.last().unwrap(), stop_bytes().as_slice());
    }

    /// Plays `ticks` from the top, following the jumps in a closed log;
    /// what went out, with when.
    fn play_jumps(
        ticks: &[Duration],
        jumps: &[crate::section_loop::SectionJump],
        loops: bool,
        seams: config::BeatClockSeams,
    ) -> Vec<(Vec<u8>, Duration)> {
        let clock = PlaybackClock::wall();
        clock.start();
        let log = Arc::new(crate::playsync::SectionJumpLog::default());
        for jump in jumps {
            log.push(jump.clone());
        }
        log.close();
        let sections = BeatClockSections {
            active_section: Default::default(),
            section_loop_break: Default::default(),
            jumps: log,
            loop_break: Default::default(),
            loops,
            seams,
        };
        let mut sender = StampedSender::new(&clock);
        let (_tx, rx) = mpsc::channel();
        run_beat_clock(
            &mut sender,
            ticks,
            Duration::ZERO,
            Some(&sections),
            None,
            &CancelHandle::new(),
            &clock,
            &rx,
        );
        sender.sent.into_inner()
    }

    fn jump(at: u64, from: u64, to: u64) -> crate::section_loop::SectionJump {
        crate::section_loop::SectionJump {
            at: ms(at),
            from: ms(from),
            to: ms(to),
            overlap: Duration::ZERO,
            pass: false,
        }
    }

    /// The messages that are not clocks, and how many clocks came
    /// before each.
    fn transport(sent: &[(Vec<u8>, Duration)]) -> Vec<(usize, Vec<u8>)> {
        let mut clocks = 0;
        let mut out = Vec::new();
        for (bytes, _) in sent {
            if *bytes == clock_bytes() {
                clocks += 1;
            } else {
                out.push((clocks, bytes.clone()));
            }
        }
        out
    }

    /// Played from part way in, the gear is put at the play-from point
    /// with a Song Position Pointer and Continue: at the first MIDI beat
    /// (16th) at or after it, as the pointer gives no finer, the clocks
    /// going on from there.
    #[test]
    fn a_start_part_way_in_sends_the_song_position() {
        let clock = PlaybackClock::wall();
        clock.start();
        let mut sender = StampedSender::new(&clock);
        let (_tx, rx) = mpsc::channel();
        let ticks = every_10ms(500);
        run_beat_clock(
            &mut sender,
            &ticks,
            ms(135),
            None,
            None,
            &CancelHandle::new(),
            &clock,
            &rx,
        );
        let sent = sender.sent.into_inner();
        // Pulse 14 is the first at or after 135ms; the MIDI beat after,
        // the 3rd (pulse 18, 180ms), is where the gear goes on.
        assert_eq!(
            transport(&sent),
            vec![
                (0, vec![0xF2, 3, 0]),
                (0, continue_bytes()),
                (32, stop_bytes())
            ]
        );
        // Due 45ms into the schedule (180ms in the song, from 135ms).
        let first = sent.iter().find(|(bytes, _)| *bytes == clock_bytes());
        assert!(first.unwrap().1 >= ms(45), "{first:?}");
    }

    /// Relocating, a section loop's jump stops the gear and puts it at
    /// the section's start (its first MIDI beat, at or after it), and
    /// Continues; a jump to the top of the song Starts it.
    #[test]
    fn a_relocating_jump_sends_the_song_position_it_goes_back_to() {
        use crate::config::BeatClockSeams::Relocate;
        let ticks = every_10ms(500);
        // Back from 300ms to 150ms at 300ms: on at 180ms, pulse 18.
        let sent = play_jumps(&ticks, &[jump(300, 300, 150)], false, Relocate);
        assert_eq!(
            transport(&sent),
            vec![
                (0, start_bytes()),
                (30, stop_bytes()),
                (30, vec![0xF2, 3, 0]),
                (30, continue_bytes()),
                (30 + 32, stop_bytes())
            ]
        );
        // 180ms in the song, 30ms after the jump.
        let clocks: Vec<Duration> = sent
            .iter()
            .filter(|(bytes, _)| *bytes == clock_bytes())
            .map(|(_, at)| *at)
            .collect();
        assert!(clocks[30] >= ms(330), "{:?}", clocks[30]);

        // A looping song's pass, back to the top.
        let mut pass = jump(400, 400, 0);
        pass.pass = true;
        let sent = play_jumps(&ticks, &[pass], true, Relocate);
        assert_eq!(
            transport(&sent),
            vec![
                (0, start_bytes()),
                (40, stop_bytes()),
                (40, start_bytes()),
                (40 + 50, stop_bytes())
            ]
        );
    }

    /// Relocating, a jump the clock hears of late (here 100ms after it
    /// fell, or more) puts the gear at the next MIDI beat still to come,
    /// not back at the jump's target with the pulses since in a burst:
    /// the position and the clocks after it add up to the song's. The
    /// jump is committed by the clock's own sender as it sends the
    /// 400ms pulse, so however late a loaded machine runs the clock, the
    /// jump is heard late, and its section (0-300ms) still has a MIDI
    /// beat to come.
    #[test]
    fn a_jump_heard_late_goes_on_at_the_next_midi_beat() {
        struct Committing {
            stamped: StampedSender,
            log: Arc<crate::playsync::SectionJumpLog>,
            clocks: usize,
        }
        impl MidiSender for Committing {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                if bytes == clock_bytes().as_slice() {
                    self.clocks += 1;
                    if self.clocks == 41 {
                        // Back from 300ms to the top at 300ms.
                        self.log.push(jump(300, 300, 0));
                        self.log.close();
                    }
                }
                self.stamped.send(bytes)
            }
        }
        let clock = PlaybackClock::wall();
        clock.start();
        let log = Arc::new(crate::playsync::SectionJumpLog::default());
        let sections = BeatClockSections {
            active_section: Default::default(),
            section_loop_break: Default::default(),
            jumps: log.clone(),
            loop_break: Default::default(),
            loops: false,
            seams: config::BeatClockSeams::Relocate,
        };
        let mut sender = Committing {
            stamped: StampedSender::new(&clock),
            log,
            clocks: 0,
        };
        let (_tx, rx) = mpsc::channel();
        let ticks = every_10ms(500);
        run_beat_clock(
            &mut sender,
            &ticks,
            Duration::ZERO,
            Some(&sections),
            None,
            &CancelHandle::new(),
            &clock,
            &rx,
        );
        let sent = sender.stamped.sent.into_inner();
        let pointer = sent
            .iter()
            .position(|(bytes, _)| bytes[0] == 0xF2)
            .expect("no Song Position Pointer");
        let position = usize::from(sent[pointer].0[1]) | usize::from(sent[pointer].0[2]) << 7;
        // Heard at 400ms or later, 100ms or more into the song again:
        // the next MIDI beat is the 2nd (120ms) or later.
        assert!(position >= 2, "{position}");
        let after: Vec<Duration> = sent[pointer..]
            .iter()
            .filter(|(bytes, _)| *bytes == clock_bytes())
            .map(|(_, at)| *at)
            .collect();
        assert_eq!(position * PULSES_PER_MIDI_BEAT + after.len(), ticks.len());
        // On the song's time from there: the first is due 300ms after
        // its song time.
        assert!(
            after[0] >= ms(300) + ticks[position * PULSES_PER_MIDI_BEAT],
            "{:?}",
            after[0]
        );
    }

    /// A section shorter than a pulse, with none in it, is not gone back
    /// to, but the song is put back all the same: each time round, the
    /// pulses after it fall due later by its length. The clock used to
    /// keep running ahead of the audio from there on.
    #[test]
    fn a_section_with_no_pulse_puts_the_clock_back() {
        let ticks = every_10ms(500);
        // 103-107ms, three times round: on from 103ms at 115ms, so 110ms
        // in the song falls at 122ms.
        let sent = play_jumps(
            &ticks,
            &[
                jump(107, 107, 103),
                jump(111, 107, 103),
                jump(115, 107, 103),
            ],
            false,
            config::BeatClockSeams::Relocate,
        );
        assert_eq!(
            transport(&sent),
            vec![(0, start_bytes()), (50, stop_bytes())]
        );
        let clocks: Vec<Duration> = sent
            .iter()
            .filter(|(bytes, _)| *bytes == clock_bytes())
            .map(|(_, at)| *at)
            .collect();
        for (k, at) in clocks.iter().enumerate().skip(11) {
            assert!(*at >= ms(k as u64 * 10 + 12), "pulse {k} at {at:?}");
        }

        // 101-104ms, once: its jump comes due within half a pulse of
        // the 100ms pulse, before the section, which goes out on time;
        // 110ms falls at 113ms.
        let sent = play_jumps(
            &ticks,
            &[jump(104, 104, 101)],
            false,
            config::BeatClockSeams::Relocate,
        );
        let clocks: Vec<Duration> = sent
            .iter()
            .filter(|(bytes, _)| *bytes == clock_bytes())
            .map(|(_, at)| *at)
            .collect();
        assert_eq!(clocks.len(), 50);
        for (k, at) in clocks.iter().enumerate().skip(11) {
            assert!(*at >= ms(k as u64 * 10 + 3), "pulse {k} at {at:?}");
        }
    }

    /// By default a loop's clock runs straight on through the seam, back
    /// to the first pulse in the stretch gone back to, with nothing else
    /// sent: gear that takes Continue for Start, or cuts its notes at
    /// Stop, plays on.
    #[test]
    fn a_jump_runs_through_by_default() {
        use crate::config::BeatClockSeams::RunThrough;
        let ticks = every_10ms(500);
        let sent = play_jumps(&ticks, &[jump(300, 300, 150)], false, RunThrough);
        // 30 to 300ms, then from 150ms to the end: 35.
        assert_eq!(
            transport(&sent),
            vec![(0, start_bytes()), (30 + 35, stop_bytes())]
        );
        let mut pass = jump(400, 400, 0);
        pass.pass = true;
        let sent = play_jumps(&ticks, &[pass], true, RunThrough);
        assert_eq!(
            transport(&sent),
            vec![(0, start_bytes()), (40 + 50, stop_bytes())]
        );
    }

    /// Relocating, a section with a pulse but no MIDI beat in it (here
    /// 108-118ms, its one pulse 110ms) is gone round as it runs through:
    /// relocating to the next MIDI beat, 120ms, past its end, the next
    /// jump fell due at once, and only Stop, the pointer and Continue
    /// went out each time round, never a clock.
    #[test]
    fn a_section_with_no_midi_beat_runs_through_when_relocating() {
        let ticks = every_10ms(500);
        let sent = play_jumps(
            &ticks,
            &[
                jump(118, 118, 108),
                jump(128, 118, 108),
                jump(138, 118, 108),
            ],
            false,
            config::BeatClockSeams::Relocate,
        );
        // 0-110ms, 110ms three more times round, then on from 120ms: 38.
        assert_eq!(
            transport(&sent),
            vec![(0, start_bytes()), (12 + 3 + 38, stop_bytes())]
        );
    }

    /// A start, or a section's start, a hair after the pulse on its
    /// downbeat (a beat grid's time against pulses rounded to the
    /// microsecond) goes on from that MIDI beat, not the next.
    #[test]
    fn a_start_a_hair_after_a_midi_beat_goes_on_from_it() {
        let ticks = every_10ms(500);
        let hair = Duration::from_nanos(1);
        assert_eq!(midi_beat_at(&ticks, ms(120) + hair), 12);
        assert_eq!(midi_beat_at(&ticks, ms(120)), 12);
        assert_eq!(midi_beat_at(&ticks, ms(121)), 12);
        assert_eq!(midi_beat_at(&ticks, ms(106)), 12);
        assert_eq!(midi_beat_at(&ticks, ms(116)), 12);
        assert_eq!(midi_beat_at(&ticks, ms(125)), 12);
        assert_eq!(midi_beat_at(&ticks, ms(126)), 18);
        assert_eq!(midi_beat_at(&ticks, Duration::ZERO), 0);

        let clock = PlaybackClock::wall();
        clock.start();
        let mut sender = StampedSender::new(&clock);
        let (_tx, rx) = mpsc::channel();
        run_beat_clock(
            &mut sender,
            &ticks,
            ms(120) + hair,
            None,
            None,
            &CancelHandle::new(),
            &clock,
            &rx,
        );
        let sent = sender.sent.into_inner();
        assert_eq!(
            transport(&sent),
            vec![
                (0, vec![0xF2, 2, 0]),
                (0, continue_bytes()),
                (38, stop_bytes())
            ]
        );

        // A section from 120ms and a hair, relocating: back to pulse 12.
        let mut back = jump(300, 300, 120);
        back.to += hair;
        let sent = play_jumps(&ticks, &[back], false, config::BeatClockSeams::Relocate);
        assert_eq!(
            transport(&sent),
            vec![
                (0, start_bytes()),
                (30, stop_bytes()),
                (30, vec![0xF2, 2, 0]),
                (30, continue_bytes()),
                (30 + 38, stop_bytes())
            ]
        );
    }

    /// A Song Position Pointer is three bytes: 0xF2 and the position in
    /// MIDI beats, low seven bits first; one past its 14 bits is clamped
    /// to the furthest it reaches.
    #[test]
    fn song_position_pointer_bytes() {
        assert_eq!(song_position_bytes(0), [0xF2, 0, 0]);
        assert_eq!(song_position_bytes(200), [0xF2, 200 & 0x7F, 1]);
        assert_eq!(song_position_bytes(MAX_SONG_POSITION), [0xF2, 0x7F, 0x7F]);
        assert_eq!(
            song_position_bytes(MAX_SONG_POSITION + 6),
            [0xF2, 0x7F, 0x7F]
        );
    }

    /// At a tempo whose pulse is a whole number of audio buffers (or
    /// near one), readings taken at the ticks fell at one place in the
    /// playback clock's steps, which wandered as the two drifted, and a
    /// clock keeping perfect time was steered milliseconds off. Readings
    /// are taken at random points in each wait, as `sleep_until` takes
    /// them, and the steering holds to the true drift.
    #[test]
    fn steering_holds_at_a_tempo_matching_the_buffer_rate() {
        for (bpm, frames) in [
            (117.0, 1024.0),
            (117.1875, 1024.0),
            (117.1875, 256.0),
            (107.67, 1024.0),
        ] {
            let step = frames / 48_000.0;
            let pulse = 60.0 / bpm / 24.0;
            for ppm in [0.0, -50.0, 50.0] {
                let mut discipline = Discipline::default();
                let mut worst: f64 = 0.0;
                let mut k = 0u64;
                loop {
                    // Somewhere in the wait before tick k.
                    let wall = (k as f64 + discipline.next_fraction()) * pulse;
                    if wall > 120.0 {
                        break;
                    }
                    let true_ahead = wall * ppm * 1e-6;
                    let reading = ((wall + true_ahead) / step).floor() * step;
                    discipline.observe(Duration::from_secs_f64(wall), reading - wall);
                    worst = worst.max((discipline.correction - true_ahead).abs());
                    k += 1;
                }
                assert!(
                    worst < 0.002,
                    "{bpm} BPM, {frames} frames, {ppm} ppm: steered {:.1}ms off",
                    worst * 1000.0
                );
            }
        }
    }

    /// A stop just past a seam hands back the song's tempo to hold, not
    /// the gap across the seam.
    #[test]
    fn a_stop_just_after_a_seam_holds_the_songs_tempo() {
        struct CancelAfter {
            n: usize,
            sent: usize,
            cancel: CancelHandle,
        }
        impl MidiSender for CancelAfter {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                if bytes == clock_bytes().as_slice() {
                    self.sent += 1;
                    if self.sent == self.n {
                        self.cancel.cancel();
                    }
                }
                Ok(())
            }
        }
        let cancel = CancelHandle::new();
        // Clocks every 10ms; a pass every 291ms: 30 clocks a pass, the
        // seam 1ms after the last.
        let mut sender = CancelAfter {
            n: 31,
            sent: 0,
            cancel: cancel.clone(),
        };
        let clock = PlaybackClock::wall();
        clock.start();
        let (_tx, rx) = mpsc::channel();
        let interval = run_beat_clock(
            &mut sender,
            &every_10ms(500),
            Duration::ZERO,
            None,
            None,
            &cancel,
            &clock,
            &rx,
        )
        .interval();
        assert_eq!(interval, Some(Duration::from_millis(10)));
    }

    #[test]
    fn sends_start_clocks_and_stop() {
        // 1 beat at default 120 BPM = 24 clock ticks
        let beat_clock = crate::midi::beat_clock::PrecomputedBeatClock::from_tempo_info(
            &[crate::midi::playback::TempoEntry {
                tick: 0,
                micros_per_beat: 500_000,
            }],
            480,
            480,
        );
        let ticks: Vec<Duration> = beat_clock.ticks_from(Duration::ZERO).to_vec();
        let cancel = CancelHandle::new();
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();

        let (_tx, rx) = mpsc::channel();
        run_beat_clock(
            &mut sender,
            &ticks,
            Duration::ZERO,
            None,
            None,
            &cancel,
            &clock,
            &rx,
        );

        let sent = sender.sent.lock();
        // START + 24 clock ticks + STOP
        assert_eq!(sent.len(), 26);
        assert_eq!(sent[0], start_bytes());
        for msg in &sent[1..25] {
            assert_eq!(*msg, clock_bytes());
        }
        assert_eq!(sent[25], stop_bytes());
    }

    #[test]
    fn sends_start_when_seeking_before_the_first_pulse() {
        let ticks = vec![Duration::from_millis(500), Duration::from_millis(600)];
        let cancel = CancelHandle::new();
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();

        let (_tx, rx) = mpsc::channel();
        run_beat_clock(
            &mut sender,
            &ticks,
            Duration::from_millis(100),
            None,
            None,
            &cancel,
            &clock,
            &rx,
        );

        let sent = sender.sent.lock();
        // Played from before the first pulse, the gear's position zero:
        // START + 2 ticks + STOP.
        assert_eq!(sent.len(), 4);
        assert_eq!(sent[0], start_bytes());
    }

    #[test]
    fn empty_ticks_sends_start_and_stop() {
        let ticks: Vec<Duration> = Vec::new();
        let cancel = CancelHandle::new();
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();

        let (_tx, rx) = mpsc::channel();
        run_beat_clock(
            &mut sender,
            &ticks,
            Duration::ZERO,
            None,
            None,
            &cancel,
            &clock,
            &rx,
        );

        let sent = sender.sent.lock();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0], start_bytes());
        assert_eq!(sent[1], stop_bytes());
    }

    #[test]
    fn cancellation_sends_stop() {
        let ticks = vec![Duration::from_secs(10), Duration::from_secs(20)];
        let cancel = CancelHandle::new();
        cancel.cancel();
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();

        let (_tx, rx) = mpsc::channel();
        run_beat_clock(
            &mut sender,
            &ticks,
            Duration::ZERO,
            None,
            None,
            &cancel,
            &clock,
            &rx,
        );

        let sent = sender.sent.lock();
        // START + STOP (cancelled before any ticks)
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0], start_bytes());
        assert_eq!(sent[1], stop_bytes());
    }

    #[test]
    fn returns_last_delivered_interval() {
        // The returned interval reflects the tempo at the end of playback:
        // the spacing between the final two delivered ticks.
        let ticks = vec![
            Duration::from_millis(1),
            Duration::from_millis(3),
            Duration::from_millis(6),
        ];
        let cancel = CancelHandle::new();
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();

        let (_tx, rx) = mpsc::channel();
        let interval = run_beat_clock(
            &mut sender,
            &ticks,
            Duration::ZERO,
            None,
            None,
            &cancel,
            &clock,
            &rx,
        )
        .interval();

        // Last two ticks: 6ms - 3ms = 3ms.
        assert_eq!(interval, Some(Duration::from_millis(3)));
    }

    #[test]
    fn returns_none_for_single_tick() {
        // A single tick can't establish an interval, so no tempo is reported.
        let ticks = vec![Duration::from_millis(1)];
        let cancel = CancelHandle::new();
        let mut sender = MockSender::new();
        let clock = PlaybackClock::wall();

        let (_tx, rx) = mpsc::channel();
        let interval = run_beat_clock(
            &mut sender,
            &ticks,
            Duration::ZERO,
            None,
            None,
            &cancel,
            &clock,
            &rx,
        )
        .interval();

        assert_eq!(interval, None);
    }
}

mod beat_clock_engine_tests {
    use super::*;
    use crate::playsync::CancelHandle;

    use std::sync::mpsc;
    use std::sync::Arc;

    /// A sender whose buffer can be inspected from another thread while the
    /// engine loop runs.
    #[derive(Clone)]
    struct SharedSender {
        sent: Arc<parking_lot::Mutex<Vec<Vec<u8>>>>,
    }

    impl SharedSender {
        fn new() -> Self {
            SharedSender {
                sent: Arc::new(parking_lot::Mutex::new(Vec::new())),
            }
        }
        fn len(&self) -> usize {
            self.sent.lock().len()
        }
    }

    impl MidiSender for SharedSender {
        fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
            self.sent.lock().push(bytes.to_vec());
            Ok(())
        }
    }

    /// Builds a `BeatClockPlay` whose clock has already been started, so the
    /// engine's `wait_for_start_or_cancel` returns immediately.
    fn ready_play(ticks: Vec<Duration>, cancel: CancelHandle) -> BeatClockPlay {
        let clock = PlaybackClock::wall();
        clock.start();
        BeatClockPlay {
            ticks: Arc::new(ticks),
            start_time: Duration::ZERO,
            playback_delay: Duration::ZERO,
            cancel,
            clock,
            sections: None,
            transport: None,
        }
    }

    /// A closed channel ends the engine while it awaits a start that never
    /// comes, as it does anywhere else.
    #[test]
    fn a_closed_channel_ends_the_engine_awaiting_a_start() {
        let (tx, rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let cancel = CancelHandle::new();
        let play = BeatClockPlay {
            ticks: Arc::new(vec![Duration::ZERO]),
            start_time: Duration::ZERO,
            playback_delay: Duration::ZERO,
            cancel: cancel.clone(),
            clock: PlaybackClock::wall(),
            sections: None,
            transport: None,
        };
        thread::spawn(move || {
            let mut sender = SharedSender::new();
            beat_clock_engine_loop(
                &mut sender,
                true,
                &Mutex::new(Some(Duration::from_millis(20))),
                &rx,
            );
            let _ = done_tx.send(());
        });
        tx.send(BeatClockCommand::Play(play)).unwrap();
        thread::sleep(Duration::from_millis(50));
        drop(tx);
        let ended = done_rx.recv_timeout(Duration::from_secs(1)).is_ok();
        cancel.cancel();
        assert!(ended, "the engine kept waiting on a closed channel");
    }

    /// A stop during a song's playback delay is heard at once, not once
    /// the delay is over: a reopen joins this thread while it holds its
    /// lock.
    #[test]
    fn a_stop_during_the_playback_delay_is_heard_at_once() {
        let (tx, rx) = mpsc::channel();
        let engine = thread::spawn(move || {
            let mut sender = SharedSender::new();
            beat_clock_engine_loop(
                &mut sender,
                true,
                &Mutex::new(Some(Duration::from_millis(20))),
                &rx,
            );
        });
        let mut play = ready_play(vec![Duration::ZERO], CancelHandle::new());
        play.playback_delay = Duration::from_secs(2);
        tx.send(BeatClockCommand::Play(play)).unwrap();
        thread::sleep(Duration::from_millis(50));
        let stopping = std::time::Instant::now();
        tx.send(BeatClockCommand::Shutdown).unwrap();
        engine.join().unwrap();
        assert!(
            stopping.elapsed() < Duration::from_millis(500),
            "the stop waited {:?}",
            stopping.elapsed()
        );
    }

    /// Pacing, decided from a table of elapsed times rather than a clock.
    ///
    /// Nothing here sleeps, spawns, or reads the time, so these assertions
    /// mean the same thing on an idle laptop and a saturated CI runner.
    mod pacing {
        use super::*;

        /// Ticks come one per interval and do not drift.
        #[test]
        fn one_tick_per_interval() {
            let mut hold = TempoHold::new(Duration::from_millis(10));
            let mut sender = SharedSender::new();

            assert!(!hold.tick_if_due_at(&mut sender, Duration::from_millis(9)));
            assert!(hold.tick_if_due_at(&mut sender, Duration::from_millis(10)));
            assert!(!hold.tick_if_due_at(&mut sender, Duration::from_millis(19)));
            assert!(hold.tick_if_due_at(&mut sender, Duration::from_millis(20)));

            assert_eq!(sender.sent.lock().len(), 2);
            assert!(sender.sent.lock().iter().all(|m| *m == clock_bytes()));
        }

        /// A tick delivered late still schedules the next one on the
        /// original grid, so small jitter does not accumulate into drift.
        #[test]
        fn a_slightly_late_tick_keeps_the_grid() {
            let mut hold = TempoHold::new(Duration::from_millis(10));
            let mut sender = SharedSender::new();

            // Due at 10ms, delivered at 15ms: within one interval late.
            assert!(hold.tick_if_due_at(&mut sender, Duration::from_millis(15)));
            // The next is still due at 20ms, not 25ms.
            assert_eq!(
                hold.until_next_at(Duration::from_millis(15)),
                Duration::from_millis(5)
            );
        }

        /// Falling more than an interval behind re-anchors instead of
        /// replaying the backlog. This is the case that matters: a 10s stall
        /// at a 10ms interval is a thousand ticks owed, and firing them back
        /// to back reads downstream as an enormous tempo spike.
        #[test]
        fn a_long_stall_re_anchors_instead_of_bursting() {
            let mut hold = TempoHold::new(Duration::from_millis(10));
            let mut sender = SharedSender::new();

            // Gone for ten seconds.
            assert!(hold.tick_if_due_at(&mut sender, Duration::from_secs(10)));

            // Exactly one tick, and the next is a normal interval away --
            // not a thousand ticks owed at zero spacing.
            assert_eq!(sender.sent.lock().len(), 1);
            assert_eq!(
                hold.until_next_at(Duration::from_secs(10)),
                Duration::from_millis(10)
            );

            // And it keeps pacing normally from the new anchor.
            assert!(!hold.tick_if_due_at(&mut sender, Duration::from_millis(10_005)));
            assert!(hold.tick_if_due_at(&mut sender, Duration::from_millis(10_010)));
            assert_eq!(sender.sent.lock().len(), 2);
        }

        /// The wait timeout never goes negative or saturates oddly when the
        /// tick is already overdue.
        #[test]
        fn an_overdue_tick_asks_for_no_wait() {
            let hold = TempoHold::new(Duration::from_millis(10));
            assert_eq!(
                hold.until_next_at(Duration::from_millis(50)),
                Duration::ZERO
            );
        }
    }

    /// Polls until `cond` holds or the deadline passes.
    ///
    /// These tests assert that a free-running clock keeps running at all,
    /// not that it hits a precise count in a precise window. A shared CI
    /// runner can starve this thread for tens of milliseconds at a time, so
    /// a fixed sleep plus an exact count is a flake waiting to happen --
    /// which is exactly how both of these first landed. Polling makes a slow
    /// machine slower rather than red.
    fn wait_for(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if cond() {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        cond()
    }

    /// Held ticks required before believing the clock is really running.
    ///
    /// At most one tick can slip out between sampling the baseline and the
    /// engine picking the queued command up, so anything above that
    /// separates "held the tempo" from "went silent and emitted a straggler".
    const HELD_TICKS: usize = 5;

    /// An engine started holding a tempo, for a device opened afresh,
    /// free-runs it before any song, and a song's tempo replaces it
    /// where the device can read it.
    #[test]
    fn an_engine_started_holding_a_tempo_runs_it_and_keeps_it_current() {
        let (tx, rx) = mpsc::channel();
        let sender = SharedSender::new();
        let loop_sender = sender.clone();
        let tempo = Arc::new(Mutex::new(Some(Duration::from_millis(10))));
        let handle = thread::spawn({
            let tempo = tempo.clone();
            move || {
                let mut sender = loop_sender;
                beat_clock_engine_loop(&mut sender, true, &tempo, &rx);
            }
        });

        assert!(
            wait_for(Duration::from_secs(10), || sender.len() >= HELD_TICKS),
            "the held tempo was not run before a song"
        );
        assert!(sender.sent.lock().iter().all(|msg| *msg == clock_bytes()));

        tx.send(BeatClockCommand::Play(ready_play(
            vec![
                Duration::from_millis(20),
                Duration::from_millis(40),
                Duration::from_millis(60),
            ],
            CancelHandle::new(),
        )))
        .unwrap();
        let set = wait_for(Duration::from_secs(10), || {
            *tempo.lock().unwrap() == Some(Duration::from_millis(20))
        });
        tx.send(BeatClockCommand::Shutdown).unwrap();
        handle.join().unwrap();
        assert!(
            set,
            "the song's tempo was not kept: {:?}",
            tempo.lock().unwrap()
        );
    }

    /// The gap between "user pressed play" and "every subsystem is ready"
    /// is the boundary persistence exists for, and the engine spends it
    /// waiting on the shared clock. It must keep the tempo running there.
    #[test]
    fn holds_tempo_while_waiting_for_the_go_signal() {
        let (tx, rx) = mpsc::channel();
        let sender = SharedSender::new();
        let loop_sender = sender.clone();
        let handle = thread::spawn(move || {
            let mut sender = loop_sender;
            beat_clock_engine_loop(&mut sender, true, &Mutex::default(), &rx);
        });

        // First song establishes a 20ms tempo and ends: START + 3 clocks +
        // STOP, then held ticks.
        tx.send(BeatClockCommand::Play(ready_play(
            vec![
                Duration::from_millis(20),
                Duration::from_millis(40),
                Duration::from_millis(60),
            ],
            CancelHandle::new(),
        )))
        .unwrap();
        assert!(
            wait_for(Duration::from_secs(10), || sender.len() > 5),
            "the first song never established a held tempo"
        );
        let before = sender.len();

        // Second song arrives but its clock has NOT started: the player is
        // still opening devices and loading samples.
        let slow_clock = PlaybackClock::wall();
        tx.send(BeatClockCommand::Play(BeatClockPlay {
            ticks: Arc::new(vec![Duration::from_millis(20), Duration::from_millis(40)]),
            start_time: Duration::ZERO,
            playback_delay: Duration::ZERO,
            cancel: CancelHandle::new(),
            clock: slow_clock.clone(),
            sections: None,
            transport: None,
        }))
        .unwrap();

        // The go signal is withheld for this whole window, so everything
        // arriving is the held tempo. Blocking silently on the clock -- the
        // behaviour this guards against -- emits nothing here at all.
        let held = wait_for(Duration::from_secs(10), || {
            sender.len() >= before + HELD_TICKS
        });
        let during = sender.len();
        assert!(
            held,
            "clock went silent while waiting for the go signal: {} messages in 10s",
            during - before
        );

        // Releasing the go signal must still let the schedule play.
        slow_clock.start();
        assert!(
            wait_for(Duration::from_secs(10), || sender.len() > during),
            "the schedule never ran once the clock started"
        );
        tx.send(BeatClockCommand::Shutdown).unwrap();
        handle.join().unwrap();

        // The Start belongs with the first note, not with the wait.
        let sent = sender.sent.lock();
        for msg in &sent[before..during] {
            assert_eq!(*msg, clock_bytes(), "only timing clocks may be held");
        }
    }

    /// A schedule too short to establish a tempo must not erase the one
    /// already being held.
    #[test]
    fn a_schedule_with_too_few_ticks_keeps_the_previous_tempo() {
        let (tx, rx) = mpsc::channel();
        let sender = SharedSender::new();
        let loop_sender = sender.clone();
        let handle = thread::spawn(move || {
            let mut sender = loop_sender;
            beat_clock_engine_loop(&mut sender, true, &Mutex::default(), &rx);
        });

        // Establish a tempo.
        tx.send(BeatClockCommand::Play(ready_play(
            vec![
                Duration::from_millis(20),
                Duration::from_millis(40),
                Duration::from_millis(60),
            ],
            CancelHandle::new(),
        )))
        .unwrap();
        assert!(
            wait_for(Duration::from_secs(10), || sender.len() > 5),
            "the first song never established a held tempo"
        );

        // A one-tick schedule: `run_beat_clock` cannot derive an interval
        // from it and reports None. Its own output is START + 1 clock + STOP.
        let before = sender.len();
        tx.send(BeatClockCommand::Play(ready_play(
            vec![Duration::from_millis(5)],
            CancelHandle::new(),
        )))
        .unwrap();
        assert!(
            wait_for(Duration::from_secs(10), || sender.len() >= before + 3),
            "the short schedule never played"
        );
        let after_short = sender.len();

        let still_running = wait_for(Duration::from_secs(10), || {
            sender.len() >= after_short + HELD_TICKS
        });
        tx.send(BeatClockCommand::Shutdown).unwrap();
        handle.join().unwrap();

        assert!(
            still_running,
            "the short schedule wiped the held tempo: {} messages in the 10s after it",
            sender.len() - after_short
        );
    }

    /// Nothing cancels a song's schedule when it ends naturally -- the
    /// cleanup path only clears the join and advances the playlist -- so a
    /// MIDI file longer than its audio leaves the engine still running the
    /// old grid. The next song must take over at once rather than queue
    /// behind it.
    #[test]
    fn a_new_schedule_preempts_a_running_one() {
        let (tx, rx) = mpsc::channel();
        let sender = SharedSender::new();
        let loop_sender = sender.clone();
        let handle = thread::spawn(move || {
            let mut sender = loop_sender;
            beat_clock_engine_loop(&mut sender, false, &Mutex::default(), &rx);
        });

        // Two seconds of schedule, far longer than this test waits.
        let long: Vec<Duration> = (1..=200).map(|i| Duration::from_millis(i * 10)).collect();
        tx.send(BeatClockCommand::Play(ready_play(
            long,
            CancelHandle::new(),
        )))
        .unwrap();
        thread::sleep(Duration::from_millis(40));

        // The next song, arriving while the first schedule is still going.
        tx.send(BeatClockCommand::Play(ready_play(
            vec![Duration::from_millis(5), Duration::from_millis(10)],
            CancelHandle::new(),
        )))
        .unwrap();
        thread::sleep(Duration::from_millis(80));

        tx.send(BeatClockCommand::Shutdown).unwrap();
        handle.join().unwrap();

        // Two Starts within ~120ms. Queued behind the first schedule, the
        // second would not have started for another two seconds.
        let sent = sender.sent.lock();
        let starts = sent.iter().filter(|m| **m == start_bytes()).count();
        assert_eq!(
            starts,
            2,
            "the second song did not preempt the first: {starts} Start(s) in {} messages",
            sent.len()
        );
    }

    /// Shutdown must be observed mid-schedule. Device teardown runs on a
    /// tokio worker and joins this thread, so waiting out a whole song there
    /// blocks a runtime thread for the length of the song.
    #[test]
    fn shutdown_is_observed_during_a_schedule() {
        let (tx, rx) = mpsc::channel();
        let sender = SharedSender::new();
        let loop_sender = sender.clone();
        let handle = thread::spawn(move || {
            let mut sender = loop_sender;
            beat_clock_engine_loop(&mut sender, false, &Mutex::default(), &rx);
        });

        let long: Vec<Duration> = (1..=200).map(|i| Duration::from_millis(i * 10)).collect();
        tx.send(BeatClockCommand::Play(ready_play(
            long,
            CancelHandle::new(),
        )))
        .unwrap();
        thread::sleep(Duration::from_millis(30));

        let sent_shutdown = std::time::Instant::now();
        tx.send(BeatClockCommand::Shutdown).unwrap();
        handle.join().unwrap();
        let waited = sent_shutdown.elapsed();

        assert!(
            waited < Duration::from_millis(500),
            "shutdown waited {waited:?} for the schedule to finish"
        );
    }

    #[test]
    fn shutdown_before_any_schedule_sends_nothing() {
        let (tx, rx) = mpsc::channel();
        let mut sender = SharedSender::new();

        tx.send(BeatClockCommand::Shutdown).unwrap();
        // persist on, but no tempo known yet → must block on recv, then exit.
        beat_clock_engine_loop(&mut sender, true, &Mutex::default(), &rx);

        assert!(sender.sent.lock().is_empty());
    }

    #[test]
    fn plays_schedule_then_holds_tempo_when_persisting() {
        let (tx, rx) = mpsc::channel();
        let sender = SharedSender::new();
        let loop_sender = sender.clone();
        let handle = thread::spawn(move || {
            let mut sender = loop_sender;
            beat_clock_engine_loop(&mut sender, true, &Mutex::default(), &rx);
        });

        // A 3-tick schedule at ~10ms spacing; final interval = 10ms.
        let cancel = CancelHandle::new();
        tx.send(BeatClockCommand::Play(ready_play(
            vec![
                Duration::from_millis(10),
                Duration::from_millis(20),
                Duration::from_millis(30),
            ],
            cancel,
        )))
        .unwrap();

        // Wait for the schedule (START + 3 clocks + STOP = 5) plus some held
        // ticks to accumulate. Polled rather than slept: a starved thread
        // makes this slower, not wrong.
        let held_ticks = wait_for(Duration::from_secs(10), || sender.len() > 5);
        let held = sender.len();
        assert!(
            held_ticks,
            "expected schedule (5 msgs) plus held ticks, got {held}"
        );

        tx.send(BeatClockCommand::Shutdown).unwrap();
        handle.join().unwrap();

        let sent = sender.sent.lock();
        // Schedule framing is intact at the front.
        assert_eq!(sent[0], start_bytes());
        assert_eq!(sent[1], clock_bytes());
        assert_eq!(sent[2], clock_bytes());
        assert_eq!(sent[3], clock_bytes());
        assert_eq!(sent[4], stop_bytes());
        // Everything after the STOP is a held timing clock.
        for msg in &sent[5..] {
            assert_eq!(*msg, clock_bytes(), "held tempo must be timing clocks");
        }
    }

    #[test]
    fn stays_silent_between_songs_without_persist() {
        let (tx, rx) = mpsc::channel();
        let sender = SharedSender::new();
        let loop_sender = sender.clone();
        let handle = thread::spawn(move || {
            let mut sender = loop_sender;
            beat_clock_engine_loop(&mut sender, false, &Mutex::default(), &rx);
        });

        let cancel = CancelHandle::new();
        tx.send(BeatClockCommand::Play(ready_play(
            vec![Duration::from_millis(5), Duration::from_millis(10)],
            cancel,
        )))
        .unwrap();

        // Wait for the schedule to finish, then hold still and confirm
        // nothing follows it. The negative half stays a fixed window on
        // purpose: load can only make a silent engine emit *less*, so it
        // cannot turn this green when it should be red.
        assert!(
            wait_for(Duration::from_secs(10), || sender.len() >= 4),
            "the schedule never played, got {}",
            sender.len()
        );
        thread::sleep(Duration::from_millis(200));
        let count = sender.len();
        // START + 2 clocks + STOP = 4, and nothing more without persistence.
        assert_eq!(count, 4, "expected exactly the schedule, got {count}");

        tx.send(BeatClockCommand::Shutdown).unwrap();
        handle.join().unwrap();

        let sent = sender.sent.lock();
        assert_eq!(sent[0], start_bytes());
        assert_eq!(sent[3], stop_bytes());
    }

    #[test]
    fn queued_schedule_preempts_idle_without_blip() {
        // With a schedule already queued, next_command must take it via
        // try_recv rather than emitting an idle tick first.
        let (tx, rx) = mpsc::channel();
        let mut sender = SharedSender::new();

        let cancel = CancelHandle::new();
        tx.send(BeatClockCommand::Play(ready_play(
            vec![Duration::from_millis(1)],
            cancel,
        )))
        .unwrap();

        let mut hold = None;
        let command = next_command(
            &mut sender,
            true,
            Some(Duration::from_millis(10)),
            &mut hold,
            &rx,
        );
        assert!(matches!(command, Some(BeatClockCommand::Play(_))));
        // No idle tick was emitted while a command was already waiting.
        assert!(sender.sent.lock().is_empty());
    }

    #[test]
    fn cancelled_schedule_is_skipped() {
        let (tx, rx) = mpsc::channel();
        let sender = SharedSender::new();
        let loop_sender = sender.clone();
        let handle = thread::spawn(move || {
            let mut sender = loop_sender;
            beat_clock_engine_loop(&mut sender, false, &Mutex::default(), &rx);
        });

        // A schedule whose clock never starts and is already cancelled: the
        // engine's cancel-aware wait must let it through without wedging.
        let cancel = CancelHandle::new();
        cancel.cancel();
        let clock = PlaybackClock::wall();
        tx.send(BeatClockCommand::Play(BeatClockPlay {
            ticks: Arc::new(vec![Duration::from_millis(10)]),
            start_time: Duration::ZERO,
            playback_delay: Duration::ZERO,
            cancel,
            clock,
            sections: None,
            transport: None,
        }))
        .unwrap();

        // The engine must remain responsive to Shutdown afterward.
        tx.send(BeatClockCommand::Shutdown).unwrap();
        handle.join().unwrap();
        assert!(sender.sent.lock().is_empty());
    }

    #[test]
    fn withholds_start_until_the_clock_go_signal() {
        // Every other test uses `ready_play`, whose clock is already
        // started, so the engine's `wait_for_start_or_cancel` returns
        // immediately and the wait window is zero. This exercises a
        // non-zero window: the engine must take the Play command, then emit
        // nothing until the shared clock is started, so its Start lands with
        // the first note rather than ahead of it.
        let (tx, rx) = mpsc::channel();
        let sender = SharedSender::new();
        let loop_sender = sender.clone();
        let handle = thread::spawn(move || {
            let mut sender = loop_sender;
            beat_clock_engine_loop(&mut sender, false, &Mutex::default(), &rx);
        });

        // A schedule whose clock has NOT been started yet.
        let cancel = CancelHandle::new();
        let clock = PlaybackClock::wall();
        tx.send(BeatClockCommand::Play(BeatClockPlay {
            ticks: Arc::new(vec![Duration::from_millis(5), Duration::from_millis(10)]),
            start_time: Duration::ZERO,
            playback_delay: Duration::ZERO,
            cancel,
            clock: clock.clone(),
            sections: None,
            transport: None,
        }))
        .unwrap();

        // While the clock is unstarted the engine blocks in
        // `wait_for_start_or_cancel` and must emit nothing — not even Start.
        thread::sleep(Duration::from_millis(40));
        assert_eq!(sender.len(), 0, "engine emitted before the go signal");

        // Releasing the go signal lets the schedule play.
        clock.start();
        assert!(
            wait_for(Duration::from_secs(10), || sender.len() >= 4),
            "schedule should play once the clock starts, got {}",
            sender.len()
        );

        tx.send(BeatClockCommand::Shutdown).unwrap();
        handle.join().unwrap();

        // START + 2 clocks + STOP, framed correctly, and (no persist) nothing
        // after.
        let sent = sender.sent.lock();
        assert_eq!(
            sent.len(),
            4,
            "expected exactly the schedule, got {}",
            sent.len()
        );
        assert_eq!(sent[0], start_bytes());
        assert_eq!(sent[1], clock_bytes());
        assert_eq!(sent[2], clock_bytes());
        assert_eq!(sent[3], stop_bytes());
    }

    #[test]
    fn run_idle_until_command_returns_command_and_emits_ticks() {
        let (tx, rx) = mpsc::channel();
        let sender = SharedSender::new();
        let loop_sender = sender.clone();
        let handle = thread::spawn(move || {
            let mut sender = loop_sender;
            let mut hold = TempoHold::new(Duration::from_millis(10));
            let cmd = run_idle_until_command(&mut sender, &mut hold, &rx);
            assert!(matches!(cmd, Some(BeatClockCommand::Shutdown)));
        });

        let ticked = wait_for(Duration::from_secs(10), || !sender.sent.lock().is_empty());
        tx.send(BeatClockCommand::Shutdown).unwrap();
        handle.join().unwrap();

        let sent = sender.sent.lock();
        assert!(ticked, "idle free-run should emit timing clocks");
        for msg in sent.iter() {
            assert_eq!(*msg, clock_bytes());
        }
    }
}
