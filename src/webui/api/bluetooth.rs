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

//! The Bluetooth controller's pairing as plain functions: what
//! `ConfigService`'s Bluetooth RPCs call. No transport types here, so a
//! caller maps a [`BluetoothError`] to its own refusal.

use std::fmt;
use std::time::{Duration, Instant};

use crate::controller::ble::{BleError, BleHandle, PairingState};
use crate::player::Player;
use crate::webui::wire::bluetooth::{BluetoothPhone, BluetoothStatus, PairingWindow};

/// How long a pairing window stays open when the request names no time.
const DEFAULT_WINDOW: Duration = Duration::from_secs(120);
/// The shortest and longest window a request may ask for, in seconds.
const WINDOW_RANGE: std::ops::RangeInclusive<u32> = 10..=600;

/// Why a pairing call was refused. The message is the caller's to show.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BluetoothError {
    /// Bluetooth is not running.
    NotRunning(String),
    /// The address is none.
    BadAddress(String),
    /// The address is not a phone paired with this player.
    NotPaired(String),
    /// The radio stack refused.
    Adapter(String),
}

impl fmt::Display for BluetoothError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BluetoothError::NotRunning(m)
            | BluetoothError::BadAddress(m)
            | BluetoothError::NotPaired(m)
            | BluetoothError::Adapter(m) => f.write_str(m),
        }
    }
}

impl From<&BleError> for BluetoothError {
    fn from(e: &BleError) -> Self {
        match e {
            BleError::NotRunning => BluetoothError::NotRunning(e.to_string()),
            BleError::BadAddress(_) => {
                BluetoothError::BadAddress("Not a Bluetooth address".to_string())
            }
            BleError::NotPaired(_) => {
                BluetoothError::NotPaired("Not a phone paired with this player".to_string())
            }
            BleError::Adapter(_) => BluetoothError::Adapter(e.to_string()),
        }
    }
}

/// The Bluetooth controller as it is now: configured when the running config
/// has one, running when its handle answers. Its failure text is the
/// controller status's own. Whether Bluetooth runs, the pairing window and the
/// paired phones.
pub(crate) async fn status_of(player: &Player) -> BluetoothStatus {
    let statuses = player.controller_statuses();
    let ble = statuses.iter().find(|s| s.kind == "ble");
    let handle = player.ble_handle();
    let mut status = BluetoothStatus {
        configured: ble.is_some() || handle.is_some(),
        running: false,
        error: None,
        pairing: PairingWindow {
            open: false,
            seconds_left: 0,
        },
        devices: Vec::new(),
    };
    let Some(handle) = handle.as_ref() else {
        status.error = ble.and_then(|s| s.error.clone());
        return status;
    };
    let Ok(window) = handle.pairing_state() else {
        // A controller that is retrying says why itself; one that stopped
        // for good says it through its status.
        status.error = handle
            .failure()
            .or_else(|| ble.and_then(|s| s.error.clone()));
        return status;
    };
    status.running = true;
    if let PairingState::Open { until } = window {
        let left = until.saturating_duration_since(Instant::now());
        status.pairing = PairingWindow {
            open: !left.is_zero(),
            seconds_left: u32::try_from(left.as_secs_f64().ceil() as u64).unwrap_or(u32::MAX),
        };
    }
    match handle.paired_devices().await {
        Ok(phones) => {
            status.devices = phones
                .into_iter()
                .map(|p| BluetoothPhone {
                    address: p.address,
                    name: p.name,
                    connected: p.connected,
                })
                .collect();
        }
        Err(e) => status.error = Some(e.to_string()),
    }
    status
}

/// The running controller's handle, or the refusal that says there is none.
fn running_handle(player: &Player) -> Result<BleHandle, BluetoothError> {
    player
        .ble_handle()
        .ok_or_else(|| BluetoothError::from(&BleError::NotRunning))
}

/// Opens the pairing window (for `seconds`, 120 by default, kept between 10
/// and 600) or closes it, and answers the controller as it is after. The
/// caller refuses it while the player is locked: who may control the player
/// is not for a locked player to change.
pub(crate) async fn set_pairing(
    player: &Player,
    open: bool,
    seconds: Option<u32>,
) -> Result<BluetoothStatus, BluetoothError> {
    let handle = running_handle(player)?;
    let result = if open {
        let seconds = seconds.map_or(DEFAULT_WINDOW.as_secs() as u32, |s| {
            s.clamp(*WINDOW_RANGE.start(), *WINDOW_RANGE.end())
        });
        handle
            .open_pairing(Duration::from_secs(u64::from(seconds)))
            .await
    } else {
        handle.close_pairing().await
    };
    result.map_err(|e| BluetoothError::from(&e))?;
    Ok(status_of(player).await)
}

/// Forgets a paired phone, which must pair again, and answers the controller
/// as it is after.
pub(crate) async fn forget(
    player: &Player,
    address: &str,
) -> Result<BluetoothStatus, BluetoothError> {
    let handle = running_handle(player)?;
    handle
        .forget(address)
        .await
        .map_err(|e| BluetoothError::from(&e))?;
    Ok(status_of(player).await)
}
