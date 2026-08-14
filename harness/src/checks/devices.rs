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
//! Agreement between the device lists mtrack publishes.
//!
//! mtrack enumerates audio devices through two separate paths, and a user only
//! ever sees the first one:
//!
//! * [`mtrack::audio::list_device_info`] backs `GET /api/devices/audio`, which
//!   is what the web UI's device picker offers.
//! * [`mtrack::audio::list_devices`] backs the `mtrack devices` command and,
//!   more importantly, is the list `Device::get` searches when the player
//!   actually opens a device.
//!
//! If the first list is a superset of the second, the UI offers devices that
//! playback will then reject. That is invisible to unit tests, because it only
//! happens against real hardware that ALSA reports inconsistently.

use crate::capabilities::Capabilities;
use crate::client::Client;
use crate::outcome::{CheckError, CheckOutcome};
use crate::server::Server;
use crate::{check, skip};

/// Every device offered to users can actually be opened by the player.
pub async fn advertised_devices_are_openable() -> CheckOutcome {
    // Gated on the raw advertised list, not `all_audio_out` -- that is the
    // *intersection* of advertised and openable, so on a machine where none of
    // the advertised devices can be opened (the maximal form of exactly the
    // defect this check reports) it would be empty and the run would print SKIP
    // instead of FAIL.
    let advertised_raw = mtrack::audio::list_device_info()
        .map_err(|e| CheckError::Harness(format!("could not list device info: {e}")))?;
    if advertised_raw.is_empty() {
        skip!("this machine advertises no audio devices at all, so there is nothing to compare");
    }
    // Failing to enumerate is a harness problem, not a finding about mtrack's
    // behaviour, so it propagates rather than being reported as a defect.
    let advertised: Vec<String> = advertised_raw.into_iter().map(|d| d.name).collect();

    // Each device is resolved on its own, the way playback resolves the one the
    // operator chose.
    //
    // The original form of this check compared the picker's list against
    // `list_devices()`, and that comparison was measuring the wrong thing:
    // every device in that list holds an open ALSA handle, and ALSA will not
    // describe a device while its siblings are held, so *building* the list
    // truncates it. Alternating the two enumerations in one process on the test
    // rig gave 19 devices, then 8, then 7, then 3 -- the "openable" side of the
    // comparison was an artifact of having enumerated at all.
    //
    // What an operator actually needs is that picking any advertised device
    // works, so that is what is asked, one device at a time.
    let unresolvable: Vec<&String> = advertised
        .iter()
        .filter(|name| {
            if crate::sabotage::active() {
                return true;
            }
            !mtrack::audio::can_open_device(name)
        })
        .collect();

    check!(
        unresolvable.is_empty(),
        "GET /api/devices/audio offers {} device(s) that the player cannot resolve, so selecting \
         one in the web UI fails with \"no device found\":\n  {}\n\nadvertised: {:?}",
        unresolvable.len(),
        unresolvable
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  "),
        advertised,
    );

    crate::outcome::record(format!(
        "{} advertised device(s), all openable by the player",
        advertised.len()
    ));
    Ok(())
}

