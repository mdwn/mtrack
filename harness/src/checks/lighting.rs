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
//! Lighting show creation, validation, and playback.
//!
//! Where OLA is absent the DMX engine runs against its null client, so the
//! timeline, cue list, and effect state are all still exercised; only the
//! frames on the wire go unverified. That gap is recorded rather than assumed
//! away.

use std::time::Duration;

use mtrack::proto::player::v1::{
    GetActiveEffectsRequest, GetCuesRequest, PlayRequest, StopRequest,
};

use crate::capabilities::Capabilities;
use crate::client::Client;
use crate::outcome::CheckOutcome;
use crate::project::{ProfileSpec, ProjectBuilder, LIGHTING_GROUP};
use crate::server::Server;
use crate::songs::{LightingSpec, SongSpec};
use crate::{check, check_eq};

/// A project whose profile has lighting wired up and whose first song has a show.
fn lighting_project() -> Result<crate::project::Project, Box<dyn std::error::Error>> {
    let song = SongSpec::tones("Lit Song", "lit-song", 2, 8.0).with_lighting(LightingSpec::simple(
        "E2E Show",
        "lighting/show.light",
        LIGHTING_GROUP,
    ));

    ProjectBuilder::new()
        .profiles(vec![ProfileSpec::detected("01-e2e").with_lighting()])
        .songs(vec![song])
        .build()
}

/// The generated show is one mtrack's own validator accepts.
///
/// Run before the playback cases so a DSL mistake in the harness is reported
/// as a validation failure rather than as an empty cue list later on.
pub async fn generated_show_passes_validation() -> CheckOutcome {
    let evidence: Vec<String> = Vec::new();
    crate::runner::require_area("lighting")?;
    let project = lighting_project()?;
    let server = Server::start(&project).await?;
    let client = Client::connect(&server).await?;

    let show = LightingSpec::simple("Validation", "lighting/v.light", LIGHTING_GROUP);
    let (status, body) = client
        .send_text(
            reqwest::Method::POST,
            "lighting/validate",
            show.source.clone(),
        )
        .await?;

    check!(
        status.is_success(),
        "the generated light show did not validate: HTTP {status}\n{body}\n--- show ---\n{}",
        show.source
    );

    server.check_clean_log(&[])?;
    Ok(evidence)
}

/// A malformed show is rejected rather than silently accepted.
///
/// Without this, the validation case above would pass against a validator that
/// approves everything.
pub async fn malformed_show_is_rejected() -> CheckOutcome {
    let evidence: Vec<String> = Vec::new();
    crate::runner::require_area("lighting")?;
    let project = lighting_project()?;
    let server = Server::start(&project).await?;
    let client = Client::connect(&server).await?;

    let broken = "show \"Broken\" { @00:00.000 this is not a valid effect line ".to_string();
    let (status, body) = client
        .send_text(reqwest::Method::POST, "lighting/validate", broken)
        .await?;

    check!(
        !status.is_success(),
        "a malformed light show was accepted as valid (HTTP {status}): {body}"
    );

    server.check_clean_log(&["invalid", "parse", "lighting"])?;
    Ok(evidence)
}

/// A show attached to a song turns into cues the player can report.
pub async fn song_lighting_produces_cues() -> CheckOutcome {
    let mut evidence: Vec<String> = Vec::new();
    crate::runner::require_area("lighting")?;
    let project = lighting_project()?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    check_eq!(
        client.subsystem_status("dmx").await?,
        "connected",
        "the DMX engine did not come up, so lighting cannot be exercised.\n--- log ---\n{}",
        server.log()
    );

    // The cue list comes from the DMX engine's timeline, which is populated
    // when a song's lighting is applied at playback -- not when the song is
    // merely selected.
    client.grpc().play(PlayRequest {}).await?;
    client.wait_until_playing(Duration::from_secs(10)).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let cues = client
        .grpc()
        .get_cues(GetCuesRequest {})
        .await?
        .into_inner()
        .cues;
    client.grpc().stop(StopRequest {}).await?;

    check!(
        !cues.is_empty(),
        "the song's light show produced no cues during playback.\n--- log ---\n{}",
        server.log()
    );
    evidence.push(format!("{} cue(s) from the generated show", cues.len()));

    server.check_clean_log(&[])?;
    Ok(evidence)
}

/// Effects become active while the song plays.
///
/// The show's first cue is at 00:00, so something must be active shortly after
/// playback starts and nothing should be active before it.
pub async fn lighting_effects_activate_during_playback() -> CheckOutcome {
    let mut evidence: Vec<String> = Vec::new();
    crate::runner::require_area("lighting")?;
    let caps = Capabilities::get();
    let project = lighting_project()?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    let idle = client
        .grpc()
        .get_active_effects(GetActiveEffectsRequest {})
        .await?
        .into_inner()
        .active_effects;

    client.grpc().play(PlayRequest {}).await?;
    client.wait_until_playing(Duration::from_secs(10)).await?;
    // The first cue fires at 00:00; allow the effects loop a few frames.
    tokio::time::sleep(Duration::from_millis(700)).await;

    let during = client
        .grpc()
        .get_active_effects(GetActiveEffectsRequest {})
        .await?
        .into_inner()
        .active_effects;

    client.grpc().stop(StopRequest {}).await?;

    assert_ne!(
        during.trim(),
        idle.trim(),
        "the active effects did not change once the show started.\nidle: {idle:?}\n--- log ---\n{}",
        server.log()
    );
    check!(
        !during.trim().is_empty(),
        "no lighting effects were active during playback.\n--- log ---\n{}",
        server.log()
    );
    evidence.push(format!("active during playback: {}", during.trim()));

    if caps.ola_port.is_none() {
        evidence.push(
            "caveat: effects verified through the player's own state only; without olad the \
             DMX frames on the wire are unverified"
                .to_string(),
        );
    }

    server.check_clean_log(&[])?;
    Ok(evidence)
}

/// A show written through the API can be read back.
pub async fn show_written_via_api_is_readable() -> CheckOutcome {
    let evidence: Vec<String> = Vec::new();
    crate::runner::require_area("lighting")?;
    let project = lighting_project()?;
    let server = Server::start(&project).await?;
    let client = Client::connect(&server).await?;

    let show = LightingSpec::simple("Api Show", "api_show.light", LIGHTING_GROUP);
    let (status, body) = client
        .send_text(
            reqwest::Method::PUT,
            "lighting/api_show.light",
            show.source.clone(),
        )
        .await?;
    check!(
        status.is_success(),
        "writing a light show through the API failed: HTTP {status}\n{body}"
    );

    let (status, readback) = client.get_text("lighting/api_show.light").await?;
    check!(
        status.is_success(),
        "reading the show back failed: HTTP {status}\n{readback}"
    );
    check!(
        readback.contains(LIGHTING_GROUP),
        "the show read back does not contain what was written:\n{readback}"
    );

    server.check_clean_log(&[])?;
    Ok(evidence)
}
