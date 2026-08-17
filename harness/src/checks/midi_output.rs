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
//! What mtrack puts on the MIDI wire during playback.
//!
//! These are the only cases that check MIDI against something other than
//! mtrack's own account of itself.

use std::time::Duration;

use mtrack::proto::player::v1::{NextRequest, PlayRequest, StopRequest};

use crate::capabilities::Capabilities;
use crate::client::Client;
use crate::discovery::Discovery;
use crate::midi::{self, MidiCapture};
use crate::outcome::CheckOutcome;
use crate::project::{ProfileSpec, ProjectBuilder, Subsystem};
use crate::server::Server;
use crate::songs::{MidiSpec, SongSpec};
use crate::{check, check_eq, inconclusive, skip};

/// Tempo of the generated MIDI song. Chosen away from round numbers so a clock
/// derived from a default rather than from the file is obvious.
const TEMPO_BPM: f32 = 104.0;

/// How long to keep waiting for a port that will not go quiet.
///
/// Long enough to outlast traffic still draining from a stopped player, short
/// enough that a genuinely busy rig is reported rather than hung on — the runner
/// awaits checks without a timeout, so an unbounded wait is a silent hang of the
/// whole suite.
const SETTLE_DEADLINE: Duration = Duration::from_secs(10);

/// How long the port must stay silent before a capture is considered clean.
///
/// Two beats at [`TEMPO_BPM`]. The generated files hold each note for three
/// quarters of a beat, so the longest gap inside a *playing* stream is most of a
/// beat, and the beat clock's 24 PPQN is a 24ms gap — both well inside this.
///
/// It does not bound the rig, only the last window: another run's structural
/// silences are longer than this (a 2s pause between songs, 3s post-stop
/// observation windows, server teardown and startup), so a drain can go quiet
/// mid-way through someone else's timeline. One run at a time is the actual
/// guarantee; this only catches traffic still in flight.
fn quiet_window() -> Duration {
    Duration::from_secs_f32(2.0 * 60.0 / TEMPO_BPM)
}

/// Waits for the port to go quiet before a check captures its own traffic, and
/// reports what it found.
///
/// Every check here cleared the buffer and then acted, which does not empty the
/// driver queue — so a previous check's clock or notes arrive afterwards and are
/// counted as this one's (#378).
///
/// A port that never settles fails the run, but as a *before-assertion* defect.
/// The class matters twice over. `fail!` would mint an assertion-class defect
/// from a helper, and `--self-test` reads that flag: on a busy rig it would
/// print "assertion fired" for all six of these checks, crediting them as
/// proven capable of failing when their sabotage was never evaluated — the false
/// proof the flag exists to prevent. `inconclusive!` errs the other way, since
/// `Outcome::is_bad` counts only Failed and HarnessError, so the run would exit
/// zero and report no defect while six checks tested nothing.
///
/// The message names both possible causes. mtrack transmitting before anything
/// was asked to play is itself a defect, and blaming the environment for it
/// would hide exactly the build `beat_clock_is_silent_when_disabled` exists to
/// catch — so the breakdown by message type is reported, since note-ons mean
/// another player and clock pulses mean a clock that should not be running.
async fn settle_port(capture: &MidiCapture) -> Result<(), crate::outcome::CheckError> {
    let outcome = capture.drain(quiet_window(), SETTLE_DEADLINE).await;

    // Note-*ons* specifically: `note()` also matches note-offs, and a player
    // being torn down emits a trailing note-off burst. Counting those as
    // "another player is on the port" would steer an operator at traffic that
    // contains no note-on at all.
    let note_ons = outcome.discarded.iter().filter(|m| m.is_note_on()).count();
    let note_offs = outcome
        .discarded
        .iter()
        .filter(|m| m.note().is_some() && !m.is_note_on())
        .count();
    let pulses = outcome
        .discarded
        .iter()
        .filter(|m| m.status() == Some(midi::TIMING_CLOCK))
        .count();
    let other = outcome.discarded.len() - note_ons - note_offs - pulses;
    let breakdown = format!(
        "{note_ons} note-on(s), {note_offs} note-off(s), {pulses} clock pulse(s), {other} other"
    );

    if !outcome.went_quiet {
        // `before_assertion`, not `inconclusive!`: `Outcome::is_bad` counts only
        // Failed and HarnessError, so an inconclusive result exits 0 and
        // `run_repeated` reports "no check reported an mtrack defect". One of the
        // two causes named below *is* a defect — a build transmitting before
        // anything was asked to play — and it would have produced a green run
        // with six checks silently testing nothing. This class fails the run
        // while still telling `--self-test` the truth: it proves nothing as a
        // negative control, because the check's own assertion never ran.
        return Err(crate::outcome::CheckError::before_assertion(format!(
            "the MIDI port never went quiet across {}s: {} kept arriving before this check \
             asked for anything, so nothing it captured would be attributable to it. Either \
             something else is using the rig — only one harness run at a time — or this build \
             is transmitting when nothing has been asked to play, which is itself a defect.",
            SETTLE_DEADLINE.as_secs(),
            breakdown
        )));
    }
    // Recorded either way. A quiet window proves nothing about the rig, only
    // about that window — a concurrent run sitting in its own inter-song pause
    // is silent for longer than this — so "the port looked idle" is itself the
    // evidence a reader needs when a recurrence has to be explained.
    if outcome.discarded.is_empty() {
        crate::outcome::record("the MIDI port was already idle before play was requested");
    } else {
        crate::outcome::record(format!(
            "caveat: {breakdown} were on the port before play was requested, and were discarded"
        ));
    }
    Ok(())
}

