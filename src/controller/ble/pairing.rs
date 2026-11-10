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

//! Who may pair, and when. The adapter accepts new pairings only while a
//! pairing window is open, and a phone that connects without being paired
//! is dropped once the window is closed. The policy asks the radio stack
//! for nothing but four operations, [`AdapterOps`], which BlueZ glue
//! implements.
//!
//! The player accepts a pairing without anyone checking a code, so a
//! pairing is encrypted but not authenticated: whoever
//! pairs during the window is trusted from then on. So the window is
//! closed by default. It opens by itself only while no phone has ever
//! paired, so the first phone can claim a fresh player (trust on first
//! use), and otherwise only when the owner opens it through a
//! [`BleHandle`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use parking_lot::Mutex;
use tokio::task::AbortHandle;

use super::session::{format_address, parse_address, PeerId};

/// How long the window stays open at start while no phone is paired.
pub(crate) const FIRST_WINDOW: Duration = Duration::from_secs(5 * 60);

/// How long an unpaired phone may stay connected after the window closes,
/// or after it connects to a closed window. Pairing takes a moment after
/// the connection, and a phone mid-pairing must not be cut off.
pub(crate) const UNPAIRED_GRACE: Duration = Duration::from_secs(2);

/// A device the radio stack knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeviceInfo {
    pub address: PeerId,
    pub name: Option<String>,
    pub paired: bool,
    pub connected: bool,
}

/// What the policy needs of the radio stack. Errors are the stack's own
/// message.
pub(crate) trait AdapterOps: Send + Sync + 'static {
    /// Lets the adapter accept new pairings, or stops it.
    fn set_pairable(&self, pairable: bool) -> BoxFuture<'_, Result<(), String>>;
    /// Every device the stack remembers or sees.
    fn devices(&self) -> BoxFuture<'_, Result<Vec<DeviceInfo>, String>>;
    /// Drops a device's connection.
    fn disconnect(&self, address: PeerId) -> BoxFuture<'_, Result<(), String>>;
    /// Forgets a device, its pairing included.
    fn remove(&self, address: PeerId) -> BoxFuture<'_, Result<(), String>>;
}

/// Whether the pairing window is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairingState {
    Closed,
    /// Open until the instant.
    Open {
        until: Instant,
    },
}

/// A phone the player trusts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairedDevice {
    /// Its Bluetooth address, `AA:BB:CC:DD:EE:FF`.
    pub address: String,
    /// The name it gave, when it gave one.
    pub name: Option<String>,
}

/// Why a Bluetooth operation did not happen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BleError {
    /// The Bluetooth controller is not running: it is not configured, has
    /// not started, or could not (its status says why).
    #[error("Bluetooth is not running")]
    NotRunning,
    /// Not a Bluetooth address.
    #[error("{0:?} is not a Bluetooth address")]
    BadAddress(String),
    /// The radio stack refused.
    #[error("{0}")]
    Adapter(String),
}

#[derive(Default)]
struct Window {
    state: Option<Instant>,
    /// Counts openings, so a timer set by an earlier one can tell it was
    /// superseded.
    generation: u64,
    timer: Option<AbortHandle>,
}

/// The pairing policy over one adapter.
pub(crate) struct Pairing {
    adapter: Arc<dyn AdapterOps>,
    grace: Duration,
    window: Mutex<Window>,
}

impl Drop for Pairing {
    fn drop(&mut self) {
        if let Some(timer) = self.window.lock().timer.take() {
            timer.abort();
        }
    }
}

impl Pairing {
    /// Starts the policy: the window opens for `first_window` if no device
    /// is paired, and is closed otherwise. Phones already connected and
    /// unpaired are held to the same rule as ones that connect later.
    pub(crate) async fn start(
        adapter: Arc<dyn AdapterOps>,
        first_window: Duration,
        grace: Duration,
    ) -> Result<Arc<Pairing>, String> {
        let devices = adapter.devices().await?;
        let pairing = Arc::new(Pairing {
            adapter,
            grace,
            window: Mutex::new(Window::default()),
        });
        if devices.iter().any(|d| d.paired) {
            pairing.adapter.set_pairable(false).await?;
        } else {
            pairing.open(first_window).await?;
        }
        for device in devices.iter().filter(|d| d.connected && !d.paired) {
            pairing.police(device.address);
        }
        Ok(pairing)
    }

