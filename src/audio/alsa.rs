// Copyright (C) 2024 Michael Wilson <mike@mdwn.dev>
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
use std::{collections::HashMap, error::Error, fmt};

use cpal::traits::{DeviceTrait, HostTrait};
use lazy_static::lazy_static;
use regex::Regex;
use tracing::error;

const PLUGHW: &str = "plughw";
const PULSE: &str = "pulse";

lazy_static! {
    static ref ALSA_CARD_NAME: Regex = Regex::new(r".*CARD=(?<name>[^,]*).*$").unwrap();
}

// Device is a small abstraction of an ALSA audio device.
pub struct Device {
    // Name is the name of the audio device.
    pub name: String,
    pub device: cpal::Device,
    pub channels: u32,
    pub long_name: String,
    pub matches: Vec<String>,
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (Channels={}) (ALSA)", self.name, self.channels)
    }
}

struct CardInfo {
    channels: u32,
    name: String,
    long_name: String,
    matches: Vec<String>,
}

/// Lists ALSA devices. Requires both an hw and plughw plugin for the given card.
pub fn list_devices() -> Result<Vec<Device>, Box<dyn Error>> {
    let mut device_lookup: HashMap<String, cpal::Device> = HashMap::new();
    let mut info_lookup: HashMap<String, CardInfo> = HashMap::new();

    // Hard code pulse info.
    info_lookup.insert(
        PULSE.to_string(),
        CardInfo {
            channels: 2,
            name: PULSE.to_string(),
            long_name: PULSE.to_string(),
            matches: vec![PULSE.to_string()],
        },
    );

    for card_result in alsa::card::Iter::new() {
        let card = card_result?;

        let card_name = format!("hw:{}", card.get_index());
        let ctl = alsa::Ctl::new(&card_name, false)?;
        let card_info = ctl.card_info()?;
        let id = card_info.get_id()?.to_string();
        let name = card_info.get_name()?.to_string();
        let long_name = card_info.get_longname()?.to_string();

        // Calculate max channels.
        let Ok(pcm) = alsa::pcm::PCM::new(&card_name, alsa::Direction::Playback, false) else {
            continue;
        };
        let Ok(hw_params) = alsa::pcm::HwParams::any(&pcm) else {
            continue;
        };
        info_lookup.insert(
            id.clone(),
            CardInfo {
                channels: hw_params.get_channels_max()?,
                name: name.clone(),
                long_name: long_name.clone(),
                matches: vec![id, name, long_name],
            },
        );
    }

    let host = cpal::host_from_id(cpal::HostId::Alsa)?;

    let result = host.output_devices();

    // We do this because the underlying ALSA lib is super noisy. We can iterate
    // directly over the call to host.devices(), but it looks like that's when the
    // noisy bits happen.
    let ssh = shh::stderr()?;
    let devices = result?.collect::<Vec<cpal::Device>>();
    drop(ssh);

    for device in devices {
        let full_name = &device.name()?;
        let result = get_card_name(full_name);
        match result {
            Ok(card_name) => {
                // We can query the channels from the HW plugin.
                if full_name.starts_with(PLUGHW) || card_name == PULSE {
                    device_lookup.insert(card_name.as_str().to_string(), device);
                }
            }
            Err(e) => error!(err = e.as_ref(), "Error getting audio device"),
        }
    }

    let mut devices: Vec<Device> = Vec::new();
    for ele in device_lookup {
        if info_lookup.contains_key(&ele.0) {
            let info = &info_lookup[&ele.0];
            devices.push(Device {
                name: info.name.clone(),
                device: ele.1,
                channels: info.channels,
                long_name: info.long_name.clone(),
                matches: info.matches.clone(),
            })
        }
    }
    Ok(devices)
}

// Gets the card name from the full ALSA name.
fn get_card_name(full_name: &String) -> Result<String, Box<dyn Error>> {
    if full_name == PULSE {
        return Ok(full_name.clone());
    }
    let Some(card_name_match) = ALSA_CARD_NAME.captures(full_name) else {
        return Err(format!("no match for device name {}", full_name).into());
    };
    let Some(card_name) = card_name_match.name("name") else {
        return Err(format!("unable to find card name in {}", full_name).into());
    };
    Ok(card_name.as_str().to_string())
}