/// Tempo of the second song in the between-songs checks. Distinct from
/// [`TEMPO_BPM`] by enough that a clock still running at the first song's tempo
/// is unmistakable, and equally un-round.
const SECOND_TEMPO_BPM: f32 = 152.0;

/// How long the beat clock may go silent across a song boundary before the
/// tempo it is meant to be holding has, from downstream gear's point of view,
/// been dropped.
///
/// Four ticks at [`TEMPO_BPM`] (~96 ms), not a round number of milliseconds,
/// because the quantity that matters downstream is missing *pulses*.
///
/// Two of those four are structural and cannot be removed: a song's ticks sit on
/// its own beat grid, so the last held tick and the new song's first tick are up
/// to an interval apart at each end of the handover. The measured gap on the
/// test rig is a stable 48 ms, so this catches a doubling while leaving room for
/// a loaded machine.
///
/// What this does *not* cover is the startup window -- the stretch between the
/// play request and every subsystem reporting ready. On a fast rig that is ~40
/// ms whether or not the engine holds tempo through it, so an absolute budget
/// cannot tell the two apart. That case is pinned by the unit test
/// `holds_tempo_while_waiting_for_the_go_signal`, which drives a non-zero wait
/// directly.
const MAX_BOUNDARY_SILENCE: Duration =
    Duration::from_micros((4.0 * 60.0 / (TEMPO_BPM as f64 * 24.0) * 1_000_000.0) as u64);

/// How long the player sits stopped between the two songs.
///
/// Must be comfortably longer than [`MAX_BOUNDARY_SILENCE`], and that is the
/// entire point. Driving `stop → next → play` back to back leaves only the
/// player's own startup latency to bridge -- about 40 ms on the test rig, which
/// an engine with no persistence at all covers just as well as one with it. The
/// first version of this check did exactly that and could not fail. A real gap
/// between songs is an operator talking to an audience, and it is measured in
/// seconds; that is the gap `persist_tempo` exists for, so it is the gap this
/// check has to reproduce.
const BETWEEN_SONGS_PAUSE: Duration = Duration::from_secs(2);

/// The beat clock settings a generated project applies to its profile.
#[derive(Clone, Copy, Default)]
struct BeatClock {
    enabled: bool,
    persist_tempo: bool,
}

impl BeatClock {
    fn off() -> BeatClock {
        BeatClock::default()
    }

