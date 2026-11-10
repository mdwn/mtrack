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

//! The BlueZ side of the Bluetooth controller: the one place that touches
//! the radio stack. It registers the pairing agent, the GATT service and
//! the advertisement, forwards what phones write to the
//! [`Link`](super::session::Link), and tells the link and the pairing
//! policy about devices coming and going. Everything it decides is made in
//! the modules beside it, which are tested without a radio.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::time::Instant;

use bluer::adv::Advertisement;
use bluer::agent::{Agent, ReqError as AgentError};
use bluer::gatt::local::{
    Application, Characteristic, CharacteristicNotifier, CharacteristicNotify,
    CharacteristicNotifyMethod, CharacteristicRead, CharacteristicWrite, CharacteristicWriteMethod,
    ReqError, Service,
};
use bluer::{Adapter, AdapterEvent, AdapterProperty, Address, DeviceEvent, DeviceProperty, Uuid};
use futures_util::future::BoxFuture;
use futures_util::{FutureExt, StreamExt};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::player::Player;

use super::dispatch::Dispatcher;
use super::notify::{self, Greeting, Notifier};
use super::pairing::{
    AdapterOps, BleHandle, DeviceInfo, Pairing, PairingState, FIRST_WINDOW, UNPAIRED_GRACE,
};
use super::session::{Link, PeerId, PUMP_DEPTH};
use super::status;
use super::{REQUEST_UUID, RESPONSE_UUID, SERVICE_UUID, STATUS_UUID};

/// Why the adapter is unusable, with the likely fix, as an error.
fn unusable(what: &str, e: &bluer::Error) -> io::Error {
    io::Error::other(format!(
        "Bluetooth is unavailable: {what} ({e}). Check that the adapter exists, that \
         bluetooth.service is running, and that the radio is not soft-blocked \
         (`rfkill unblock bluetooth`)"
    ))
}

/// A task that ends when this does: dropping the driver's future must not
/// leave its tasks running.
struct Abort(AbortHandle);

impl Drop for Abort {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Spawns `task`, ending with the returned guard.
fn spawn_guarded(task: impl std::future::Future<Output = ()> + Send + 'static) -> Abort {
    Abort(tokio::spawn(task).abort_handle())
}

/// On the driver ending, however it does: the handle stops pointing at it
/// and the adapter stops accepting pairings. The registrations (agent,
/// application, advertisement) are dropped with the driver's own locals.
struct Cleanup {
    adapter: Adapter,
    handle: BleHandle,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        self.handle.detach();
        // A drop can happen as the runtime shuts down, with nothing to run
        // this on; the adapter is then left as it is.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let adapter = self.adapter.clone();
            runtime.spawn(async move {
                if let Err(e) = adapter.set_pairable(false).await {
                    tracing::debug!(error = %e, "Bluetooth adapter left pairable");
                }
            });
        }
    }
}

/// The pairing policy's view of BlueZ.
struct Bluez(Adapter);

impl AdapterOps for Bluez {
    fn set_pairable(&self, pairable: bool) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.0
                .set_pairable(pairable)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn devices(&self) -> BoxFuture<'_, Result<Vec<DeviceInfo>, String>> {
        Box::pin(async move {
            let mut devices = Vec::new();
            for address in self.0.device_addresses().await.map_err(|e| e.to_string())? {
                let Ok(device) = self.0.device(address) else {
                    continue;
                };
                devices.push(DeviceInfo {
                    address: address.0,
                    name: match device.name().await {
                        Ok(Some(name)) => Some(name),
                        _ => device.alias().await.ok(),
                    },
                    paired: device.is_paired().await.unwrap_or(false),
                    connected: device.is_connected().await.unwrap_or(false),
                });
            }
            Ok(devices)
        })
    }

    fn disconnect(&self, address: PeerId) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let device = self.0.device(Address(address)).map_err(|e| e.to_string())?;
            device.disconnect().await.map_err(|e| e.to_string())
        })
    }

    fn remove(&self, address: PeerId) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.0
                .remove_device(Address(address))
                .await
                .map_err(|e| e.to_string())
        })
    }
}

/// One phone's notification session.
struct Session(CharacteristicNotifier);

impl Notifier for Session {
    fn is_stopped(&self) -> bool {
        self.0.is_stopped()
    }

    fn notify(&mut self, value: Vec<u8>) -> BoxFuture<'_, bool> {
        Box::pin(async move { self.0.notify(value).await.is_ok() })
    }
}

