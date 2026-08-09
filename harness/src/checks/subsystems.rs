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
//! Turning hardware subsystems on and off, and misconfiguring them.
//!
//! mtrack's profile semantics are that a subsystem present in a profile is
//! required for that host, and one absent is skipped. Both directions matter:
//! a missing subsystem must not stop the player, and a broken one must degrade
//! rather than take the process down.

use std::time::Duration;

use crate::capabilities::Capabilities;
use crate::client::Client;
use crate::outcome::CheckOutcome;
use crate::project::{ProfileSpec, ProjectBuilder, Subsystem};
use crate::server::Server;
use crate::{check, check_eq};

/// How long to watch a deliberately-broken subsystem before concluding it
/// degraded rather than misbehaved.
///
/// `start_degraded` returns on the first HTTP 200, which mtrack serves while
/// hardware initialization is still running. Reading the status at that instant
/// always yields "initializing", so an assertion made there passes against any
/// build -- including one that later reports the bogus device as connected.
const SETTLE: Duration = Duration::from_secs(6);

/// Watches a subsystem for [`SETTLE`], returning every distinct status seen.
///
/// A device that is present in the profile but unresolvable is one mtrack
/// retries forever, so "initializing" for the whole window is the expected
/// outcome -- what matters is that it never claims to be connected, and that
/// nothing panics while it keeps trying.
async fn watch_subsystem(
    client: &crate::client::Client,
    subsystem: &str,
) -> Result<Vec<String>, crate::outcome::CheckError> {
    let deadline = std::time::Instant::now() + SETTLE;
    let mut seen: Vec<String> = Vec::new();
    while std::time::Instant::now() < deadline {
        let status = client.subsystem_status(subsystem).await?;
        if !seen.contains(&status) {
            seen.push(status.clone());
        }
        // Deliberately no early break. Stopping as soon as the status settled
        // would read the log a few hundred milliseconds in, reintroducing the
        // "missed by milliseconds" gap the settle window exists to close.
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(seen)
}

/// A profile with no `midi:` block leaves the player running without MIDI.
pub async fn absent_midi_is_skipped_not_fatal() -> CheckOutcome {
    crate::runner::require_area("subsystems")?;
    let mut profile = ProfileSpec::detected("01-e2e");
    profile.midi = Subsystem::Absent;

    let project = ProjectBuilder::new()
        .profiles(vec![profile])
        .songs(crate::checks::standard_songs())
        .build()?;
    let server = Server::start(&project).await?;
    let client = Client::connect(&server).await?;

    check_eq!(
        client.subsystem_status("midi").await?,
        "not_connected",
        "a profile without a midi block should report MIDI as not_connected"
    );
    check_eq!(
        client.subsystem_status("audio").await?,
        "connected",
        "omitting MIDI should not affect audio"
    );

    server.check_config_understood()?;
    server.check_clean_log(&[])?;
    Ok(())
}

/// A profile with no `dmx:` block leaves the player running without DMX.
pub async fn absent_dmx_is_skipped_not_fatal() -> CheckOutcome {
    crate::runner::require_area("subsystems")?;
    let mut profile = ProfileSpec::detected("01-e2e");
    profile.dmx = Subsystem::Absent;

    let project = ProjectBuilder::new()
        .profiles(vec![profile])
        .songs(crate::checks::standard_songs())
        .build()?;
    let server = Server::start(&project).await?;
    let client = Client::connect(&server).await?;

    check_eq!(
        client.subsystem_status("dmx").await?,
        "not_connected",
        "a profile without a dmx block should report DMX as not_connected"
    );
    check_eq!(
        client.subsystem_status("audio").await?,
        "connected",
        "omitting DMX should not affect audio"
    );

    server.check_config_understood()?;
    server.check_clean_log(&[])?;
    Ok(())
}

/// A MIDI device that does not exist degrades instead of killing the player.
///
/// This is the closest the suite gets to unplugging hardware without a human:
/// the device name is real-looking but resolves to nothing.
pub async fn bogus_midi_device_degrades_gracefully() -> CheckOutcome {
    crate::runner::require_area("subsystems")?;
    let mut profile = ProfileSpec::detected("01-e2e");
    profile.midi = Subsystem::Bogus("e2e-nonexistent-midi-device".to_string());

    let project = ProjectBuilder::new()
        .profiles(vec![profile])
        .songs(crate::checks::standard_songs())
        .build()?;

    // A present-but-unresolvable subsystem is one the player waits on, so this
    // must not require full initialization to come up.
    let server = Server::start_degraded(&project).await?;
    let client = Client::connect_http_only(&server).await?;

    let seen = watch_subsystem(&client, "midi").await?;
    check!(
        !seen.iter().any(|s| s == "connected"),
        "a nonexistent MIDI device was reported as connected (statuses seen: {seen:?})"
    );

    // Read after the settle window, so a panic during device resolution is
    // caught rather than missed by looking a few milliseconds after startup.
    let log = server.log();
    check!(
        !log.contains("panicked at"),
        "a nonexistent MIDI device panicked the player.\n--- log ---\n{log}"
    );
    crate::outcome::record(format!(
        "bogus MIDI device: statuses over {}s were {seen:?}, never connected, no panic",
        SETTLE.as_secs()
    ));

    Ok(())
}

/// An audio device that does not exist degrades instead of killing the player.
pub async fn bogus_audio_device_degrades_gracefully() -> CheckOutcome {
    crate::runner::require_area("subsystems")?;
    let mut profile = ProfileSpec::detected("01-e2e");
    profile.audio = Subsystem::Bogus("e2e-nonexistent-audio-device".to_string());

    let project = ProjectBuilder::new()
        .profiles(vec![profile])
        .songs(crate::checks::standard_songs())
        .build()?;
    let server = Server::start_degraded(&project).await?;
    let client = Client::connect_http_only(&server).await?;

    let seen = watch_subsystem(&client, "audio").await?;
    check!(
        !seen.iter().any(|s| s == "connected"),
        "a nonexistent audio device was reported as connected (statuses seen: {seen:?})"
    );

    // Read after the settle window, so a panic during device resolution is
    // caught rather than missed by looking a few milliseconds after startup.
    let log = server.log();
    check!(
        !log.contains("panicked at"),
        "a nonexistent audio device panicked the player.\n--- log ---\n{log}"
    );
    crate::outcome::record(format!(
        "bogus audio device: statuses over {}s were {seen:?}, never connected, no panic",
        SETTLE.as_secs()
    ));

    Ok(())
}

/// Only the first matching profile is active, and it is the one applied.
///
/// Profiles are sorted by filename, so a second profile pointing at a
/// different device must not be the one that gets claimed.
pub async fn first_profile_wins() -> CheckOutcome {
    crate::runner::require_area("subsystems")?;
    let caps = Capabilities::get();
    let Some(expected) = caps.audio_out.as_ref().map(|d| d.name.clone()) else {
        return Ok(());
    };

    let mut second = ProfileSpec::detected("02-decoy");
    second.audio = Subsystem::Bogus("e2e-decoy-device".to_string());

    let project = ProjectBuilder::new()
        .profiles(vec![ProfileSpec::detected("01-e2e"), second])
        .songs(crate::checks::standard_songs())
        .build()?;
    let server = Server::start(&project).await?;
    let client = Client::connect(&server).await?;

    let claimed = client.subsystem_device("audio").await?.unwrap_or_default();
    check!(
        claimed.starts_with(&expected),
        "expected the first profile's device '{expected}' to be claimed, got '{claimed}'"
    );

    server.check_clean_log(&[])?;
    Ok(())
}

/// Controllers can be restarted while the player is idle.
pub async fn controllers_restart_while_idle() -> CheckOutcome {
    crate::runner::require_area("subsystems")?;
    let project = crate::checks::standard_project()?;
    let server = Server::start(&project).await?;
    let client = Client::connect(&server).await?;

    let (status, body) = client
        .send_text(reqwest::Method::POST, "controllers/restart", String::new())
        .await?;
    check!(
        status.is_success(),
        "restarting controllers failed: HTTP {status}\n{body}"
    );

    // The gRPC controller must come back, or the player has lost its control
    // surface without saying so.
    let mut reconnected = Client::connect(&server).await?;
    reconnected.status().await?;

    server.check_clean_log(&[])?;
    Ok(())
}
