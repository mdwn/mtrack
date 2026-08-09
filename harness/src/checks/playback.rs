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
//! Playback against real hardware, and proof that audio reached the physical
//! channels it was mapped to.

use std::collections::BTreeMap;
use std::time::Duration;

use mtrack::proto::player::v1::{NextRequest, PlayRequest, PreviousRequest, StopRequest};

use crate::capabilities::Capabilities;
use crate::capture::{self, Capture};
use crate::client::{elapsed_of, Client};
use crate::project::{ProfileSpec, ProjectBuilder};
use crate::server::Server;
use crate::songs::SongSpec;
use crate::outcome::CheckOutcome;
use crate::{check, check_eq, fail, inconclusive, skip};

/// Playing a song advances the clock and then stops on its own.
///
/// The elapsed check is monotonic rather than a single reading: a clock that
/// is stuck or that jumps backwards is a real defect that a "did it finish?"
/// assertion alone would miss.
pub async fn plays_a_song_to_completion() -> CheckOutcome {
    let mut evidence: Vec<String> = Vec::new();
    crate::runner::require_area("playback")?;
    let project = ProjectBuilder::new()
        .songs(vec![SongSpec::tones("Short Tone", "short-tone", 2, 4.0)])
        .build()?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    client.grpc().play(PlayRequest {}).await?;
    client.wait_until_playing(Duration::from_secs(10)).await?;

    let mut readings = Vec::new();
    for _ in 0..6 {
        tokio::time::sleep(Duration::from_millis(400)).await;
        let status = client.status().await?;
        if !status.playing {
            break;
        }
        readings.push(elapsed_of(&status));
    }

    check!(
        readings.len() >= 3,
        "playback ended too quickly to observe the clock: {readings:?}"
    );
    for pair in readings.windows(2) {
        check!(
            pair[1] >= pair[0],
            "elapsed went backwards: {:?} then {:?}",
            pair[0],
            pair[1]
        );
    }
    check!(
        readings.last() > readings.first(),
        "elapsed never advanced: {readings:?}"
    );

    client.wait_until_stopped(Duration::from_secs(15)).await?;
    server.check_clean_log(&[])?;
    Ok(evidence)
}

/// Stop takes effect while a song is mid-flight.
pub async fn stop_halts_playback() -> CheckOutcome {
    let mut evidence: Vec<String> = Vec::new();
    crate::runner::require_area("playback")?;
    let project = crate::checks::standard_project()?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    client.grpc().play(PlayRequest {}).await?;
    client.wait_until_playing(Duration::from_secs(10)).await?;
    client.grpc().stop(StopRequest {}).await?;
    client.wait_until_stopped(Duration::from_secs(10)).await?;

    server.check_clean_log(&[])?;
    Ok(evidence)
}

/// Playlist navigation moves between songs without playback running.
pub async fn playlist_navigation_moves_between_songs() -> CheckOutcome {
    let mut evidence: Vec<String> = Vec::new();
    crate::runner::require_area("playback")?;
    let project = crate::checks::standard_project()?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    let first = client.status().await?.current_song.map(|s| s.name);
    client.grpc().next(NextRequest {}).await?;
    let second = client.status().await?.current_song.map(|s| s.name);
    assert_ne!(first, second, "next did not change the current song");

    client.grpc().previous(PreviousRequest {}).await?;
    let back = client.status().await?.current_song.map(|s| s.name);
    check_eq!(back, first, "previous did not return to the first song");

    server.check_clean_log(&[])?;
    Ok(evidence)
}

