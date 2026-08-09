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
//! Hardware probing and cold start.

use crate::capabilities::Capabilities;
use crate::client::Client;
use crate::outcome::CheckOutcome;
use crate::server::Server;
use crate::{check, check_eq, fail};

/// Starts the player against a generated project and confirms it reaches a
/// ready state with the audio device actually claimed.
pub async fn player_starts_against_detected_hardware() -> CheckOutcome {
    crate::runner::require_area("startup")?;
    let caps = Capabilities::get();
    let project = crate::checks::standard_project()?;
    let server = Server::start(&project).await?;
    let client = Client::connect(&server).await?;

    check_eq!(
        client.subsystem_status("audio").await?,
        "connected",
        "audio device was not claimed.\n--- log ---\n{}",
        server.log()
    );

    // Proves the generated profile is the one that was applied. The status
    // API's `profile` field is the profile's hostname (or "default"), so the
    // claimed device is the reliable signal. It is reported in display form
    // ("name (Channels=N) (Host)"), so the config name is a prefix of it.
    let configured = caps
        .audio_out
        .as_ref()
        .map(|d| d.name.clone())
        .unwrap_or_default();
    let claimed = client.subsystem_device("audio").await?.unwrap_or_default();
    check!(
        claimed.starts_with(&configured),
        "the player claimed '{claimed}' but the generated profile named '{configured}'"
    );

    // MIDI and DMX are reported against what the probe found, so a machine
    // without them still passes -- but the gap is recorded.
    let midi = client.subsystem_status("midi").await?;
    match (&caps.midi_out, midi.as_str()) {
        (Some(device), "connected") => {
            crate::outcome::record(format!("midi connected: {}", device.name));
        }
        (Some(device), other) => fail!(
            "MIDI device {} was detected but the player reports '{other}'.\n--- log ---\n{}",
            device.name,
            server.log()
        ),
        // No MIDI is a normal configuration, not a defect, so the check still
        // passes on its own terms and the gap is noted as evidence.
        (None, _) => crate::outcome::record("no MIDI device present to claim".to_string()),
    }

    let dmx = client.subsystem_status("dmx").await?;
    if caps.ola_port.is_some() && dmx != "connected" {
        fail!(
            "olad is running but the DMX subsystem reports '{dmx}'.\n--- log ---\n{}",
            server.log()
        );
    }

    server.check_config_understood()?;
    server.check_clean_log(&[])?;
    Ok(())
}

/// Confirms the generated project is one mtrack itself considers valid, so a
/// later failure points at the player rather than at the harness.
pub async fn generated_project_loads_all_songs() -> CheckOutcome {
    let project = crate::checks::standard_project()?;
    let server = Server::start(&project).await?;
    let client = Client::connect(&server).await?;

    let songs = client.get_json("songs").await?;
    let listed = songs
        .as_array()
        .map(|a| a.len())
        .or_else(|| {
            songs
                .get("songs")
                .and_then(|s| s.as_array())
                .map(|a| a.len())
        })
        .unwrap_or(0);

    check_eq!(
        listed,
        project.songs().len(),
        "expected {} generated songs to load, got {listed}: {songs}",
        project.songs().len()
    );

    server.check_clean_log(&[])?;
    Ok(())
}