    /// Whether the window is open, and until when.
    pub(crate) fn state(&self) -> PairingState {
        match self.window.lock().state {
            Some(until) if until > Instant::now() => PairingState::Open { until },
            _ => PairingState::Closed,
        }
    }

    /// Opens the window for `duration`, or restarts it if it is open.
    pub(crate) async fn open(self: &Arc<Self>, duration: Duration) -> Result<(), String> {
        self.adapter.set_pairable(true).await?;
        let mut window = self.window.lock();
        window.generation += 1;
        let generation = window.generation;
        window.state = Some(Instant::now() + duration);
        if let Some(timer) = window.timer.take() {
            timer.abort();
        }
        let pairing = Arc::downgrade(self);
        window.timer = Some(
            tokio::spawn(async move {
                tokio::time::sleep(duration).await;
                if let Some(pairing) = pairing.upgrade() {
                    pairing.expire(generation).await;
                }
            })
            .abort_handle(),
        );
        Ok(())
    }

    /// Closes the window.
    pub(crate) async fn close(self: &Arc<Self>) -> Result<(), String> {
        {
            let mut window = self.window.lock();
            window.generation += 1;
            window.state = None;
            if let Some(timer) = window.timer.take() {
                timer.abort();
            }
        }
        self.closed().await
    }

    /// The window's time is up, unless it was opened again since.
    async fn expire(self: &Arc<Self>, generation: u64) {
        {
            let mut window = self.window.lock();
            if window.generation != generation {
                return;
            }
            window.state = None;
            // This is the timer's own task: it is done, not to be aborted.
            window.timer = None;
        }
        if let Err(e) = self.closed().await {
            tracing::warn!(error = %e, "Bluetooth pairing window not closed");
        }
    }

    /// What follows the window closing: the adapter stops accepting
    /// pairings, and phones connected without pairing are dropped.
    async fn closed(self: &Arc<Self>) -> Result<(), String> {
        let result = self.adapter.set_pairable(false).await;
        let pairing = Arc::downgrade(self);
        let grace = self.grace;
        tokio::spawn(async move {
            tokio::time::sleep(grace).await;
            if let Some(pairing) = pairing.upgrade() {
                pairing.sweep().await;
            }
        });
        result
    }

    /// Drops every connected, unpaired phone, if the window is closed.
    async fn sweep(&self) {
        if self.state() != PairingState::Closed {
            return;
        }
        let Ok(devices) = self.adapter.devices().await else {
            return;
        };
        for device in devices.iter().filter(|d| d.connected && !d.paired) {
            self.drop_unpaired(device.address).await;
        }
    }

    /// Holds a phone that has connected to the rule: once the grace is
    /// over, if the window is closed and it is still connected and still
    /// unpaired, it is disconnected. While the window is open it is left
    /// alone, and the window closing sweeps it up if it never paired.
    pub(crate) fn police(self: &Arc<Self>, address: PeerId) {
        let pairing = Arc::downgrade(self);
        let grace = self.grace;
        tokio::spawn(async move {
            tokio::time::sleep(grace).await;
            let Some(pairing) = pairing.upgrade() else {
                return;
            };
            if pairing.state() != PairingState::Closed {
                return;
            }
            let Ok(devices) = pairing.adapter.devices().await else {
                return;
            };
            if devices
                .iter()
                .any(|d| d.address == address && d.connected && !d.paired)
            {
                pairing.drop_unpaired(address).await;
            }
        });
    }

    /// Disconnects an unpaired phone, checking the window is still closed
    /// first: the listing it came from may be stale.
    async fn drop_unpaired(&self, address: PeerId) {
        if self.state() != PairingState::Closed {
            return;
        }
        match self.adapter.disconnect(address).await {
            Ok(()) => tracing::info!(
                device = %format_address(address),
                "Dropped an unpaired Bluetooth device: the pairing window is closed"
            ),
            Err(e) => tracing::debug!(error = %e, "Unpaired Bluetooth device not dropped"),
        }
    }

