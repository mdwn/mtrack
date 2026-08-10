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
//! Configuration changes survive the round trip to disk and back.
//!
//! A setting can look correct in three different places and still be broken:
//! applied in memory, written to the wrong file, or written correctly but not
//! reloaded. [`round_trip`] checks all of them, so covering a new config knob
//! costs one call rather than one hand-written test.

use std::time::Duration;

use mtrack::proto::player::v1::{GetConfigRequest, UpdateMidiRequest};

use crate::capabilities::Capabilities;
use crate::client::Client;
use crate::outcome::{CheckError, CheckOutcome};
use crate::project::Project;
use crate::server::Server;
use crate::{check, check_eq, fail, inconclusive, skip};

/// Asserts that a config mutation reaches memory, disk, and a restarted process.
///
/// `expect` is matched against the served YAML and against the owning profile
/// file. `profile` names the profile file the change should land in.
async fn round_trip<F, Fut>(
    project: &Project,
    server: Server,
    mut client: Client,
    what: &str,
    profile: &str,
    expect: &str,
    mutate: F,
) -> CheckOutcome
where
    F: FnOnce(Client, String) -> Fut,
    Fut: std::future::Future<Output = Result<Client, CheckError>>,
{
    let before = client
        .grpc()
        .get_config(GetConfigRequest {})
        .await?
        .into_inner();
    if before.yaml.contains(expect) {
        // The harness could not establish its own starting conditions; that is
        // not a finding about mtrack, and reporting it as one would exit
        // non-zero claiming a defect.
        inconclusive!(
            "{what}: '{expect}' was already present before the mutation, so this check could \
             not prove anything"
        );
    }

    // The store uses optimistic concurrency, so the mutation carries the
    // checksum it read.
    let mut client = mutate(client, before.checksum).await?;

    // Each stage is recorded rather than asserted immediately. A setting can
    // fail at several stages at once, and seeing all of them together is what
    // distinguishes "written to the wrong file" from "written nowhere" from
    // "written somewhere the loader ignores".
    let mut problems = Vec::new();

    let after = client
        .grpc()
        .get_config(GetConfigRequest {})
        .await?
        .into_inner();
    if !after.yaml.contains(expect) {
        problems.push(format!(
            "the running config does not contain '{expect}' after the update"
        ));
    }

    match project.read_profile(profile) {
        Ok(on_disk) if on_disk.contains(expect) => {}
        Ok(_) => problems.push(format!(
            "'{expect}' did not reach {profile}.yaml, the profile file that owns this setting"
        )),
        Err(e) => problems.push(format!("could not read {profile}.yaml: {e}")),
    }

    // The real test of persistence: a cold process reading the file back.
    drop(server);
    let server = Server::start(project).await?;
    let mut client = Client::connect(&server).await?;

    let reloaded = client
        .grpc()
        .get_config(GetConfigRequest {})
        .await?
        .into_inner();
    if !reloaded.yaml.contains(expect) {
        problems.push(format!("'{expect}' did not survive a restart"));
    }

    // Landing in a spot the loader discards is worse than not persisting at
    // all: the value is visible in the served config yet has no effect.
    let ignored: Vec<String> = server
        .log()
        .lines()
        .filter(|l| l.contains("ignored when 'profiles' is present"))
        .map(|l| l.trim().to_string())
        .collect();
    if !ignored.is_empty() {
        problems.push(format!(
            "the reloaded config discarded the written value:\n      {}",
            ignored.join("\n      ")
        ));
    }

    if !problems.is_empty() {
        fail!(
            "{what} did not survive the round trip to disk:\n  - {}\n\nProject files follow:{}",
            problems.join("\n  - "),
            project.dump_yaml()
        );
    }
    crate::outcome::record(format!(
        "{what} reached the profile file and survived a restart"
    ));

    server.check_clean_log(&[])?;
    Ok(())
}