/// The channel count advertised for the selected device is plausible.
///
/// ALSA plug devices claim 32 channels for hardware that has far fewer. A
/// profile generated from that number maps tracks onto channels that do not
/// exist, which fails silently: the mixer writes them and nothing is heard.
pub async fn selected_output_is_real_hardware() -> CheckOutcome {
    let caps = Capabilities::get();
    let Some(device) = &caps.audio_out else {
        skip!("no audio output device was selected");
    };

    crate::outcome::record(format!(
        "selected output: {} ({} ch)",
        device.name, device.max_channels
    ));
    for other in &caps.all_audio_out {
        crate::outcome::record(format!(
            "  candidate: {} ({} ch)",
            other.name, other.max_channels
        ));
    }

    let is_raw_hw = crate::sabotage::perform()
        && device.name.contains("hw:CARD=")
        && !device.name.contains("plughw:");
    if is_raw_hw {
        return Ok(());
    }

    // Selecting a plug device is only wrong when a raw one was available. Some
    // interfaces expose a native format cpal cannot open at all -- packed
    // 24-bit (S24_3LE) is the common case, and cpal has it commented out of
    // its ALSA format table -- so their raw device never appears here and the
    // plug layer is the only way in. That is a legitimate configuration, not a
    // defect, but it does mean the channel count is the plug layer's.
    // Under sabotage, pretend a raw device was available. Relying on the rig to
    // supply one made this control rig-dependent: on a console whose only
    // openable device is a plug node -- the case this check exists to tolerate
    // -- the failure condition is unreachable and the check could not fail.
    let raw_alternatives: Vec<&str> = if crate::sabotage::active() {
        vec!["e2e-pretend-raw:hw:CARD=X,DEV=0"]
    } else {
        caps.all_audio_out
            .iter()
            .filter(|d| d.name.contains("hw:CARD=") && !d.name.contains("plughw:"))
            .map(|d| d.name.as_str())
            .collect()
    };

    if !raw_alternatives.is_empty() {
        // Reachable only by an operator override; device_rank already prefers
        // raw hardware. Blaming mtrack (and exiting non-zero) for the harness's
        // own device choice would be wrong.
        crate::inconclusive_verdict!(
            "selected the plug device '{}' even though raw hardware was available: {:?}. Its \
             {}-channel count is the plug layer's, not the interface's, so routing measured \
             through it would not mean what it appears to. Check MTRACK_E2E_AUDIO_DEVICE.",
            device.name,
            raw_alternatives,
            device.max_channels
        );
    }

    crate::outcome::record(format!(
        "caveat: '{}' is an ALSA plug device, but no raw hw device was usable -- its \
         {}-channel count comes from the plug layer, not the interface",
        device.name, device.max_channels
    ));
    Ok(())
}

/// The device the suite will actually use can stream, not merely resolve.
///
/// `advertised_devices_are_openable` asks whether each advertised name
/// *resolves*, which is the strongest question that can be asked cheaply — but
/// it is not the question that matters. Production is the counterexample:
/// `hw:CARD=WING` resolves and then refuses to open, because cpal has S24_3LE
/// commented out of its ALSA format table. Resolution says nothing about
/// whether the format will be accepted or the callback will ever fire.
///
/// This opens one device — the one the suite is about to play through — waits
/// for its output callback, and closes it. Silent: no sources are added, so the
/// device is handed zeroes.
///
/// Deliberately not run against every advertised device. Opening nineteen
/// interfaces to see which survive would disturb whatever else is using them,
/// and the one that matters is the one about to be used.
pub async fn selected_device_actually_streams() -> CheckOutcome {
    let Some(device) = Capabilities::get().audio_out.as_ref() else {
        skip!("no audio output device was detected, so there is nothing to open");
    };

    let name = if crate::sabotage::active() {
        // Break the world, not the assertion: a device that cannot be opened
        // must make this fail, or it is asserting nothing.
        "e2e-nonexistent-audio-device".to_string()
    } else {
        device.name.clone()
    };

    let outcome = mtrack::audio::probe_device(mtrack::config::Audio::new(&name));
    check!(
        outcome.is_ok(),
        "the device the suite plays through does not stream: {outcome}.\n\n\
         Resolving a device proves its name exists; it does not prove the format \
         is accepted or that the callback ever runs."
    );
    crate::outcome::record(format!("{name}: {outcome}"));
    Ok(())
}