/// A notify characteristic whose sessions go to `sessions`.
fn notify_method(sessions: mpsc::UnboundedSender<Session>) -> CharacteristicNotify {
    CharacteristicNotify {
        notify: true,
        method: CharacteristicNotifyMethod::Fun(Box::new(move |notifier| {
            let _ = sessions.send(Session(notifier));
            async {}.boxed()
        })),
        ..Default::default()
    }
}

/// Runs the Bluetooth controller until it fails: it returns the error that
/// ended it, and never returns `Ok`. Dropping its future unregisters
/// everything and leaves the adapter unpairable.
pub(super) async fn run(
    name: Option<String>,
    player: Arc<Player>,
    handle: BleHandle,
) -> io::Result<()> {
    let session = bluer::Session::new()
        .await
        .map_err(|e| unusable("bluetoothd cannot be reached", &e))?;
    let adapter = session
        .default_adapter()
        .await
        .map_err(|e| unusable("there is no Bluetooth adapter", &e))?;
    if !adapter
        .is_powered()
        .await
        .map_err(|e| unusable("the adapter cannot be read", &e))?
    {
        adapter.set_powered(true).await.map_err(|e| {
            unusable(
                &format!("adapter {} cannot be powered on", adapter.name()),
                &e,
            )
        })?;
    }
    // BlueZ closes pairing on a timer of its own unless told not to; the
    // window is ours to close.
    adapter
        .set_pairable_timeout(0)
        .await
        .map_err(|e| unusable("the pairing timeout cannot be set", &e))?;

    let _cleanup = Cleanup {
        adapter: adapter.clone(),
        handle: handle.clone(),
    };

    // The player has no screen or keys, so nobody confirms a pairing at
    // it: the agent says yes while the pairing window is open and no
    // otherwise. BlueZ asks the agent to authorize an incoming pairing
    // even when there is no code to compare, and an agent with no answer
    // refuses it.
    let _agent = session
        .register_agent(Agent {
            request_default: true,
            request_confirmation: Some(Box::new({
                let handle = handle.clone();
                move |request| answer_pairing(handle.clone(), request.device).boxed()
            })),
            request_authorization: Some(Box::new({
                let handle = handle.clone();
                move |request| answer_pairing(handle.clone(), request.device).boxed()
            })),
            ..Default::default()
        })
        .await
        .map_err(|e| unusable("the pairing agent cannot be registered", &e))?;

    let pairing = Pairing::start(
        Arc::new(Bluez(adapter.clone())),
        FIRST_WINDOW,
        UNPAIRED_GRACE,
    )
    .await
    .map_err(|e| io::Error::other(format!("Bluetooth pairing could not start: {e}")))?;
    handle.attach(pairing.clone());

    // The link, and the pumps that notify what it produces.
    let (responses, response_frames) = mpsc::channel(PUMP_DEPTH);
    let (status_out, status_frames) = mpsc::channel(PUMP_DEPTH);
    let link = Link::new(Dispatcher::new(player.clone()), responses, status_out);
    let (response_sessions, response_rx) = mpsc::unbounded_channel();
    let (status_sessions, status_rx) = mpsc::unbounded_channel();
    let _response_pump = spawn_guarded(notify::run(response_rx, response_frames, None));
    let greeting: Greeting = {
        let link = link.clone();
        Box::new(move || link.status_chunks())
    };
    let _status_pump = spawn_guarded(notify::run(status_rx, status_frames, Some(greeting)));
    let _status_poll = spawn_guarded({
        let link = link.clone();
        status::poll(player, move |status| {
            link.publish_status(Instant::now(), status)
        })
    });

    let request = {
        let link = link.clone();
        Characteristic {
            uuid: Uuid::from_u128(REQUEST_UUID),
            write: Some(CharacteristicWrite {
                write: true,
                write_without_response: true,
                encrypt_write: true,
                method: CharacteristicWriteMethod::Fun(Box::new(move |value, request| {
                    let link = link.clone();
                    async move {
                        // Long writes (a nonzero offset) are not part of
                        // the protocol: every chunk is a whole write.
                        if request.offset != 0 {
                            return Err(ReqError::InvalidOffset);
                        }
                        link.submit(request.device_address.0, request.mtu, &value);
                        Ok(())
                    }
                    .boxed()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }
    };
    let response = Characteristic {
        uuid: Uuid::from_u128(RESPONSE_UUID),
        notify: Some(notify_method(response_sessions)),
        ..Default::default()
    };
    let status_characteristic = {
        let link = link.clone();
        Characteristic {
            uuid: Uuid::from_u128(STATUS_UUID),
            read: Some(CharacteristicRead {
                read: true,
                encrypt_read: true,
                fun: Box::new(move |request| {
                    let link = link.clone();
                    async move {
                        Ok(link.status_read(
                            request.device_address.0,
                            request.mtu,
                            usize::from(request.offset),
                        ))
                    }
                    .boxed()
                }),
                ..Default::default()
            }),
            notify: Some(notify_method(status_sessions)),
            ..Default::default()
        }
    };
    let _application = adapter
        .serve_gatt_application(Application {
            services: vec![Service {
                uuid: Uuid::from_u128(SERVICE_UUID),
                primary: true,
                characteristics: vec![request, response, status_characteristic],
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .map_err(|e| unusable("the GATT service cannot be registered", &e))?;

    let advertised = name.unwrap_or_else(crate::config::resolve_hostname);
    let _advertisement = adapter
        .advertise(Advertisement {
            service_uuids: [Uuid::from_u128(SERVICE_UUID)].into_iter().collect(),
            discoverable: Some(true),
            local_name: Some(advertised.clone()),
            ..Default::default()
        })
        .await
        .map_err(|e| unusable("the advertisement cannot be registered", &e))?;
    tracing::info!(
        name = %advertised,
        adapter = %adapter.name(),
        "Bluetooth controller advertising"
    );

    // Devices coming and going, until the adapter does.
    let mut watches: HashMap<Address, Abort> = HashMap::new();
    let mut events = adapter
        .events()
        .await
        .map_err(|e| unusable("adapter events cannot be read", &e))?;
    let known = adapter
        .device_addresses()
        .await
        .map_err(|e| unusable("devices cannot be listed", &e))?;
    let watch = |address: Address| {
        Abort(
            tokio::spawn(watch_device(
                adapter.clone(),
                address,
                link.clone(),
                pairing.clone(),
            ))
            .abort_handle(),
        )
    };
    for address in known {
        watches.insert(address, watch(address));
    }
    while let Some(event) = events.next().await {
        match event {
            AdapterEvent::DeviceAdded(address) => {
                watches.insert(address, watch(address));
            }
            AdapterEvent::DeviceRemoved(address) => {
                watches.remove(&address);
                link.on_disconnect(address.0);
            }
            AdapterEvent::PropertyChanged(AdapterProperty::Powered(false)) => {
                return Err(io::Error::other(
                    "Bluetooth adapter was powered off; if it was soft-blocked, \
                     `rfkill unblock bluetooth`, then restart the controllers",
                ));
            }
            AdapterEvent::PropertyChanged(_) => {}
        }
    }
    Err(io::Error::other(
        "Bluetooth stopped: bluetoothd went away or the adapter was removed",
    ))
}

/// Answers BlueZ's question whether `device` may pair: yes while the
/// pairing window is open.
async fn answer_pairing(handle: BleHandle, device: Address) -> Result<(), AgentError> {
    if matches!(handle.pairing_state(), Ok(PairingState::Open { .. })) {
        tracing::info!(%device, "Bluetooth device pairing: the pairing window is open");
        Ok(())
    } else {
        tracing::info!(%device, "Refused a Bluetooth pairing: the pairing window is closed");
        Err(AgentError::Rejected)
    }
}

/// Follows one device's connection: the link learns of it, and the pairing
/// policy holds it to account.
async fn watch_device(
    adapter: Adapter,
    address: Address,
    link: Arc<Link<Dispatcher>>,
    pairing: Arc<Pairing>,
) {
    let Ok(device) = adapter.device(address) else {
        return;
    };
    // Subscribed before the current state is read, so a change in between
    // is not missed.
    let Ok(events) = device.events().await else {
        return;
    };
    let mut events = std::pin::pin!(events);
    if device.is_connected().await.unwrap_or(false) {
        link.on_connect(address.0);
        pairing.police(address.0);
    }
    while let Some(DeviceEvent::PropertyChanged(property)) = events.next().await {
        match property {
            DeviceProperty::Connected(true) => {
                tracing::info!(device = %address, "Bluetooth device connected");
                link.on_connect(address.0);
                pairing.police(address.0);
            }
            DeviceProperty::Connected(false) => {
                tracing::info!(device = %address, "Bluetooth device disconnected");
                link.on_disconnect(address.0);
            }
            DeviceProperty::Paired(true) => {
                tracing::info!(device = %address, "Bluetooth device paired");
            }
            _ => {}
        }
    }
}