/// MIDI beat clock survives being enabled, written, and reloaded.
pub async fn midi_beat_clock_persists() -> CheckOutcome {
    let _evidence: Vec<String> = Vec::new();
    crate::runner::require_area("midi-config")?;
    let caps = Capabilities::get();
    let Some(midi) = caps.midi_out.clone() else {
        skip!("no MIDI device, so beat clock configuration cannot be exercised",);
    };

    let project = crate::checks::standard_project()?;
    let server = Server::start(&project).await?;
    let client = Client::connect(&server).await?;

    let device = midi.name.clone();
    round_trip(
        &project,
        server,
        client,
        "midi beat_clock",
        "01-e2e",
        crate::sabotage::pick("beat_clock: true", "beat_clock: false"),
        move |mut client, checksum| async move {
            let midi_json = serde_json::json!({
                "device": device,
                "beat_clock": true,
            })
            .to_string();
            client
                .grpc()
                .update_midi(UpdateMidiRequest {
                    midi_json,
                    expected_checksum: checksum,
                })
                .await?;
            Ok::<_, CheckError>(client)
        },
    )
    .await
}

/// A stale checksum is rejected rather than silently overwriting.
///
/// Optimistic concurrency is what stops the web UI and a controller from
/// clobbering each other, so it is worth proving against the real store.
pub async fn stale_checksum_is_rejected() -> CheckOutcome {
    crate::runner::require_area("midi-config")?;
    let caps = Capabilities::get();
    let Some(midi) = caps.midi_out.clone() else {
        skip!("no MIDI device, so config concurrency cannot be exercised",);
    };

    let project = crate::checks::standard_project()?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    let first = client
        .grpc()
        .get_config(GetConfigRequest {})
        .await?
        .into_inner();

    // Land one accepted update so the stored checksum moves on.
    client
        .grpc()
        .update_midi(UpdateMidiRequest {
            midi_json: serde_json::json!({"device": midi.name, "beat_clock": true}).to_string(),
            expected_checksum: first.checksum.clone(),
        })
        .await?;

    // Under sabotage, use a *current* checksum, which the store should accept --
    // proving the rejection above is really about staleness.
    // Fetched only when it will be used: an extra RPC on every normal run to
    // serve the sabotage path is waste.
    let second_checksum = if crate::sabotage::active() {
        client
            .grpc()
            .get_config(GetConfigRequest {})
            .await?
            .into_inner()
            .checksum
    } else {
        String::new()
    };

    // Replaying the original checksum must now fail.
    let stale = client
        .grpc()
        .update_midi(UpdateMidiRequest {
            midi_json: serde_json::json!({"device": midi.name, "beat_clock": false}).to_string(),
            expected_checksum: crate::sabotage::pick(first.checksum.clone(), second_checksum),
        })
        .await;

    check!(
        stale.is_err(),
        "a stale checksum was accepted; concurrent editors can silently overwrite each other"
    );

    let current = client
        .grpc()
        .get_config(GetConfigRequest {})
        .await?
        .into_inner();
    check!(
        current.yaml.contains("beat_clock: true"),
        "the rejected update still changed the config:\n{}",
        current.yaml
    );

    server.check_clean_log(&["invalid", "checksum", "conflict"])?;
    Ok(())
}

/// The active playlist selection survives a restart.
///
/// Unlike the MIDI settings this lives in `mtrack.yaml` rather than a profile,
/// so it exercises the other persistence path.
///
/// Switches to a second generated playlist rather than `all_songs`, which the
/// player deliberately treats as session-only (`player/navigation.rs`). An
/// earlier version of this check used `all_songs` and so asserted the opposite
/// of the documented behaviour -- it failed on every rig, and the "defect" it
/// reported was not one.
pub async fn active_playlist_persists_across_restart() -> CheckOutcome {
    crate::runner::require_area("persistence")?;
    use mtrack::proto::player::v1::SwitchToPlaylistRequest;

    let project = crate::checks::standard_project()?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    let original = client.status().await?.playlist_name;
    client
        .grpc()
        .switch_to_playlist(SwitchToPlaylistRequest {
            playlist_name: crate::sabotage::pick(
                crate::project::ProjectBuilder::ALTERNATE_PLAYLIST,
                "playlist",
            )
            .to_string(),
        })
        .await?;

    let switched = client.status().await?.playlist_name;
    check!(
        original != switched,
        "switching to the alternate playlist did not change the reported selection \
         (still {original:?})"
    );

    drop(server);
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;
    // Give the reloaded player a moment to settle on its restored selection.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let after_restart = client.status().await?.playlist_name;
    check_eq!(
        after_restart,
        switched,
        "the active playlist did not survive a restart, though `active_playlist` in \
         config/player.rs is documented as persisted across restarts.\n\
         Project files follow:{}",
        project.dump_yaml()
    );

    server.check_clean_log(&[])?;
    Ok(())
}
