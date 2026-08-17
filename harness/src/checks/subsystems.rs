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

use mtrack::proto::player::v1::{GetConfigRequest, UpdateAudioRequest};

use crate::capabilities::Capabilities;
use crate::client::Client;
use crate::outcome::CheckOutcome;
use crate::project::{ProfileSpec, ProjectBuilder, Subsystem};
use crate::server::Server;
use crate::{check, check_eq, fail, inconclusive, skip};

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
    if crate::sabotage::active() && Capabilities::get().midi_out.is_none() {
        crate::no_control_here!(
            "this machine has no MIDI device, so making the profile declare one is a no-op"
        );
    }
    profile.midi = crate::sabotage::pick(Subsystem::Absent, Subsystem::Detected);

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
    profile.dmx = crate::sabotage::pick(Subsystem::Absent, Subsystem::Detected);

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
    // Same no-op as its sibling above: where no MIDI device exists, render_midi
    // returns early and the sabotaged profile is byte-identical.
    if crate::sabotage::active() && Capabilities::get().midi_out.is_none() {
        crate::no_control_here!(
            "this machine has no MIDI device, so making the profile declare one is a no-op"
        );
    }
    profile.midi = crate::sabotage::pick(
        Subsystem::Bogus("e2e-nonexistent-midi-device".to_string()),
        Subsystem::Detected,
    );

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
    profile.audio = crate::sabotage::pick(
        Subsystem::Bogus("e2e-nonexistent-audio-device".to_string()),
        Subsystem::Detected,
    );

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