    fn on() -> BeatClock {
        BeatClock {
            enabled: true,
            persist_tempo: false,
        }
    }

    fn persisting(persist: bool) -> BeatClock {
        BeatClock {
            enabled: true,
            persist_tempo: persist,
        }
    }
}

/// Builds a project from the given songs, with the given beat clock settings.
fn midi_project_with(
    songs: Vec<SongSpec>,
    clock: BeatClock,
) -> Result<crate::project::Project, Box<dyn std::error::Error>> {
    let mut profile = ProfileSpec::detected("01-e2e");
    if let Some(pair) = Discovery::get().midi_pair() {
        profile.midi = Subsystem::Named(pair.out_port.clone());
    }
    if clock.enabled {
        profile = profile.with_midi_key("beat_clock", "true");
    }
    if clock.persist_tempo {
        profile = profile.with_midi_key("persist_tempo", "true");
    }

    ProjectBuilder::new()
        .profiles(vec![profile])
        .songs(songs)
        .build()
}

/// Builds a project whose song carries a MIDI file, optionally with beat clock.
fn midi_project(beat_clock: bool) -> Result<crate::project::Project, Box<dyn std::error::Error>> {
    let song = SongSpec::tones("Midi Song", "midi-song", 1, 10.0)
        .with_midi(MidiSpec::scale("song.mid", TEMPO_BPM, 8));
    let clock = if beat_clock {
        BeatClock::on()
    } else {
        BeatClock::off()
    };
    midi_project_with(vec![song], clock)
}

/// A song whose MIDI runs the full length of its audio.
///
/// The beat clock follows the *MIDI file*, not the song: when the schedule runs
/// out mid-song the engine sends Stop and, with persistence on, begins holding
/// tempo while audio is still playing. A check that meant to observe the
/// between-songs state would then be observing the end-of-schedule state
/// instead, and would pass for the wrong reason. Sizing the MIDI to outlast the
/// audio keeps "the schedule ended" and "the song ended" from being confused.
fn covered_song(name: &str, dir: &str, tempo_bpm: f32, secs: f32) -> SongSpec {
    let beats = (secs * tempo_bpm / 60.0).ceil() as usize + 4;
    SongSpec::tones(name, dir, 1, secs).with_midi(MidiSpec::scale("song.mid", tempo_bpm, beats))
}

/// The longest silence between consecutive timing clocks, and how far into the
/// run it began.
///
/// Measured on the harness's own arrival clock, never the driver's `micros`:
/// the latter does not advance while the port is quiet, so every silence
/// measures as roughly zero and a check built on it passes whatever mtrack
/// does. That is exactly how the first version of this check reported a 48 ms
/// boundary gap with persistence on and 49 ms with it off.
///
/// Reported rather than merely compared so a failure says *how* badly the clock
/// stalled -- a 300 ms gap and a 4 s gap are the same boolean and very different
/// defects.
fn longest_clock_gap(pulses: &[midi::TimedMessage]) -> Option<(Duration, Duration)> {
    let first = pulses.first()?.at;
    pulses
        .windows(2)
        .map(|w| {
            (
                w[1].at.saturating_duration_since(w[0].at),
                w[0].at.saturating_duration_since(first),
            )
        })
        .max_by_key(|(gap, _)| *gap)
}

/// The ports these checks transmit and listen on.
struct MidiPath {
    send: String,
    listen: String,
}

impl MidiPath {
    /// Describes the path, stating plainly when it is not the hardware the
    /// operator would actually use.
    ///
    /// Verification needs a loopback, and the only port that loops is often a
    /// software one. That makes the strongest MIDI assertion run on the least
    /// production-like path -- acceptable, but only if every result says so.
    /// Otherwise a reader sees "beat clock verified" and reasonably concludes
    /// it was verified through their interface.
    fn describe(&self) -> String {
        let configured = Capabilities::get()
            .midi_out
            .as_ref()
            .map(|d| d.name.clone())
            .unwrap_or_else(|| "none".to_string());

        if self.send == configured {
            format!("via {} (the configured MIDI device)", self.send)
        } else {
            format!(
                "via {} -- a loopback port, NOT the configured device ({configured}), which has \
                 no loopback to verify through",
                self.send
            )
        }
    }
}