    /// The phones the player trusts.
    pub(crate) async fn paired_devices(&self) -> Result<Vec<PairedDevice>, String> {
        Ok(self
            .adapter
            .devices()
            .await?
            .into_iter()
            .filter(|d| d.paired)
            .map(|d| PairedDevice {
                address: format_address(d.address),
                name: d.name,
            })
            .collect())
    }

    /// Forgets a phone: it is disconnected and must pair again.
    pub(crate) async fn forget(&self, address: PeerId) -> Result<(), String> {
        self.adapter.remove(address).await
    }
}

/// How the rest of mtrack reaches the pairing policy of a running Bluetooth
/// controller: the web API takes it from the player. Cloning is cheap, and
/// every clone sees the controller as it is now: before it has started, or
/// after it stopped or failed, an operation says [`BleError::NotRunning`].
#[derive(Clone, Default)]
pub struct BleHandle {
    pairing: Arc<Mutex<Option<Arc<Pairing>>>>,
}

impl BleHandle {
    /// Points the handle at a running policy.
    pub(crate) fn attach(&self, pairing: Arc<Pairing>) {
        *self.pairing.lock() = Some(pairing);
    }

    /// Points the handle at nothing, as the controller stops.
    pub(crate) fn detach(&self) {
        *self.pairing.lock() = None;
    }

    fn running(&self) -> Result<Arc<Pairing>, BleError> {
        self.pairing.lock().clone().ok_or(BleError::NotRunning)
    }

    /// Whether the pairing window is open.
    pub fn pairing_state(&self) -> Result<PairingState, BleError> {
        Ok(self.running()?.state())
    }

    /// Opens the pairing window for `duration`.
    pub async fn open_pairing(&self, duration: Duration) -> Result<(), BleError> {
        self.running()?
            .open(duration)
            .await
            .map_err(BleError::Adapter)
    }

    /// Closes the pairing window.
    pub async fn close_pairing(&self) -> Result<(), BleError> {
        self.running()?.close().await.map_err(BleError::Adapter)
    }

    /// The phones the player trusts.
    pub async fn paired_devices(&self) -> Result<Vec<PairedDevice>, BleError> {
        self.running()?
            .paired_devices()
            .await
            .map_err(BleError::Adapter)
    }

