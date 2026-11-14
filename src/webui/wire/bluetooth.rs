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

//! The Bluetooth pairing's payloads (`ConfigService`'s Bluetooth RPCs): whether the controller
//! runs, whether phones may pair, and the phones paired.

use serde::Serialize;

/// What `GetBluetooth` answers, and what every change to it answers: the
/// Bluetooth controller as it is now.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct BluetoothStatus {
    /// Whether the running config has a Bluetooth controller.
    pub configured: bool,
    /// Whether the controller is up and answering. Closed and empty when
    /// it is not.
    pub running: bool,
    /// Why it is not running when it is configured, or what the adapter
    /// refused when listing phones.
    pub error: Option<String>,
    pub pairing: PairingWindow,
    pub devices: Vec<BluetoothPhone>,
}

/// Whether phones may pair.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct PairingWindow {
    pub open: bool,
    /// Whole seconds until it closes by itself, rounded up; 0 when closed.
    pub seconds_left: u32,
}

/// A phone paired with the player.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct BluetoothPhone {
    /// `AA:BB:CC:DD:EE:FF`.
    pub address: String,
    /// The name it gave, when it gave one.
    pub name: Option<String>,
    pub connected: bool,
}