/// The loopback path to verify over, or an error explaining why there is none.
fn midi_path() -> Result<MidiPath, crate::outcome::CheckError> {
    crate::runner::require_area("midi-transmit")?;
    match Discovery::get().midi_pair() {
        Some(pair) => Ok(MidiPath {
            send: pair.out_port.clone(),
            listen: pair.in_port.clone(),
        }),
        None => Err(crate::outcome::CheckError::Skipped(
            "no MIDI port loops back, so transmitted bytes cannot be observed".to_string(),
        )),
    }
}

/// The notes in a song's MIDI file are actually transmitted.
pub async fn song_midi_notes_are_transmitted() -> CheckOutcome {
    let path = midi_path()?;
    let listen = path.listen.clone();
    crate::outcome::record(path.describe());

    let project = midi_project(false)?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    let capture = MidiCapture::open(&listen)?;
    // `clear` alone does not empty the driver queue, so a previous check's
    // traffic would be counted as this check's. `settle_port` subsumes it, and
    // clearing first would discard the evidence it exists to report.
    settle_port(&capture).await?;

    client.grpc().play(PlayRequest {}).await?;
    client.wait_until_playing(Duration::from_secs(10)).await?;

    // The generated file has eight notes, one per beat.
    let expected_notes = 8;
    let arrived = midi::wait_for_messages(&capture, Duration::from_secs(15), |c| {
        c.note_ons().len() >= expected_notes
    })
    .await;

    client.grpc().stop(StopRequest {}).await?;

    let notes = capture.note_ons();
    // The full timeline, with inter-note spacing. Two sources sending the same
    // file put notes at roughly half the expected spacing; one source playing
    // twice leaves a gap and then restarts. The failure signature — the first
    // four notes twice, in half the time a clean pass takes — has to be one or
    // the other, and these numbers say which.
    let timeline: Vec<String> = notes
        .iter()
        .scan(None::<std::time::Instant>, |prev, n| {
            let delta = prev.map(|p| n.at.saturating_duration_since(p));
            *prev = Some(n.at);
            Some(match (n.note(), delta) {
                (Some(note), Some(d)) => format!("{note}(+{}ms)", d.as_millis()),
                (Some(note), None) => format!("{note}(first)"),
                _ => "?".to_string(),
            })
        })
        .collect();
    crate::outcome::record(format!("note timeline: {}", timeline.join(" ")));

    check!(
        arrived,
        "expected {expected_notes} note-on messages, captured {}: {:?}\n--- log ---\n{}",
        notes.len(),
        notes.iter().filter_map(|n| n.note()).collect::<Vec<_>>(),
        server.log()
    );

    // The generated file ascends from middle C, so the transmitted notes must
    // match in both value and order.
    let received: Vec<u8> = notes
        .iter()
        .take(expected_notes)
        .filter_map(|n| n.note())
        .collect();
    let expected: Vec<u8> = (0..expected_notes)
        .map(|b| crate::sabotage::pick(60, 90) + (b as u8 % 12))
        .collect();
    check_eq!(
        received,
        expected,
        "the transmitted notes do not match the song's MIDI file"
    );
    crate::outcome::record(format!("transmitted notes: {received:?}"));

    server.check_clean_log(&[])?;
    Ok(())
}