/// Testing a device from the web UI does not call the working one broken.
///
/// The config editor's Test button probes the device named beside it, and
/// probing means opening. An ALSA device the player already holds will refuse
/// the second open — so the naive implementation reports "could not be opened"
/// for the one device with minutes of positive evidence behind it, which is the
/// worst answer the endpoint could give. mtrack answers from live health
/// instead, and this is the only place that can be proven: the unit test's mock
/// device would never refuse a second open, so it cannot see the regression.
pub async fn probing_the_device_in_use_reports_it_rather_than_failing() -> CheckOutcome {
    let Some(device) = Capabilities::get().audio_out.as_ref() else {
        skip!("no audio output device was detected, so nothing can be held open");
    };

    let project = crate::checks::standard_project()?;
    let server = Server::start(&project).await?;
    let client = Client::connect_http_only(&server).await?;

    // Asking about a device nobody holds must not answer "in use", or this is
    // reading a constant. Note the weaker class of control: the world is left
    // intact and the request is changed, so this check stays off WORLD_LEVEL.
    let asked = crate::sabotage::pick(
        device.name.clone(),
        "e2e-nonexistent-audio-device".to_string(),
    );

    let (status, body) = client
        .post_json("devices/audio/probe", serde_json::json!({"device": asked}))
        .await?;
    check!(
        status.is_success(),
        "POST /api/devices/audio/probe returned {status}: {body}"
    );

    let outcome = body["outcome"].as_str().unwrap_or("<missing>");
    check!(
        outcome == "in_use",
        "testing the device the player is holding reported '{outcome}', not 'in_use'.\n\n\
         The device is open and streaming; a probe that reopens it is refused, and the \
         operator is told their working interface is broken."
    );
    check!(
        body["ok"] == serde_json::json!(true),
        "the device in use was reported as not ok: {body}"
    );

    crate::outcome::record(format!(
        "{asked}: reported in_use after {} callbacks, not reopened",
        body["callbacks"]
    ));
    Ok(())
}

/// Testing a device leaves it usable, rather than holding it for the process.
///
/// A probe opens a real interface and is expected to hand it back. That was
/// invisible while the only caller was `mtrack test-audio`, a process that
/// exits immediately afterwards. The web UI's Test button runs the same probe
/// inside the long-lived player, where a leaked handle would make the tested
/// device permanently unopenable — including by the profile save that testing
/// it was the prelude to. The button would brick exactly the device it just
/// pronounced healthy.
///
/// Only real hardware can show this. ALSA refusing a second open is the whole
/// mechanism, and a mock device has nothing to refuse.
pub async fn a_tested_device_is_still_usable_afterwards() -> CheckOutcome {
    let Some(device) = Capabilities::get().audio_out.as_ref() else {
        skip!("no audio output device was detected, so there is nothing to reopen");
    };
    let config = || mtrack::config::Audio::new(&device.name);

    let first = mtrack::audio::probe_device(config());
    if !first.is_ok() {
        skip!("the device does not stream at all ({first}), so reuse cannot be measured");
    }

    // Break the world, not the assertion: holding the device open across the
    // second probe is precisely the leak this check exists to catch, so the
    // check must fail when it is simulated.
    let held = if crate::sabotage::active() {
        Some(
            mtrack::audio::get_device(Some(config()))
                .map_err(|e| CheckError::Harness(format!("could not hold the device open: {e}")))?,
        )
    } else {
        None
    };

    let second = mtrack::audio::probe_device(config());
    check!(
        second.is_ok(),
        "the device could not be tested twice in one process: {second}.\n\n\
         The first probe did not hand the interface back, so testing a device from the \
         web UI makes it unopenable until mtrack restarts — including by the profile \
         save that testing it was the prelude to."
    );

    // What saving the profile does. A probe that released enough for another
    // probe but not enough for playback would still break the flow.
    let opened = mtrack::audio::get_device(Some(config()));
    check!(
        opened.is_ok(),
        "the device could not be opened for playback after being tested: {}.\n\n\
         Testing a device must not stand between it and the profile save that follows.",
        opened.err().map(|e| e.to_string()).unwrap_or_default()
    );

    drop(held);
    crate::outcome::record(format!(
        "{}: probed twice and then opened for playback in one process",
        device.name
    ));
    Ok(())
}

