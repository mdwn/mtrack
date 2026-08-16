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
    GetActiveEffectsRequest, GetCuesRequest, NextRequest, PlayRequest, StopRequest,
};

use crate::capabilities::Capabilities;
use crate::client::Client;
use crate::outcome::CheckOutcome;
use crate::project::{ProfileSpec, ProjectBuilder, LIGHTING_GROUP};
use crate::server::Server;
use crate::songs::{LightingSpec, SongSpec};
use crate::{check, check_eq};

/// A project whose profile has lighting wired up and whose first song has a show.
fn lighting_project(
    strip_show_under_sabotage: bool,
) -> Result<crate::project::Project, Box<dyn std::error::Error>> {
    // The break point for the checks that read cues and live effects: a song
    // with no show at all should leave both empty. The other three lighting
    // checks pass `false`, because they have their own break points and would
    // otherwise run two sabotages at once -- a fired assertion could then not
    // be attributed to the intended one.
    let song = SongSpec::tones("Lit Song", "lit-song", 2, 8.0);
    let song = if !strip_show_under_sabotage || crate::sabotage::perform() {
        song.with_lighting(LightingSpec::simple(
            "E2E Show",
            "lighting/show.light",
            LIGHTING_GROUP,
        ))
    } else {
        song
    };

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
    crate::runner::require_area("lighting")?;
    let project = lighting_project(false)?;
    let server = Server::start(&project).await?;
    let client = Client::connect(&server).await?;

    let show = LightingSpec::simple("Validation", "lighting/v.light", LIGHTING_GROUP);
    let (status, body) = client
        .send_text(
            reqwest::Method::POST,
            "lighting/validate",
            crate::sabotage::pick(show.source.clone(), "not a show".to_string()),
        )
        .await?;

    check!(
        status.is_success(),
        "the generated light show did not validate: HTTP {status}\n{body}\n--- show ---\n{}",
        show.source
    );

    server.check_clean_log(&[])?;
    Ok(())
}

/// A malformed show is rejected rather than silently accepted.
///
/// Without this, the validation case above would pass against a validator that
/// approves everything.
pub async fn malformed_show_is_rejected() -> CheckOutcome {
    crate::runner::require_area("lighting")?;
    let project = lighting_project(false)?;
    let server = Server::start(&project).await?;
    let client = Client::connect(&server).await?;

    let broken = crate::sabotage::pick(
        "show \"Broken\" { @00:00.000 this is not a valid effect line ".to_string(),
        LightingSpec::simple("Valid", "v.light", LIGHTING_GROUP).source,
    );
    let (status, body) = client
        .send_text(reqwest::Method::POST, "lighting/validate", broken)
        .await?;

    check!(
        !status.is_success(),
        "a malformed light show was accepted as valid (HTTP {status}): {body}"
    );

    server.check_clean_log(&["invalid", "parse", "lighting"])?;
    Ok(())
}

/// A show attached to a song turns into cues the player can report.
pub async fn song_lighting_produces_cues() -> CheckOutcome {
    crate::runner::require_area("lighting")?;
    let project = lighting_project(true)?;
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
    crate::outcome::record(format!("{} cue(s) from the generated show", cues.len()));

    server.check_clean_log(&[])?;
    Ok(())
}

/// Effects become active while the song plays.
///
/// The show's first cue is at 00:00, so something must be active shortly after
/// playback starts and nothing should be active before it.
pub async fn lighting_effects_activate_during_playback() -> CheckOutcome {
    crate::runner::require_area("lighting")?;
    let caps = Capabilities::get();
    let project = lighting_project(true)?;
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

    check!(
        during.trim() != idle.trim(),
        "the active effects did not change once the show started.\nidle: {idle:?}\n--- log ---\n{}",
        server.log()
    );
    check!(
        !during.trim().is_empty(),
        "no lighting effects were active during playback.\n--- log ---\n{}",
        server.log()
    );
    crate::outcome::record(format!("active during playback: {}", during.trim()));

    // Stated either way. Silence when olad is present would read as "the wire
    // was checked", and nothing here reads DMX back.
    crate::outcome::record(match caps.ola_port {
        Some(port) => format!(
            "caveat: frames were handed to olad on port {port}; nothing reads DMX back, so what \
             reached the fixtures is unverified"
        ),
        None => "caveat: no olad, so the DMX engine ran against its null client; effects were \
                 verified through the player's own state only"
            .to_string(),
    });

    server.check_clean_log(&[])?;
    Ok(())
}

