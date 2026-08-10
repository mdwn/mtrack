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
use crate::outcome::{CheckError, CheckOutcome};
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