/// With beat clock enabled, mtrack emits 24 pulses per quarter note at the
/// song's tempo.
///
/// The rate is what makes this meaningful: any implementation can emit 0xF8
/// bytes, but only a correct one emits them at `bpm * 24 / 60` Hz.
pub async fn beat_clock_runs_at_the_song_tempo() -> CheckOutcome {
    let path = midi_path()?;
    let listen = path.listen.clone();
    crate::outcome::record(path.describe());

    let project = midi_project(true)?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    let capture = MidiCapture::open(&listen)?;
    settle_port(&capture).await?;

    client.grpc().play(PlayRequest {}).await?;
    client.wait_until_playing(Duration::from_secs(10)).await?;

    // Measure over a window long enough that a few pulses of jitter cannot
    // shift the average appreciably.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let pulses = capture.clock_pulses();
    let measured = capture.clock_hz();
    client.grpc().stop(StopRequest {}).await?;

    check!(
        pulses.len() > 24,
        "beat_clock was enabled but only {} timing clock pulses arrived; \
         MIDI beat clock does not appear to be transmitted.\n--- log ---\n{}",
        pulses.len(),
        server.log()
    );

    let expected = MidiSpec::scale(
        "song.mid",
        crate::sabotage::pick(TEMPO_BPM, TEMPO_BPM * 2.0),
        8,
    )
    .expected_clock_hz();
    // `clock_hz` also returns None when the span is zero, which a backend
    // without real port timestamping can produce. Panicking there would surface
    // as a harness error -- the one outcome a measurement should never yield.
    let Some(measured) = measured else {
        inconclusive!(
            "{} clock pulses arrived but carried no usable timestamps, so the rate could not \
             be measured",
            pulses.len()
        );
    };
    let error_pct = ((measured - expected) / expected).abs() * 100.0;

    crate::outcome::record(format!(
        "beat clock: {measured:.2} Hz measured, {expected:.2} Hz expected \
         ({error_pct:.1}% off, {} pulses)",
        pulses.len()
    ));

    // 5% accommodates scheduling jitter on a non-realtime kernel while still
    // catching a wrong tempo, a wrong pulse count per beat, or a fixed rate.
    check!(
        error_pct < 5.0,
        "beat clock ran at {measured:.2} Hz but {TEMPO_BPM} BPM requires {expected:.2} Hz \
         (24 PPQN); that is {error_pct:.1}% off"
    );

    server.check_clean_log(&[])?;
    Ok(())
}

/// With beat clock disabled, no timing clock is transmitted.
///
/// Without this, the rate check above would still pass against a build that
/// ignores the setting and always emits a clock.
pub async fn beat_clock_is_silent_when_disabled() -> CheckOutcome {
    let path = midi_path()?;
    let listen = path.listen.clone();
    crate::outcome::record(path.describe());

    let project = midi_project(crate::sabotage::pick(false, true))?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    let capture = MidiCapture::open(&listen)?;
    settle_port(&capture).await?;

    client.grpc().play(PlayRequest {}).await?;
    client.wait_until_playing(Duration::from_secs(10)).await?;
    tokio::time::sleep(Duration::from_secs(3)).await;

    let pulses = capture.clock_pulses();
    client.grpc().stop(StopRequest {}).await?;

    check!(
        pulses.is_empty(),
        "beat_clock was not enabled, but {} timing clock pulses were transmitted",
        pulses.len()
    );

    server.check_clean_log(&[])?;
    Ok(())
}

/// Pulses per second across a run of timing clocks, or `None` if there are too
/// few to measure.
fn hz_of(pulses: &[midi::TimedMessage]) -> Option<f32> {
    if pulses.len() < 2 {
        return None;
    }
    let span = pulses.last()?.micros.checked_sub(pulses.first()?.micros)?;
    if span == 0 {
        return None;
    }
    Some((pulses.len() - 1) as f32 * 1_000_000.0 / span as f32)
}

/// The clock rate a tempo implies at 24 pulses per quarter note.
fn expected_hz(tempo_bpm: f32) -> f32 {
    MidiSpec::scale("song.mid", tempo_bpm, 8).expected_clock_hz()
}