/// A device that is present but will not open says so, distinctly from one that
/// is not there.
///
/// `retry_until_ready` re-runs device discovery twice a second, forever, and
/// logs each attempt. The two failures behind those lines want opposite
/// responses from an operator: a device that is still enumerating gets picked up
/// by the next retry, which is the whole reason the retry is perpetual, while
/// one that is sitting right there refusing to open will still be refusing in an
/// hour. Reporting both the same way leaves the person on the receiving end
/// waiting out a retry that can never succeed.
///
/// This also covers the #350 regression from the other side: an audio device
/// that could not be opened used to be reported as `connected`, because the
/// output thread swallowed the first build failure and `get_device` returned
/// `Ok`. The status assertion below fails against that build.
///
/// The lever is `bits_per_sample: 24` with the default integer format, which
/// `stream.rs` rejects outright rather than handing to the backend. That makes
/// the failure deterministic and independent of what the interface supports --
/// an unsupported *sample rate* would not do, because ALSA's plug layer
/// converts silently and the stream would open. If 24-bit integer output is
/// ever implemented, this check starts failing, which is the correct way to
/// find out that its lever needs replacing.
pub async fn unopenable_audio_device_is_distinguished_from_a_missing_one() -> CheckOutcome {
    crate::runner::require_area("subsystems")?;

    if Capabilities::get().audio_out.is_none() {
        skip!("no audio output device was detected, so there is nothing present to fail to open");
    }

    // Sabotage removes the device rather than the assertion: with a name that
    // resolves to nothing, mtrack correctly reports it missing, and a check that
    // cannot tell "missing" from "unopenable" passes anyway.
    //
    // Note the argument order against the neighbouring bogus-device check, which
    // is the reverse of this one: there the *real* case is the missing device
    // and sabotage restores a working one. `pick` takes (real, broken).
    let mut profile = ProfileSpec::detected("01-e2e").with_audio_key("bits_per_sample", "24");
    profile.audio = crate::sabotage::pick(
        Subsystem::Detected,
        Subsystem::Bogus("e2e-nonexistent-audio-device".to_string()),
    );

    let project = ProjectBuilder::new()
        .profiles(vec![profile])
        .songs(crate::checks::standard_songs())
        .build()?;
    let server = Server::start_degraded(&project).await?;
    let client = Client::connect_http_only(&server).await?;

    let seen = watch_subsystem(&client, "audio").await?;
    check!(
        !seen.iter().any(|s| s == "connected"),
        "an audio device that cannot be opened was reported as connected \
         (statuses seen: {seen:?})"
    );

    // Read after the settle window: the retry logs on every attempt, so waiting
    // guarantees at least one line rather than racing the first one.
    let log = server.log();
    check!(
        log.contains("was found but could not be opened"),
        "a device that was located but would not open did not say so.\n--- log ---\n{log}"
    );
    check!(
        !log.contains("no device found with name"),
        "a device that is present was reported as missing, which sends an operator \
         looking for the wrong fault.\n--- log ---\n{log}"
    );
    check!(
        !log.contains("panicked at"),
        "an unopenable audio device panicked the player.\n--- log ---\n{log}"
    );

    crate::outcome::record(format!(
        "unopenable audio device: statuses over {}s were {seen:?}, never connected, \
         reported as found-but-unopenable rather than missing",
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
        // Not a pass: with no audio device there is nothing to claim, and
        // returning Ok here would report success having verified nothing --
        // and then make --self-test call the check CANNOT FAIL for what is
        // really "this should have skipped".
        skip!("no audio output device, so there is no claimed device to compare");
    };

    // Sorts first under sabotage, and names a *valid* other device: a bogus one
    // stops the player booting, so the control died before this check's
    // assertion could run.
    let mut second = ProfileSpec::detected(crate::sabotage::pick("02-decoy", "00-decoy"));
    second.audio = if crate::sabotage::active() {
        // Must have at least as many channels as the real one: render_audio
        // derives track mappings and the pinned rate from the *selected*
        // device, so a narrower decoy maps tracks to channels that do not
        // exist and the player fails to boot -- the control would then die
        // before this check's assertion.
        // Mirrors render_audio: it clamps the channel count to a minimum of two
        // and pins a rate only for a *raw* device, so screening on anything else
        // rejects usable decoys and admits unusable ones.
        let wanted = caps
            .audio_out
            .as_ref()
            .map(|d| d.max_channels)
            .unwrap_or(2)
            .max(2);
        // render_audio also pins the *selected* device's rate into every
        // profile, so a decoy that cannot play it dies at startup for a reason
        // unrelated to which profile won.
        let pinned: Option<u32> = caps
            .audio_out
            .as_ref()
            .filter(|d| d.name.contains("hw:CARD=") && !d.name.contains("plughw:"))
            .filter(|d| !d.sample_rates.is_empty() && !d.sample_rates.contains(&44_100))
            .and_then(|d| {
                if d.sample_rates.contains(&48_000) {
                    Some(48_000)
                } else {
                    d.sample_rates.iter().copied().max()
                }
            });
        match caps.all_audio_out.iter().find(|d| {
            Some(&d.name) != caps.audio_out.as_ref().map(|s| &s.name)
                && d.max_channels >= wanted
                && pinned.is_none_or(|rate| d.sample_rates.contains(&rate))
        }) {
            Some(other) => Subsystem::Named(other.name.clone()),
            None => crate::no_control_here!(
                "no alternative output matching {wanted} channels and the pinned rate, so no \
                 usable decoy exists"
            ),
        }
    } else {
        Subsystem::Bogus("e2e-decoy-device".to_string())
    };

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

    let endpoint = "controllers/restart";
    let (status, body) = client
        .send_text(reqwest::Method::POST, endpoint, String::new())
        .await?;
    check!(
        status.is_success(),
        "restarting controllers failed: HTTP {status}\n{body}"
    );

    // The gRPC controller must come back, or the player has lost its control
    // surface without saying so. Reconnecting *is* this check's assertion, so a
    // failure here is a defect in the player -- not, as it was previously
    // reported, an error in the harness.
    // Sabotage the *reconnect*, which is the assertion this check exists for --
    // "controllers must come back, or control is silently lost".
    //
    // Weaker than a predicate-level substitution: the error is injected without
    // calling `Client::connect` at all, so it proves the match arm and `fail!`
    // render, not that a real failed reconnect is detected. Making a genuine
    // reconnect fail needs a second server or a wrong address, which is the
    // improvement to make here. Listed as predicate-level in the meantime, so
    // the score does not credit it as strong evidence.
    let reconnect = if crate::sabotage::active() {
        Err("sabotage: reconnect not attempted".into())
    } else {
        Client::connect(&server).await
    };
    let mut reconnected = match reconnect {
        Ok(client) => client,
        Err(e) => fail!(
            "the gRPC controller did not come back after a restart, so the player has \
             silently lost its control surface: {e}\n--- log ---\n{}",
            server.log()
        ),
    };
    if let Err(e) = reconnected.status().await {
        fail!("the restarted gRPC controller accepted a connection but not a request: {e}");
    }
    crate::outcome::record("gRPC controller answered again after restart");

    server.check_clean_log(&[])?;
    Ok(())
}

/// A rig whose audio device will never open still comes up, and can be fixed
/// from the running player.
///
/// This is the whole point of giving up on terminal failures. Init used to
/// retry an unopenable device twice a second forever, and because subsystems
/// are joined by phase, everything behind it stayed uninitialised: no MIDI, no
/// samples, no triggers, `init_done` never firing, and no controllers. A single
/// bad `bits_per_sample` therefore took away the web UI and the gRPC API that
/// are how anyone would correct it — the rig was unreachable precisely because
/// it was misconfigured.
///
/// `Server::start` waits for `init_done`, so its returning at all is the first
/// assertion: before the fix this check could not get past its own setup.
pub async fn a_rig_with_an_unopenable_device_comes_up_and_can_be_reconfigured() -> CheckOutcome {
    crate::runner::require_area("subsystems")?;
    let Some(device) = Capabilities::get().audio_out.clone() else {
        skip!("no audio output device was detected, so there is nothing present to fail to open");
    };

    // A real device asked for a bit depth cpal will not accept: present, and
    // refusing. Under sabotage the same device is asked for nothing unusual and
    // opens normally, so the "gave up" assertions below have nothing to find.
    // `pick` takes (real, broken).
    let audio_profile = crate::sabotage::pick(
        ProfileSpec::detected("01-e2e").with_audio_key("bits_per_sample", "24"),
        ProfileSpec::detected("01-e2e"),
    );

    let project = ProjectBuilder::new()
        .profiles(vec![audio_profile])
        .songs(crate::checks::standard_songs())
        .build()?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    // The gRPC controller answering at all means Phase 3 ran, which it could
    // not have while Phase 1 was still spinning.
    let status = client.get_json("status").await?;
    let audio = &status["hardware"]["audio"];
    check!(
        audio["status"] == "failed",
        "a device that will never open reported '{}', not 'failed'.\n\n\
         Anything else reads as still-trying or not-configured, and neither tells the \
         operator that the rig has stopped waiting for them.",
        audio["status"]
    );
    let reason = audio["error"].as_str().unwrap_or_default();
    check!(
        reason.contains("could not be opened"),
        "the failed audio subsystem gave no usable reason: {:?}.\n\n\
         'failed' without the reason leaves the operator exactly where the perpetual \
         retry did.",
        audio["error"]
    );
    check!(
        status["hardware"]["init_done"] == true,
        "hardware init never completed, so the rig is still held up by the one device \
         that will never come."
    );

    // The requirement in the operator's terms: the rig came up far enough to
    // accept a corrected configuration.
    let config = client
        .grpc()
        .get_config(GetConfigRequest {})
        .await?
        .into_inner();
    let updated = client
        .grpc()
        .update_audio(UpdateAudioRequest {
            audio_json: serde_json::json!({ "device": device.name }).to_string(),
            expected_checksum: config.checksum.clone(),
        })
        .await;
    check!(
        updated.is_ok(),
        "a degraded rig would not accept a corrected audio config: {:?}.\n\n\
         Coming up degraded is only worth anything if the config can then be fixed.",
        updated.err().map(|e| e.to_string())
    );

    crate::outcome::record(format!(
        "unopenable '{}': came up with init_done, reported failed ({reason}), and accepted a \
         corrected config",
        device.name
    ));
    Ok(())
}

/// A reload that lands during init must not leave a trigger engine behind.
///
/// Coverage caveat: this cannot attribute a failure to the install-ordering fix
/// alone. Against a build missing `reload_from_disk` it fails because the
/// trigger simply returns, which the sibling check already proves more
/// directly; against a build missing `install_if_current` it fails only when a
/// delay happens to land inside the Phase 3 window. It is kept because that
/// window is the one the original issue diagnosed and nothing else exercises
/// two overlapping reloads at all — but the attributable evidence for the
/// ordering fix is the unit test in `player::hardware`, which drives the
/// interleaving directly.
///
/// #380, reported from a live rig with nothing playing: disabling the triggers
/// did not disable them, and only a restart cleared it.
///
/// `init_hardware_async` checked cancellation and then, further on, wrote its
/// devices into the shared hardware state. In between it built the trigger
/// engine, which opens a cpal input stream — so the window was as long as a
/// device open. `reload_hardware` cancels the in-flight round and then resets
/// the state, but a round cancelled inside that window still finished opening
/// its stream and wrote it into the state its successor had just cleared. The
/// engine then held the input device and fired samples with no configuration
/// describing it and no UI able to reach it.
///
/// The reported trigger is "a save landing while boot init is still running",
/// which is what this drives: start with triggers configured, then disable them
/// over the API before init has finished. `start_degraded` returns on the first
/// HTTP 200, which mtrack serves while init is still going, so the save lands
/// in the window.
///
/// Each attempt is its own player, because the defect is permanent for the life
/// of the process. One stray engine across the attempts fails the check.
pub async fn a_reload_during_init_leaves_no_stray_trigger_engine() -> CheckOutcome {
    crate::runner::require_area("triggers")?;
    let caps = Capabilities::get();
    if caps.audio_in.is_none() {
        skip!("no audio input device, so no trigger engine can be built to strand");
    }

    // Two saves in quick succession, which is the other trigger the report
    // names. Racing *boot* init cannot be driven from outside: init finishes
    // before the HTTP surface is pollable, so the disable always lands after
    // the trigger engine is already up and nothing is ever cancelled mid-phase.
    //
    // Here the first save starts a reload and the second cancels it. The delay
    // between them sweeps, because the window that matters is Phase 3 — while
    // `init_trigger_engine` opens its input stream — and where that falls
    // depends on how long this rig's devices take to open.
    const DELAYS_MS: [u64; 8] = [0, 15, 30, 50, 75, 100, 150, 250];
    let mut attempts_made = 0;

    for delay_ms in DELAYS_MS {
        let project = ProjectBuilder::new()
            .profiles(vec![ProfileSpec::detected("01-e2e").with_trigger()])
            .songs(crate::checks::standard_songs())
            .build()?;
        let server = Server::start(&project).await?;
        let client = Client::connect(&server).await?;

        // The engine has to be up before there is anything to strand.
        if client.subsystem_status("trigger").await? != "connected" {
            crate::outcome::record(format!(
                "delay {delay_ms}ms: no trigger engine came up, so nothing could be stranded"
            ));
            continue;
        }

        // `/config/profiles/{index}` rather than `/profiles/{file}`: the file
        // endpoint writes YAML to disk and reloads, but the reload re-inits
        // from the store's in-memory config, which the file write never
        // touched — so the trigger comes straight back and the check would
        // report a legitimate engine as a stray one. This endpoint updates the
        // store and then reloads.
        let store = client.get_json("config/store").await?;
        let checksum = store["checksum"].as_str().unwrap_or_default().to_string();
        // `["profile"]`, not the whole body: the endpoint wraps it as
        // {"profile": ..., "yaml": ...}. `Profile` has no deny_unknown_fields
        // and every field is optional, so PUTting the wrapper deserialises to
        // an entirely empty profile — no audio, no controllers, no trigger.
        // The check then passed because the rig had been blanked rather than
        // because the trigger key was removed.
        let profile = client.get_json("profiles/01-e2e").await?["profile"].clone();
        let mut without_trigger = profile.clone();
        if let Some(object) = without_trigger.as_object_mut() {
            object.remove("trigger");
        }

        // First save: unchanged content, but the write path reloads regardless,
        // so this starts a round that will be opening devices.
        let (status, body) = client
            .put_json(
                "config/profiles/0",
                serde_json::json!({"profile": profile, "expected_checksum": checksum}),
            )
            .await?;
        if !status.is_success() {
            crate::outcome::record(format!(
                "delay {delay_ms}ms: the first write was rejected ({status}: {body})"
            ));
            continue;
        }

        tokio::time::sleep(Duration::from_millis(delay_ms)).await;

        // Second save: cancels the round the first one started.
        let (status, body) = client.put_json("profiles/01-e2e", without_trigger).await?;
        if !status.is_success() {
            crate::outcome::record(format!(
                "delay {delay_ms}ms: the second write was rejected ({status}: {body})"
            ));
            continue;
        }
        attempts_made += 1;

        // Long enough for both rounds to finish: the stray install happens at
        // the *end* of the losing one.
        tokio::time::sleep(SETTLE).await;

        // Predicate-level, for the same reason as the sibling check above; the
        // world-level control is running against the pre-fix install ordering.
        let status = crate::sabotage::pick(
            settled_status(&client, "trigger", SETTLE).await?,
            "connected".to_string(),
        );
        // Whether the config still declares a trigger separates the defect — an
        // engine no configuration accounts for — from this check simply failing
        // to disable anything.
        let after = client.get_json("profiles/01-e2e").await?["profile"].clone();
        let still_configured = after.get("trigger").is_some_and(|t| !t.is_null());
        check!(
            status != "connected",
            "delay {delay_ms}ms: a trigger engine is live after the triggers were \
             disabled by a save that cancelled an in-flight reload (status: \
             {status}; profile still declares a trigger: {still_configured}). If \
             the profile no longer declares one, this is an engine holding the \
             input device that no configuration accounts for."
        );

        server.check_clean_log(&[])?;
    }

    if attempts_made == 0 {
        inconclusive!("no attempt managed two successful saves, so no reload was ever cancelled");
    }
    // Says what was observed, not what was hoped for: the check drives two
    // saves and knows both were accepted, but it never sees whether the second
    // actually cancelled the first mid-init. Claiming it did would overstate
    // the coverage — the delay sweep makes it likely across eight attempts,
    // not certain on any one.
    crate::outcome::record(format!(
        "{attempts_made} of {} delay(s) drove two accepted saves in succession; \
         no stray engine after any of them",
        DELAYS_MS.len()
    ));
    Ok(())
}

/// Disabling triggers by saving the profile file must actually disable them.
///
/// The other half of #380, and the half that needs no race at all. Profiles in
/// `profiles_dir` are copied into the in-memory config by `Player::deserialize`
/// at startup, and `reload_hardware` re-initialises from that copy. Writing the
/// file through `PUT /api/profiles/{name}` did not touch it, so the save was
/// acknowledged, the hardware reloaded, and the trigger engine was rebuilt from
/// the profile as it was at boot — live, holding the input device, with the
/// file on disk saying it should not exist.
///
/// That is the reported symptom exactly: nothing playing, player unlocked, the
/// checkbox cleared, and only a restart made any difference.
pub async fn disabling_triggers_by_file_takes_effect() -> CheckOutcome {
    crate::runner::require_area("triggers")?;
    let caps = Capabilities::get();
    if caps.audio_in.is_none() {
        skip!("no audio input device, so there is no trigger engine to disable");
    }

    let project = ProjectBuilder::new()
        .profiles(vec![ProfileSpec::detected("01-e2e").with_trigger()])
        .songs(crate::checks::standard_songs())
        .build()?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    check_eq!(
        client.subsystem_status("trigger").await?,
        "connected",
        "no trigger engine came up, so disabling one proves nothing"
    );

    // `["profile"]`, not the whole body — see the sibling check.
    let mut profile = client.get_json("profiles/01-e2e").await?["profile"].clone();
    if let Some(object) = profile.as_object_mut() {
        object.remove("trigger");
    }
    check!(
        profile.get("audio").is_some(),
        "the profile fetched for editing has no audio, so this would blank the rig \
         rather than disable its trigger: {profile}"
    );

    let (status, body) = client.put_json("profiles/01-e2e", profile).await?;
    check!(
        status.is_success(),
        "saving the profile file was rejected ({status}: {body})"
    );

    // The write path reloads the hardware itself; this is time for that reload
    // to finish opening or not opening devices.
    tokio::time::sleep(SETTLE).await;

    // Asserted on the *running config*, not on whether an engine is live.
    //
    // "Is a trigger engine connected" cannot tell a correctly disabled trigger
    // from one whose engine simply failed to re-open the input device — the
    // outgoing engine is still releasing it while the new round tries, so the
    // status often does not return to "connected" either way. A check built on
    // that passes against the defect: verified by running it against a build
    // without the store refresh, where it passed happily.
    //
    // What the defect actually is, is the player reloading from a config that
    // no longer matches the file it just wrote. So ask the player what config
    // it is running.
    let running = client
        .grpc()
        .get_config(GetConfigRequest {})
        .await?
        .into_inner()
        .yaml;
    let running = crate::sabotage::pick(running, "  trigger:\n    device: sabotage\n".to_string());
    // A profile with no trigger serializes it as `trigger: ~`, and the config
    // also carries an unrelated `sample_triggers:` key — so "does it mention a
    // trigger" is the wrong question and answers yes either way. What matters
    // is whether the key has a *body*: a declared trigger is followed by
    // indented fields, a disabled one by `~`.
    let declares_trigger = running
        .lines()
        .any(|line| line.trim_start().starts_with("trigger:") && !line.trim_end().ends_with('~'));
    check!(
        !declares_trigger,
        "the profile file no longer declares a trigger and the save was accepted, but \
         the player is still running a config that does — so the reload re-initialised \
         from the copy it loaded at boot, and only a restart will clear it.\n\
         --- running config ---\n{running}"
    );

    // Recorded rather than asserted: whether the engine is up after a reload
    // depends on device release timing, which is not what this check is about.
    let status = settled_status(&client, "trigger", SETTLE).await?;
    crate::outcome::record(format!("trigger subsystem after the save: {status}"));

    server.check_clean_log(&[])?;
    crate::outcome::record("a trigger disabled by saving the profile file stayed disabled");
    Ok(())
}

/// Waits for a subsystem to stop reporting `initializing`, then returns its
/// status.
///
/// Asserting `status != "connected"` straight after a reload is not the claim
/// it appears to be: `initializing` satisfies it too, so a check can pass
/// because the engine has not come *back* yet rather than because it is gone.
/// `--self-test` caught exactly that — the break point left the trigger
/// configured, the engine was still re-initialising when the assertion ran,
/// and the check reported success against its own sabotage.
async fn settled_status(
    client: &Client,
    subsystem: &str,
    timeout: Duration,
) -> Result<String, crate::outcome::CheckError> {
    let deadline = std::time::Instant::now() + timeout;
    let mut last = client.subsystem_status(subsystem).await?;
    while last == "initializing" && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        last = client.subsystem_status(subsystem).await?;
    }
    Ok(last)
}