/// A show written through the API can be read back.
pub async fn show_written_via_api_is_readable() -> CheckOutcome {
    crate::runner::require_area("lighting")?;
    let project = lighting_project(false)?;
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
        readback.contains(crate::sabotage::pick(LIGHTING_GROUP, "e2e-absent-marker")),
        "the show read back does not contain what was written:\n{readback}"
    );

    server.check_clean_log(&[])?;
    Ok(())
}

/// A song with no lighting evicts the previous song's timeline.
///
/// The DMX engine holds one installed timeline. A song that has no show at all
/// returns early from the lighting setup, and used to do so without clearing
/// what the last song left behind -- so `GetCues` and `status.lighting` kept
/// answering with the previous song's cues while a song with no lighting
/// played. That reads as "a show is loaded but never armed", which is the
/// wrong answer from the surfaces added to end exactly that guesswork.
///
/// Ordering matters: the lit song has to play first so there is a timeline to
/// leak, and playback (not selection) is what installs it.
pub async fn a_song_without_lighting_clears_the_previous_timeline() -> CheckOutcome {
    crate::runner::require_area("lighting")?;

    let lit = SongSpec::tones("Lit Song", "lit-song", 2, 8.0).with_lighting(LightingSpec::simple(
        "E2E Show",
        "lighting/show.light",
        LIGHTING_GROUP,
    ));
    // The break point: give the second song a show of its own. Its cue list is
    // then legitimately non-empty, so the assertion below must fire.
    //
    // `active()`, not `perform()` — this break point *adds* something rather
    // than omitting it, and `perform()` is false under sabotage.
    let dark = SongSpec::tones("Dark Song", "dark-song", 2, 8.0);
    let dark = if crate::sabotage::active() {
        dark.with_lighting(LightingSpec::simple(
            "Sabotage Show",
            "lighting/sabotage.light",
            LIGHTING_GROUP,
        ))
    } else {
        dark
    };

    let project = ProjectBuilder::new()
        .profiles(vec![ProfileSpec::detected("01-e2e").with_lighting()])
        .songs(vec![lit, dark])
        .build()?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    check_eq!(
        client.subsystem_status("dmx").await?,
        "connected",
        "the DMX engine did not come up, so lighting cannot be exercised.\n--- log ---\n{}",
        server.log()
    );

    // Play the lit song so its timeline is installed.
    client.grpc().play(PlayRequest {}).await?;
    client.wait_until_playing(Duration::from_secs(10)).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let lit_cues = client
        .grpc()
        .get_cues(GetCuesRequest {})
        .await?
        .into_inner()
        .cues;
    check!(
        !lit_cues.is_empty(),
        "the lit song produced no cues, so there is no stale timeline to detect.\n--- log ---\n{}",
        server.log()
    );

    // Move to the song that has no lighting at all.
    client.grpc().stop(StopRequest {}).await?;
    client.grpc().next(NextRequest {}).await?;
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
        cues.is_empty(),
        "a song with no light show reported {} cue(s), which can only be the \
         previous song's timeline still installed ({} cue(s) then).\n--- log ---\n{}",
        cues.len(),
        lit_cues.len(),
        server.log()
    );
    crate::outcome::record(format!(
        "{} cue(s) while the lit song played, none once a song without lighting took over",
        lit_cues.len()
    ));

    server.check_clean_log(&[])?;
    Ok(())
}