    /// Forgets the phone at `address` (`AA:BB:CC:DD:EE:FF`): it is
    /// disconnected and must pair again, with the window open.
    pub async fn forget(&self, address: &str) -> Result<(), BleError> {
        let parsed = parse_address(address).ok_or_else(|| BleError::BadAddress(address.into()))?;
        self.running()?
            .forget(parsed)
            .await
            .map_err(BleError::Adapter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::eventually_async;

    const GRACE: Duration = Duration::from_millis(40);

    /// An adapter that remembers what it was asked.
    #[derive(Default)]
    struct Fake {
        pairable: Mutex<Vec<bool>>,
        devices: Mutex<Vec<DeviceInfo>>,
        disconnected: Mutex<Vec<PeerId>>,
        removed: Mutex<Vec<PeerId>>,
        failing: Mutex<bool>,
    }

    impl AdapterOps for Fake {
        fn set_pairable(&self, pairable: bool) -> BoxFuture<'_, Result<(), String>> {
            Box::pin(async move {
                if *self.failing.lock() {
                    return Err("adapter gone".into());
                }
                self.pairable.lock().push(pairable);
                Ok(())
            })
        }
        fn devices(&self) -> BoxFuture<'_, Result<Vec<DeviceInfo>, String>> {
            Box::pin(async move { Ok(self.devices.lock().clone()) })
        }
        fn disconnect(&self, address: PeerId) -> BoxFuture<'_, Result<(), String>> {
            Box::pin(async move {
                self.disconnected.lock().push(address);
                Ok(())
            })
        }
        fn remove(&self, address: PeerId) -> BoxFuture<'_, Result<(), String>> {
            Box::pin(async move {
                self.removed.lock().push(address);
                self.devices.lock().retain(|d| d.address != address);
                Ok(())
            })
        }
    }

    fn device(n: u8, paired: bool, connected: bool) -> DeviceInfo {
        DeviceInfo {
            address: [0, 0, 0, 0, 0, n],
            name: Some(format!("phone {n}")),
            paired,
            connected,
        }
    }

    async fn started(devices: Vec<DeviceInfo>) -> (Arc<Fake>, Arc<Pairing>) {
        let fake = Arc::new(Fake::default());
        *fake.devices.lock() = devices;
        let pairing = Pairing::start(fake.clone(), Duration::from_secs(300), GRACE)
            .await
            .expect("started");
        (fake, pairing)
    }

    async fn gone(fake: &Arc<Fake>, n: u8) -> bool {
        fake.disconnected.lock().contains(&[0, 0, 0, 0, 0, n])
    }

    #[tokio::test]
    async fn with_no_phone_paired_the_window_opens_for_the_first() {
        let (fake, pairing) = started(vec![device(1, false, false)]).await;
        assert_eq!(*fake.pairable.lock(), vec![true]);
        match pairing.state() {
            PairingState::Open { until } => {
                let left = until - Instant::now();
                assert!(left > Duration::from_secs(290) && left <= Duration::from_secs(300));
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn with_a_phone_paired_the_window_starts_closed() {
        let (fake, pairing) = started(vec![device(1, true, false)]).await;
        assert_eq!(*fake.pairable.lock(), vec![false]);
        assert_eq!(pairing.state(), PairingState::Closed);
    }

    #[tokio::test]
    async fn an_open_window_closes_by_itself() {
        let (fake, pairing) = started(vec![device(1, true, false)]).await;
        pairing.open(Duration::from_millis(60)).await.unwrap();
        assert!(matches!(pairing.state(), PairingState::Open { .. }));
        assert_eq!(*fake.pairable.lock(), vec![false, true]);

        eventually_async(
            || async { pairing.state() == PairingState::Closed },
            "the window never closed",
        )
        .await;
        eventually_async(
            || async { *fake.pairable.lock() == vec![false, true, false] },
            "the adapter stayed pairable",
        )
        .await;
    }

    #[tokio::test]
    async fn opening_again_restarts_the_window_and_the_old_timer_does_not_close_it() {
        let (fake, pairing) = started(vec![device(1, true, false)]).await;
        pairing.open(Duration::from_millis(60)).await.unwrap();
        pairing.open(Duration::from_secs(300)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(matches!(pairing.state(), PairingState::Open { .. }));
        assert_eq!(fake.pairable.lock().last(), Some(&true));
    }

    #[tokio::test]
    async fn closing_early_stops_the_timer_and_the_adapter() {
        let (fake, pairing) = started(vec![device(1, true, false)]).await;
        pairing.open(Duration::from_secs(300)).await.unwrap();
        pairing.close().await.unwrap();
        assert_eq!(pairing.state(), PairingState::Closed);
        assert_eq!(fake.pairable.lock().last(), Some(&false));
    }

    #[tokio::test]
    async fn an_unpaired_phone_is_dropped_after_the_grace_while_closed() {
        let (fake, pairing) = started(vec![device(1, true, false)]).await;
        fake.devices.lock().push(device(2, false, true));
        pairing.police([0, 0, 0, 0, 0, 2]);

        // Not at once.
        assert!(!gone(&fake, 2).await);
        eventually_async(|| gone(&fake, 2), "the unpaired phone was left connected").await;
    }

    #[tokio::test]
    async fn a_phone_that_paired_within_the_grace_is_kept() {
        let (fake, pairing) = started(vec![device(1, true, false)]).await;
        fake.devices.lock().push(device(2, false, true));
        pairing.police([0, 0, 0, 0, 0, 2]);
        fake.devices.lock()[1].paired = true;

        tokio::time::sleep(GRACE * 4).await;
        assert!(fake.disconnected.lock().is_empty());
    }

    #[tokio::test]
    async fn a_phone_that_left_within_the_grace_is_not_touched() {
        let (fake, pairing) = started(vec![device(1, true, false)]).await;
        fake.devices.lock().push(device(2, false, true));
        pairing.police([0, 0, 0, 0, 0, 2]);
        fake.devices.lock()[1].connected = false;

        tokio::time::sleep(GRACE * 4).await;
        assert!(fake.disconnected.lock().is_empty());
    }

    #[tokio::test]
    async fn while_the_window_is_open_an_unpaired_phone_is_left_alone() {
        let (fake, pairing) = started(vec![device(1, true, false)]).await;
        pairing.open(Duration::from_secs(300)).await.unwrap();
        fake.devices.lock().push(device(2, false, true));
        pairing.police([0, 0, 0, 0, 0, 2]);

        tokio::time::sleep(GRACE * 4).await;
        assert!(fake.disconnected.lock().is_empty());
    }

    #[tokio::test]
    async fn closing_the_window_drops_the_phone_that_never_paired() {
        let (fake, pairing) = started(vec![device(1, true, false)]).await;
        pairing.open(Duration::from_secs(300)).await.unwrap();
        fake.devices.lock().push(device(2, false, true));
        fake.devices.lock().push(device(3, true, true));

        pairing.close().await.unwrap();
        eventually_async(|| gone(&fake, 2), "the unpaired phone stayed").await;
        tokio::time::sleep(GRACE * 3).await;
        assert!(!gone(&fake, 3).await, "a paired phone was dropped");
        assert!(!gone(&fake, 1).await);
    }

    #[tokio::test]
    async fn phones_already_connected_at_start_are_held_to_the_rule() {
        // A phone paired and one that is not, both connected, window closed.
        let (fake, _pairing) = started(vec![device(1, true, true), device(2, false, true)]).await;
        eventually_async(|| gone(&fake, 2), "the unpaired phone stayed").await;
        assert!(!gone(&fake, 1).await);
    }

    #[tokio::test]
    async fn the_paired_list_and_forgetting() {
        let (fake, pairing) = started(vec![device(1, true, false), device(2, false, true)]).await;
        assert_eq!(
            pairing.paired_devices().await.unwrap(),
            vec![PairedDevice {
                address: "00:00:00:00:00:01".into(),
                name: Some("phone 1".into()),
            }]
        );
        pairing.forget([0, 0, 0, 0, 0, 1]).await.unwrap();
        assert_eq!(*fake.removed.lock(), vec![[0, 0, 0, 0, 0, 1]]);
        assert!(pairing.paired_devices().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failing_adapter_fails_the_start() {
        let fake = Arc::new(Fake::default());
        *fake.failing.lock() = true;
        let result = Pairing::start(fake, Duration::from_secs(1), GRACE).await;
        assert_eq!(result.err().as_deref(), Some("adapter gone"));
    }

    #[tokio::test]
    async fn the_handle_says_when_nothing_is_running_and_works_when_attached() {
        let handle = BleHandle::default();
        assert_eq!(handle.pairing_state(), Err(BleError::NotRunning));
        assert_eq!(
            handle.open_pairing(Duration::from_secs(1)).await,
            Err(BleError::NotRunning)
        );
        assert_eq!(handle.paired_devices().await, Err(BleError::NotRunning));
        assert_eq!(
            handle.forget("00:00:00:00:00:01").await,
            Err(BleError::NotRunning)
        );

        let (fake, pairing) = started(vec![device(1, true, false)]).await;
        handle.attach(pairing);
        let clone = handle.clone();
        assert_eq!(clone.pairing_state(), Ok(PairingState::Closed));
        clone.open_pairing(Duration::from_secs(60)).await.unwrap();
        assert!(matches!(
            handle.pairing_state(),
            Ok(PairingState::Open { .. })
        ));
        handle.close_pairing().await.unwrap();
        assert_eq!(handle.pairing_state(), Ok(PairingState::Closed));
        assert_eq!(handle.paired_devices().await.unwrap().len(), 1);
        assert_eq!(
            handle.forget("not an address").await,
            Err(BleError::BadAddress("not an address".into()))
        );
        handle.forget("00:00:00:00:00:01").await.unwrap();
        assert_eq!(fake.removed.lock().len(), 1);

        // Every clone sees the controller stop.
        handle.detach();
        assert_eq!(clone.pairing_state(), Err(BleError::NotRunning));
    }

    #[tokio::test]
    async fn an_adapter_error_reaches_the_caller() {
        let handle = BleHandle::default();
        let (fake, pairing) = started(vec![device(1, true, false)]).await;
        handle.attach(pairing);
        *fake.failing.lock() = true;
        assert_eq!(
            handle.open_pairing(Duration::from_secs(1)).await,
            Err(BleError::Adapter("adapter gone".into()))
        );
    }
}