/// Each track reaches the physical output channel it was mapped to, and does
/// not reach any other.
///
/// This is the assertion the whole loopback tier exists for. "Channel produced
/// sound" would pass even if every track were summed onto every output; the
/// absence half is what actually catches misrouting.
pub async fn tracks_route_to_their_mapped_channels() -> CheckOutcome {
    let mut evidence: Vec<String> = Vec::new();
    crate::runner::require_area("audio-routing")?;
    let caps = Capabilities::get();

    // Cabling was measured once, before any server claimed the devices.
    let device = caps.audio_out.as_ref().expect("audio-routing requires an output").name.clone();
    let links = crate::discovery::Discovery::get().audio_for_device(&device);
    if links.is_empty() {
        skip!("no output channel of {device} is patched back to an input");
    }

    let outputs: Vec<u16> = links.iter().map(|l| l.out_channel).collect();
    evidence.push(format!("loopback: {}", links
            .iter()
            .map(|l| format!("out{}->in{}", l.out_channel, l.in_channel))
            .collect::<Vec<_>>()
            .join(", ")));

    // One tone track per verifiable output, mapped one-to-one so a tone's
    // identity determines which channel it should have come out of.
    let song = SongSpec::tones("Routing", "routing", outputs.len(), 12.0);
    let mut mappings = BTreeMap::new();
    for (track, output) in song.tracks.iter().zip(outputs.iter()) {
        mappings.insert(track.name.clone(), vec![*output]);
    }

    let mut profile = ProfileSpec::detected("01-e2e");
    profile.track_mappings = mappings;

    let project = ProjectBuilder::new()
        .profiles(vec![profile])
        .songs(vec![song.clone()])
        .build()?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    client.grpc().play(PlayRequest {}).await?;
    client.wait_until_playing(Duration::from_secs(10)).await?;
    // Let the stream reach steady state before measuring.
    tokio::time::sleep(Duration::from_millis(800)).await;

    let recorded = tokio::task::spawn_blocking(|| {
        Capture::record(Duration::from_secs(3)).map_err(|e| e.to_string())
    })
    .await??;
    client.grpc().stop(StopRequest {}).await?;

    // Playback and capture share one USB device here, and without real-time
    // scheduling ALSA drops periods under load. A short capture means the
    // measurement is inconclusive, which must not be reported as a routing
    // failure -- that would blame the player for the harness's xruns.
    let shortest = (1..=recorded.channel_count() as u16)
        .map(|ch| recorded.channel(ch).len())
        .min()
        .unwrap_or(0);
    let expected_samples = recorded.sample_rate() as usize; // at least one second
    if shortest < expected_samples {
        skip!(
                "capture returned only {shortest} samples per channel (wanted at least \
                 {expected_samples}); the interface underran, so routing was not verified");
    }

    let expected_tones: Vec<f32> = song.tracks.iter().filter_map(|t| t.tone_hz).collect();
    let mut failures = Vec::new();

    for (index, link) in links.iter().enumerate() {
        let output = &link.out_channel;
        let input = link.in_channel;
        let window = recorded.steady(
            input,
            Duration::from_millis(500),
            Duration::from_millis(1500),
        );
        let present = capture::tones_present(window, recorded.sample_rate(), &expected_tones);
        let found: Vec<usize> = present.iter().map(|(i, _)| *i).collect();

        if !found.contains(&index) {
            failures.push(format!(
                "track '{}' ({} Hz) was mapped to output {output} but no such tone reached input \
                 {input} (peak {:.4}, tones seen: {:?})",
                song.tracks[index].name, expected_tones[index], recorded.peak(input), found
            ));
        }

        for other in found.iter().filter(|f| **f != index) {
            failures.push(format!(
                "input {input} (fed by output {output}) also carried {} Hz, which belongs to \
                 track '{}' mapped to a different output -- tracks are bleeding across channels",
                expected_tones[*other], song.tracks[*other].name
            ));
        }
    }

    check!(
        failures.is_empty(),
        "channel routing did not match the configured track mappings:\n  {}\n--- log ---\n{}",
        failures.join("\n  "),
        server.log()
    );

    let total = caps.audio_out.as_ref().map(|d| d.max_channels).unwrap_or(0) as usize;
    if outputs.len() < total {
        evidence.push(format!(
            "caveat: only outputs {outputs:?} of {total} are patched back, so the rest were \
             exercised but not acoustically verified"
        ));
    }

    server.check_clean_log(&[])?;
    Ok(evidence)
}