/// Testing a device other than the one in use is actually tested, not waved
/// through as "in use".
///
/// The in-use branch turns on `Device::matches_name`, which compares against
/// both the configured name and the name it resolved to. A false positive there
/// is the dangerous direction: it reports a *green* result — the device is open
/// and streaming — for an interface that was never opened at all. Nothing
/// downstream would question it, because the answer looks like the good one.
///
/// Real name resolution is the point. A unit test compares strings it chose
/// itself; here `default` and `hw:CARD=X,DEV=0` are resolved by ALSA, which is
/// where a configured name and a resolved name genuinely diverge.
pub async fn testing_another_device_is_not_confused_for_the_one_in_use() -> CheckOutcome {
    let caps = Capabilities::get();
    let Some(device) = caps.audio_out.as_ref() else {
        skip!("no audio output device was detected, so nothing can be held open");
    };
    let Some(other) = caps
        .all_audio_out
        .iter()
        .find(|d| d.name != device.name)
        .map(|d| d.name.clone())
    else {
        skip!("only one audio output device on this machine, so there is no other to ask about");
    };

    let project = crate::checks::standard_project()?;
    let server = Server::start(&project).await?;
    let client = Client::connect_http_only(&server).await?;

    // Break the world, not the assertion: asking about the device that *is*
    // held must make this fail, or it is asserting nothing about the matching.
    let asked = crate::sabotage::pick(other.clone(), device.name.clone());

    let (status, body) = client
        .post_json("devices/audio/probe", serde_json::json!({"device": asked}))
        .await?;
    check!(
        status.is_success(),
        "POST /api/devices/audio/probe returned {status}: {body}"
    );

    let outcome = body["outcome"].as_str().unwrap_or("<missing>");
    check!(
        outcome != "in_use",
        "testing '{asked}' reported 'in_use', but the player is holding '{}'.\n\n\
         A device that was never opened is being reported as open and streaming — a green \
         result for an interface nobody has touched, which is the one wrong answer that \
         looks right.",
        device.name
    );

    crate::outcome::record(format!(
        "held '{}', asked about '{asked}': reported '{outcome}', not in_use",
        device.name
    ));
    Ok(())
}

/// During playback, testing another device is refused while the device in use
/// still answers.
///
/// Two behaviours that have to hold together. Opening a second interface
/// mid-song risks an xrun on the one that is playing, so it is refused — but
/// the refusal is checked *after* the in-use branch, because "is the device I
/// am playing through all right?" is exactly the question worth asking when a
/// rig goes quiet during a set, and it opens nothing to answer it.
///
/// The ordering is the part that cannot be checked anywhere else: both guards
/// are cheap to write in the wrong order and the mistake is invisible until a
/// show.
pub async fn testing_during_playback_refuses_others_and_answers_for_the_one_in_use() -> CheckOutcome
{
    crate::runner::require_area("playback")?;
    let caps = Capabilities::get();
    let Some(device) = caps.audio_out.as_ref() else {
        skip!("no audio output device was detected, so there is nothing to play through");
    };
    let Some(other) = caps
        .all_audio_out
        .iter()
        .find(|d| d.name != device.name)
        .map(|d| d.name.clone())
    else {
        skip!("only one audio output device on this machine, so there is no other to ask about");
    };

    // Long enough that the song cannot end underneath the assertions.
    let project = crate::project::ProjectBuilder::new()
        .songs(vec![crate::songs::SongSpec::tones(
            "Long Tone",
            "long-tone",
            1,
            120.0,
        )])
        .build()?;
    let server = Server::start(&project).await?;
    let mut client = Client::connect(&server).await?;

    client
        .grpc()
        .play(mtrack::proto::player::v1::PlayRequest {})
        .await?;
    client
        .wait_until_playing(std::time::Duration::from_secs(10))
        .await?;

    // Under sabotage, ask about the held device instead. It must be answered
    // rather than refused, so a check that only ever saw refusals would fail.
    let asked = crate::sabotage::pick(other.clone(), device.name.clone());
    let (status, body) = client
        .post_json("devices/audio/probe", serde_json::json!({"device": asked}))
        .await?;
    check!(
        status == reqwest::StatusCode::CONFLICT,
        "testing '{asked}' during playback returned {status} rather than 409: {body}.\n\n\
         Opening a second interface mid-song risks an xrun on the one that is playing."
    );

    // The device being played through is answered from health, opening nothing.
    let (status, body) = client
        .post_json(
            "devices/audio/probe",
            serde_json::json!({"device": device.name}),
        )
        .await?;
    check!(
        status.is_success() && body["outcome"] == "in_use",
        "testing the device being played through returned {status} / {}, not 200 / in_use.\n\n\
         The playback guard is being applied before the in-use branch, so the one question \
         worth asking during a set — is my output still alive? — is refused.",
        body["outcome"]
    );
    check!(
        body["ok"] == serde_json::json!(true),
        "the device being played through reported not ok: {body}"
    );

    client
        .grpc()
        .stop(mtrack::proto::player::v1::StopRequest {})
        .await?;
    crate::outcome::record(format!(
        "during playback: '{asked}' refused with 409, '{}' answered in_use",
        device.name
    ));
    Ok(())
}

