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

//! Bluetooth LE remote control. A phone pairs with the player and drives
//! it over a GATT service, with no network between them.
//!
//! The layers, from the radio up: `gatt` is the BlueZ glue, the only code
//! that sees the radio stack; `session` holds the per-phone state and
//! turns writes into dispatched requests and responses into chunks;
//! `pairing` is the policy of who may pair and when; `status` builds what
//! the phone watches; `notify` fans chunks out to subscribers; the
//! `codec` frames envelopes for ATT chunks and the `dispatch`er answers
//! them by calling the player's gRPC service in process. Everything but
//! `gatt` is tested without a radio.

use std::error::Error;
use std::io;
use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::config;
use crate::player::Player;

pub mod codec;
// Without the BlueZ glue nothing but tests calls these.
#[cfg_attr(not(all(target_os = "linux", feature = "ble")), allow(dead_code))]
pub(crate) mod dispatch;
#[cfg(all(target_os = "linux", feature = "ble"))]
mod gatt;
#[cfg_attr(not(all(target_os = "linux", feature = "ble")), allow(dead_code))]
mod notify;
#[cfg_attr(not(all(target_os = "linux", feature = "ble")), allow(dead_code))]
mod pairing;
#[cfg_attr(not(all(target_os = "linux", feature = "ble")), allow(dead_code))]
mod session;
#[cfg_attr(not(all(target_os = "linux", feature = "ble")), allow(dead_code))]
mod status;

#[cfg(test)]
pub(crate) use pairing::fake;
pub use pairing::{BleError, BleHandle, PairedDevice, PairingState};

/// The UUID of the GATT service a phone looks for. Its characteristics
/// share the rest of the UUID, apart from the 16 bits that tell them apart.
pub const SERVICE_UUID: u128 = 0x6d747261_636b_0001_8000_00805f9b34fb;
/// The characteristic a phone writes framed requests to: write and write
/// without response, encrypted link required.
pub const REQUEST_UUID: u128 = 0x6d747261_636b_0002_8000_00805f9b34fb;
/// The characteristic framed responses come back on: notify.
pub const RESPONSE_UUID: u128 = 0x6d747261_636b_0003_8000_00805f9b34fb;
/// The characteristic carrying the player's status: read (a bare encoded
/// `Status`) and notify (the same, framed).
pub const STATUS_UUID: u128 = 0x6d747261_636b_0004_8000_00805f9b34fb;

/// How long the controller waits before trying the radio again.
#[cfg(all(target_os = "linux", feature = "ble"))]
const RETRY: std::time::Duration = std::time::Duration::from_secs(5);

/// What a controller configured on a build without Bluetooth says.
#[cfg(not(all(target_os = "linux", feature = "ble")))]
const NO_BLUETOOTH: &str = "this build has no Bluetooth support";

/// A controller that lets a paired phone control the player over Bluetooth
/// LE.
pub struct Driver {
    #[cfg_attr(not(all(target_os = "linux", feature = "ble")), allow(dead_code))]
    player: Arc<Player>,
    #[cfg_attr(not(all(target_os = "linux", feature = "ble")), allow(dead_code))]
    name: Option<String>,
    handle: BleHandle,
}

impl Driver {
    /// A controller advertising as the config says. The radio is not
    /// touched until it runs: a missing adapter fails the running
    /// controller, not the player. Without Bluetooth in the build, it is
    /// refused here.
    pub fn new(
        config: config::BleController,
        player: Arc<Player>,
    ) -> Result<Arc<Self>, Box<dyn Error>> {
        #[cfg(not(all(target_os = "linux", feature = "ble")))]
        {
            let _ = (&config, &player);
            Err(NO_BLUETOOTH.into())
        }
        #[cfg(all(target_os = "linux", feature = "ble"))]
        Ok(Arc::new(Driver {
            player,
            name: config.name().map(str::to_string),
            handle: BleHandle::default(),
        }))
    }
}

impl super::Driver for Driver {
    fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>> {
        #[cfg(all(target_os = "linux", feature = "ble"))]
        {
            let (name, player, handle) =
                (self.name.clone(), self.player.clone(), self.handle.clone());
            // The radio is often not there yet when the player starts at
            // boot, and can be blocked and unblocked while it runs, so a
            // failure is retried rather than left for the next reload. The
            // reason is logged when it changes, not on every attempt.
            tokio::spawn(async move {
                let mut logged: Option<String> = None;
                loop {
                    let Err(e) = gatt::run(name.clone(), player.clone(), handle.clone()).await
                    else {
                        return Ok(());
                    };
                    let why = e.to_string();
                    if logged.as_deref() != Some(why.as_str()) {
                        tracing::warn!(
                            error = %why,
                            retry_secs = RETRY.as_secs(),
                            "Bluetooth controller is not running; it will retry"
                        );
                        logged = Some(why.clone());
                    }
                    handle.failed(&why);
                    tokio::time::sleep(RETRY).await;
                }
            })
        }
        #[cfg(not(all(target_os = "linux", feature = "ble")))]
        tokio::spawn(async { Err(io::Error::other(NO_BLUETOOTH)) })
    }

    fn ble_handle(&self) -> Option<BleHandle> {
        Some(self.handle.clone())
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    /// The UUIDs, as the docs and the app write them.
    #[test]
    fn the_uuids_are_the_documented_ones() {
        let text = |uuid: u128| {
            let hex = format!("{uuid:032x}");
            format!(
                "{}-{}-{}-{}-{}",
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..]
            )
        };
        assert_eq!(text(SERVICE_UUID), "6d747261-636b-0001-8000-00805f9b34fb");
        assert_eq!(text(REQUEST_UUID), "6d747261-636b-0002-8000-00805f9b34fb");
        assert_eq!(text(RESPONSE_UUID), "6d747261-636b-0003-8000-00805f9b34fb");
        assert_eq!(text(STATUS_UUID), "6d747261-636b-0004-8000-00805f9b34fb");
    }

    /// Without Bluetooth in the build, the controller says so rather than
    /// starting, and the player carries on.
    #[cfg(not(all(target_os = "linux", feature = "ble")))]
    #[tokio::test]
    async fn a_build_without_bluetooth_says_so() {
        let player = crate::testutil::player::assets_player()
            .build()
            .expect("player");
        let e = Driver::new(config::BleController::default(), player)
            .err()
            .expect("refused");
        assert!(e.to_string().contains("no Bluetooth support"), "{e}");
    }
}