/// With `persist_tempo` on, the clock keeps running at the song's tempo after
/// the song stops.
///
/// This is the whole point of the option: downstream gear that syncs to the
/// clock -- tempo-synced delays, LFOs, arpeggiators, tempo displays -- holds the
/// tempo through the gap instead of drifting or resetting. Verifying it needs
/// the wire, because mtrack's own reporting says nothing about what happens
/// after playback ends.
pub async fn beat_clock_holds_tempo_after_a_song_stops() -> CheckOutcome {
    let path = midi_path()?;
    let listen = path.listen.clone();
    crate::outcome::record(path.describe());

    let song = covered_song("Midi Song", "midi-song", TEMPO_BPM, 12.0);
    let project = midi_project_with(
        vec![song],
        BeatClock::persisting(crate::sabotage::pick(true, false)),
    )?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    let capture = MidiCapture::open(&listen)?;
    settle_port(&capture).await?;

    client.grpc().play(PlayRequest {}).await?;
    client.wait_until_playing(Duration::from_secs(10)).await?;
    tokio::time::sleep(Duration::from_secs(4)).await;

    // The premise: a clock was running at all. Without this, a build that never
    // started one would reach the held-tempo assertion with nothing to hold and
    // fail there, blaming persistence for a plain beat clock failure.
    let during = capture.clock_pulses();
    check!(
        during.len() > 24,
        "beat_clock was enabled but only {} pulses arrived during playback, so there was no \
         tempo to hold.\n--- log ---\n{}",
        during.len(),
        server.log()
    );

    client.grpc().stop(StopRequest {}).await?;

    // Let the Stop and the handover into free-run settle, so trailing in-song
    // ticks are not measured as held ones.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let saw_stop = capture
        .messages()
        .iter()
        .any(|m| m.status() == Some(midi::STOP));
    capture.clear();
    tokio::time::sleep(Duration::from_secs(3)).await;

    let held = capture.clock_pulses();
    let held_hz = hz_of(&held);

    check!(
        saw_stop,
        "no MIDI Stop (0xFC) was transmitted when the song stopped; downstream gear would still \
         think the transport is running"
    );

    check!(
        !held.is_empty(),
        "persist_tempo was on, but the beat clock went silent after the song stopped: no timing \
         clocks in the 3s following the Stop.\n--- log ---\n{}",
        server.log()
    );

    let expected = expected_hz(TEMPO_BPM);
    let Some(held_hz) = held_hz else {
        inconclusive!(
            "{} clocks were held after the stop but carried no usable timestamps, so the held \
             tempo could not be measured",
            held.len()
        );
    };
    let error_pct = ((held_hz - expected) / expected).abs() * 100.0;
    crate::outcome::record(format!(
        "held tempo: {held_hz:.2} Hz over {} pulses in 3s after Stop, {expected:.2} Hz expected \
         ({error_pct:.1}% off)",
        held.len()
    ));

    check!(
        error_pct < 5.0,
        "the clock kept running after the stop but at {held_hz:.2} Hz, not the song's \
         {expected:.2} Hz ({error_pct:.1}% off); the tempo was not held, it was replaced"
    );

    server.check_clean_log(&[])?;
    Ok(())
}

/// With `persist_tempo` off, the clock goes silent once the song stops.
///
/// The default must not change. Without this, a build that persisted
/// unconditionally would pass every other beat clock check while flooding a rig
/// that never asked for it.
pub async fn beat_clock_is_silent_after_stop_without_persist() -> CheckOutcome {
    let path = midi_path()?;
    let listen = path.listen.clone();
    crate::outcome::record(path.describe());

    let song = covered_song("Midi Song", "midi-song", TEMPO_BPM, 12.0);
    let project = midi_project_with(
        vec![song],
        BeatClock::persisting(crate::sabotage::pick(false, true)),
    )?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    let capture = MidiCapture::open(&listen)?;
    settle_port(&capture).await?;

    client.grpc().play(PlayRequest {}).await?;
    client.wait_until_playing(Duration::from_secs(10)).await?;
    tokio::time::sleep(Duration::from_secs(4)).await;

    // Same premise as above: silence afterwards only means anything if the
    // clock was running beforehand.
    let during = capture.clock_pulses();
    check!(
        during.len() > 24,
        "beat_clock was enabled but only {} pulses arrived during playback, so the silence \
         afterwards proves nothing.\n--- log ---\n{}",
        during.len(),
        server.log()
    );

    client.grpc().stop(StopRequest {}).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    capture.clear();
    tokio::time::sleep(Duration::from_secs(3)).await;

    let after = capture.clock_pulses();
    check!(
        after.is_empty(),
        "persist_tempo was off, but {} timing clocks were transmitted after the song stopped",
        after.len()
    );
    crate::outcome::record("clock went silent after Stop, as configured".to_string());

    server.check_clean_log(&[])?;
    Ok(())
}