/// Overlapping device tests do not make each other fail.
///
/// A probe opens a real interface, and ALSA refuses a concurrent second open of
/// the same one. Without serialization in the endpoint, two clients testing at
/// once -- two browser tabs is enough -- leave one of them reading "could not
/// be opened" for a device that is fine. That is the same wrong answer the
/// in-use branch exists to prevent, arriving by a different route.
///
/// Goes through the endpoint rather than calling `probe_device` directly, since
/// the lock being exercised is the endpoint's. Only real hardware shows this:
/// the refusal is ALSA's, and a mock has nothing to refuse.
pub async fn overlapping_device_tests_do_not_fail_each_other() -> CheckOutcome {
    let Some(device) = Capabilities::get().audio_out.as_ref() else {
        skip!("no audio output device was detected, so there is nothing to test");
    };

    // The player must not hold the device under test, or every request is
    // answered from health without opening anything and the contention is never
    // exercised. A profile naming a device that does not exist leaves the
    // player holding nothing while its web API still answers.
    let mut profile = crate::project::ProfileSpec::detected("01-e2e");
    profile.audio = crate::project::Subsystem::Bogus("e2e-nonexistent-audio-device".to_string());
    let project = crate::project::ProjectBuilder::new()
        .profiles(vec![profile])
        .songs(crate::checks::standard_songs())
        .build()?;
    let server = Server::start_degraded(&project).await?;
    let client = Client::connect_http_only(&server).await?;

    // Break the world, not the assertion: holding the device open makes every
    // request fail, which is both the negative control and the proof that ALSA
    // really does refuse a second open. Without that, a machine whose driver
    // allowed concurrent opens would pass this check while proving nothing.
    let held = if crate::sabotage::active() {
        Some(
            mtrack::audio::get_device(Some(mtrack::config::Audio::new(&device.name)))
                .map_err(|e| CheckError::Harness(format!("could not hold the device open: {e}")))?,
        )
    } else {
        None
    };

    let concurrent = 3;
    let mut requests = Vec::new();
    for _ in 0..concurrent {
        requests.push(client.post_json(
            "devices/audio/probe",
            serde_json::json!({"device": device.name}),
        ));
    }
    let responses = futures_util::future::join_all(requests).await;

    let mut failures = Vec::new();
    for response in responses {
        let (status, body) = response?;
        if !status.is_success() || body["ok"] != serde_json::json!(true) {
            failures.push(format!("{status}: {body}"));
        }
    }
    drop(held);

    check!(
        failures.is_empty(),
        "{} of {concurrent} overlapping tests of '{}' failed while the device was otherwise \
         free:\n  {}\n\n\
         Two clients testing at once must not make each other report a working device as \
         broken.",
        failures.len(),
        device.name,
        failures.join("\n  ")
    );

    crate::outcome::record(format!(
        "{concurrent} overlapping tests of '{}' all reported a streaming device",
        device.name
    ));
    Ok(())
}
