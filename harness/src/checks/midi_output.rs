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

use mtrack::proto::player::v1::{PlayRequest, StopRequest};

use crate::capabilities::Capabilities;
use crate::client::Client;
use crate::discovery::Discovery;
use crate::midi::{self, MidiCapture};
use crate::outcome::CheckOutcome;
use crate::project::{ProfileSpec, ProjectBuilder, Subsystem};
use crate::server::Server;
use crate::songs::{MidiSpec, SongSpec};
use crate::{check, check_eq, skip};

/// Tempo of the generated MIDI song. Chosen away from round numbers so a clock
/// derived from a default rather than from the file is obvious.
const TEMPO_BPM: f32 = 104.0;

/// Builds a project whose song carries a MIDI file, optionally with beat clock.
fn midi_project(beat_clock: bool) -> Result<crate::project::Project, Box<dyn std::error::Error>> {
    let song = SongSpec::tones("Midi Song", "midi-song", 1, 10.0)
        .with_midi(MidiSpec::scale("song.mid", TEMPO_BPM, 8));

    let mut profile = ProfileSpec::detected("01-e2e");
    if let Some(pair) = Discovery::get().midi_pair() {
        profile.midi = Subsystem::Named(pair.out_port.clone());
    }
    if beat_clock {
        profile = profile.with_midi_key("beat_clock", "true");
    }

    ProjectBuilder::new()
        .profiles(vec![profile])
        .songs(vec![song])
        .build()
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
    let mut evidence: Vec<String> = Vec::new();
    let path = midi_path()?;
    let listen = path.listen.clone();
    evidence.push(path.describe());

    let project = midi_project(false)?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    let capture = MidiCapture::open(&listen)?;
    capture.clear();

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
    let expected: Vec<u8> = (0..expected_notes).map(|b| 60 + (b as u8 % 12)).collect();
    check_eq!(
        received,
        expected,
        "the transmitted notes do not match the song's MIDI file"
    );
    evidence.push(format!("transmitted notes: {received:?}"));

    server.check_clean_log(&[])?;
    Ok(evidence)
}

/// With beat clock enabled, mtrack emits 24 pulses per quarter note at the
/// song's tempo.
///
/// The rate is what makes this meaningful: any implementation can emit 0xF8
/// bytes, but only a correct one emits them at `bpm * 24 / 60` Hz.
pub async fn beat_clock_runs_at_the_song_tempo() -> CheckOutcome {
    let mut evidence: Vec<String> = Vec::new();
    let path = midi_path()?;
    let listen = path.listen.clone();
    evidence.push(path.describe());

    let project = midi_project(true)?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    let capture = MidiCapture::open(&listen)?;
    capture.clear();

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

    let expected = MidiSpec::scale("song.mid", TEMPO_BPM, 8).expected_clock_hz();
    let measured = measured.expect("pulse count checked above");
    let error_pct = ((measured - expected) / expected).abs() * 100.0;

    evidence.push(format!(
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
    Ok(evidence)
}

/// With beat clock disabled, no timing clock is transmitted.
///
/// Without this, the rate check above would still pass against a build that
/// ignores the setting and always emits a clock.
pub async fn beat_clock_is_silent_when_disabled() -> CheckOutcome {
    let mut evidence: Vec<String> = Vec::new();
    let path = midi_path()?;
    let listen = path.listen.clone();
    evidence.push(path.describe());

    let project = midi_project(false)?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    let capture = MidiCapture::open(&listen)?;
    capture.clear();

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
    Ok(evidence)
}

/// The MIDI device the operator actually configured opens and transmits.
///
/// The byte-level checks above verify mtrack's MIDI *generation*, but they do
/// it over whichever port loops back -- typically a software one. That leaves
/// the real interface completely unexercised. This check cannot verify the
/// bytes (there is nothing to listen on), but it does prove the configured
/// port opens, is claimed, and carries a song's playback without error.
pub async fn configured_midi_device_transmits() -> CheckOutcome {
    let mut evidence: Vec<String> = Vec::new();
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

    let status = client.subsystem_status("midi").await?;
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
    evidence.push(format!(
        "transmitted a MIDI song on {} without error",
        device.name
    ));
    // Whether the bytes were checkable here depends on the rig: state which,
    // rather than asserting a limitation that may not apply to this machine.
    let verified_elsewhere = Discovery::get()
        .midi_pair()
        .is_some_and(|pair| pair.out_port == device.name);
    evidence.push(if verified_elsewhere {
        "this port also loops back, so its bytes are verified by the midi-transmit checks"
            .to_string()
    } else {
        "caveat: bytes were not verified on this port -- it has no loopback, so only the fact \
         that it opened and transmitted is proven"
            .to_string()
    });
    Ok(evidence)
}