/// The clock survives a song change without a silent gap, and the next song
/// retunes it.
///
/// The two checks above cover the two ends -- a song playing, and the idle after
/// it. This covers the handover between them, which is the moment persistence
/// exists for and the only one where a per-song clock and an always-on engine
/// visibly differ. It also confirms the engine is genuinely retuned rather than
/// stuck at the first song's tempo, which a check that only looked for
/// *continuity* would happily accept.
pub async fn beat_clock_bridges_the_gap_between_songs() -> CheckOutcome {
    let path = midi_path()?;
    let listen = path.listen.clone();
    crate::outcome::record(path.describe());

    let songs = vec![
        covered_song("Midi One", "midi-one", TEMPO_BPM, 12.0),
        covered_song("Midi Two", "midi-two", SECOND_TEMPO_BPM, 12.0),
    ];
    let project = midi_project_with(
        songs,
        BeatClock::persisting(crate::sabotage::pick(true, false)),
    )?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    let capture = MidiCapture::open(&listen)?;
    settle_port(&capture).await?;

    // Song one.
    client.grpc().play(PlayRequest {}).await?;
    client.wait_until_playing(Duration::from_secs(10)).await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    client.grpc().stop(StopRequest {}).await?;
    client.wait_until_stopped(Duration::from_secs(10)).await?;

    // The boundary an operator actually drives: a pause between songs, then
    // advance and start the next one. Everything between the Stop above and the
    // first pulse of song two is the gap persistence is supposed to cover.
    tokio::time::sleep(BETWEEN_SONGS_PAUSE).await;
    client.grpc().next(NextRequest {}).await?;
    client.grpc().play(PlayRequest {}).await?;
    client.wait_until_playing(Duration::from_secs(10)).await?;
    tokio::time::sleep(Duration::from_secs(3)).await;

    let messages = capture.messages();
    client.grpc().stop(StopRequest {}).await?;

    let song = client.status().await?.current_song.map(|s| s.name);
    check!(
        song.as_deref() == Some("Midi Two"),
        "expected to be on 'Midi Two' after next, but the player reports {song:?}"
    );

    let pulses: Vec<midi::TimedMessage> = messages
        .iter()
        .filter(|m| m.status() == Some(midi::TIMING_CLOCK))
        .cloned()
        .collect();
    check!(
        pulses.len() > 24,
        "only {} timing clocks arrived across the whole run.\n--- log ---\n{}",
        pulses.len(),
        server.log()
    );

    // Song two's pulses are the ones after its Start. Using the last Start
    // rather than a wall-clock split keeps the segmentation tied to what mtrack
    // actually sent.
    let last_start = messages
        .iter()
        .rfind(|m| m.status() == Some(midi::START))
        .map(|m| m.micros);
    let Some(last_start) = last_start else {
        return Err(crate::outcome::CheckError::assertion(
            "no MIDI Start (0xFA) was transmitted for the second song; the transport was never \
             re-run"
                .to_string(),
        ));
    };

    let second: Vec<midi::TimedMessage> = pulses
        .iter()
        .filter(|m| m.micros > last_start)
        .cloned()
        .collect();

    // Retuning: the second song must drive the clock at *its* tempo.
    let expected_second = expected_hz(SECOND_TEMPO_BPM);
    match hz_of(&second) {
        Some(measured) => {
            let error_pct = ((measured - expected_second) / expected_second).abs() * 100.0;
            crate::outcome::record(format!(
                "second song: {measured:.2} Hz measured, {expected_second:.2} Hz expected \
                 ({error_pct:.1}% off, {} pulses)",
                second.len()
            ));
            check!(
                error_pct < 5.0,
                "after the song change the clock ran at {measured:.2} Hz, but 'Midi Two' at \
                 {SECOND_TEMPO_BPM} BPM requires {expected_second:.2} Hz ({error_pct:.1}% off); \
                 the engine was not retuned"
            );
        }
        None => inconclusive!(
            "{} clocks arrived for the second song but carried no usable timestamps",
            second.len()
        ),
    }

    // Continuity: the reason persistence exists.
    let Some((gap, at)) = longest_clock_gap(&pulses) else {
        inconclusive!("too few clocks to measure a gap");
    };
    crate::outcome::record(format!(
        "longest silence in the clock stream: {:.0} ms, beginning {:.1}s into the run",
        gap.as_secs_f64() * 1000.0,
        at.as_secs_f64()
    ));

    check!(
        gap <= MAX_BOUNDARY_SILENCE,
        "persist_tempo was on, but the clock went silent for {:.0} ms across the song change \
         (budget {:.0} ms). Downstream gear does not distinguish 'between songs' from 'the clock \
         stopped'; a gap this long is the drift persistence is meant to prevent.\n--- log ---\n{}",
        gap.as_secs_f64() * 1000.0,
        MAX_BOUNDARY_SILENCE.as_secs_f64() * 1000.0,
        server.log()
    );

    server.check_clean_log(&[])?;
    Ok(())
}

/// The MIDI device the operator actually configured opens and transmits.
///
/// The byte-level checks above verify mtrack's MIDI *generation*, but they do
/// it over whichever port loops back -- typically a software one. That leaves
/// the real interface completely unexercised. This check cannot verify the
/// bytes (there is nothing to listen on), but it does prove the configured
/// port opens, is claimed, and carries a song's playback without error.
pub async fn configured_midi_device_transmits() -> CheckOutcome {
    crate::runner::require_area("midi-config")?;

    let caps = Capabilities::get();
    let Some(device) = caps.midi_out.clone() else {
        skip!("no MIDI device is configured");
    };

    // Deliberately no loopback override: this is the one check that uses the
    // hardware exactly as configured.
    let song = SongSpec::tones("Midi Hw", "midi-hw", 1, 6.0)
        .with_midi(MidiSpec::scale("song.mid", TEMPO_BPM, 4));
    let project = ProjectBuilder::new()
        .profiles(vec![ProfileSpec::detected("01-e2e")])
        .songs(vec![song])
        .build()?;

    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    // Compare against a name the profile never used. Pointing the profile at a
    // bogus device instead made startup time out, so the control died before
    // this assertion ever ran.
    let status = crate::sabotage::pick(
        client.subsystem_status("midi").await?,
        "not_connected".to_string(),
    );
    check!(
        status == "connected",
        "the configured MIDI device '{}' was not claimed (reported '{status}').\n--- log ---\n{}",
        device.name,
        server.log()
    );

    client.grpc().play(PlayRequest {}).await?;
    client.wait_until_playing(Duration::from_secs(10)).await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    client.grpc().stop(StopRequest {}).await?;

    server.check_clean_log(&[])?;
    crate::outcome::record(format!(
        "transmitted a MIDI song on {} without error",
        device.name
    ));
    // Whether the bytes were checkable here depends on the rig: state which,
    // rather than asserting a limitation that may not apply to this machine.
    let verified_elsewhere = Discovery::get()
        .midi_pair()
        .is_some_and(|pair| pair.out_port == device.name);
    crate::outcome::record(if verified_elsewhere {
        "this port also loops back, so its bytes are verified by the midi-transmit checks"
            .to_string()
    } else {
        "caveat: bytes were not verified on this port -- it has no loopback, so only the fact \
         that it opened and transmitted is proven"
            .to_string()
    });
    Ok(())
}
