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
//! The gRPC controller's mDNS / DNS-SD advert, `_mtrack._tcp`, so a phone on
//! the player's network can find it without being told an address.
//!
//! In the process, with the `mdns-sd` crate, which runs its own thread: this
//! module never reads one of its channels on the async runtime. See
//! `design/wifi_discovery_design.md` §3.1.
//!
//! The pure decisions (names, TXT records, which addresses) are plain
//! functions; [`Advert`] is the running thing. It is started by the driver
//! once the listener is bound and unregisters, then shuts its daemon down,
//! when dropped, which is when the driver's task ends or is aborted.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use if_addrs::Interface;
use mdns_sd::{DaemonEvent, DaemonStatus, IfKind, IfPredicate, ServiceDaemon, ServiceInfo};
use tracing::{debug, info, warn};

/// The DNS-SD service type this advertises.
pub(super) const SERVICE_TYPE: &str = "_mtrack._tcp.local.";

/// What is appended to the host name to make the advert's host label.
const HOST_SUFFIX: &str = "-mtrack";

/// The longest a DNS label can be.
const MAX_LABEL: usize = 63;

/// How long a stop waits for the daemon to say it sent its goodbye, and then
/// that it has shut down. The daemon answers in milliseconds; this bounds a
/// wedged one.
const STOP_WAIT: Duration = Duration::from_millis(750);

/// Which of the machine's addresses the advert carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    /// No advert: the listener cannot be reached from another machine.
    Nowhere,
    /// Listening on every interface: the non-loopback addresses the machine
    /// has, followed as they change.
    EveryInterface,
    /// Listening on one address: that address only.
    Only(IpAddr),
}

/// Which addresses the advert carries for a listener bound to `bind`.
pub(super) fn scope_for(bind: IpAddr) -> Scope {
    if bind.is_loopback() {
        Scope::Nowhere
    } else if bind.is_unspecified() {
        Scope::EveryInterface
    } else {
        Scope::Only(bind)
    }
}

/// Whether an interface address of this machine is one the advert carries
/// when the listener is on every interface: not loopback, and not an IPv6
/// link-local address (`fe80::/10`), which names nothing without a scope a
/// phone does not have. Link-local IPv4 stays: two machines on one cable
/// have no other.
pub(super) fn is_advertised_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast(),
        IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && (ip.segments()[0] & 0xffc0) != 0xfe80
        }
    }
}

/// The host label the advert gives its addresses: `<hostname>-mtrack.local.`.
///
/// Never `<hostname>.local.`. That name is the machine's own, and avahi (the
/// Pi image's) or Bonjour already claims it. Claiming it again from here
/// makes the system responder see a second owner, log "Host name conflict"
/// and rename the machine (`mtrack` becomes `mtrack-2`), which breaks
/// `http://mtrack.local:8080`. Seen on the Raspberry Pi rig on 2026-11-14.
/// The phone never uses this label: it takes the instance name and the
/// addresses.
pub(super) fn host_label(hostname: &str) -> String {
    // Only the first label of a fully qualified name, and only what a host
    // label allows.
    let first = hostname.split('.').next().unwrap_or("");
    let mut label: String = first
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    label.truncate(MAX_LABEL - HOST_SUFFIX.len());
    let label = label.trim_matches('-');
    let label = if label.is_empty() { "mtrack" } else { label };
    format!("{label}{HOST_SUFFIX}.local.")
}

/// The TXT records: the mtrack version and, when a web UI is reachable, its
/// port. Nothing else; they are public to the network.
pub(super) fn txt_records(web_port: Option<u16>) -> HashMap<String, String> {
    let mut txt = HashMap::new();
    txt.insert("v".to_string(), env!("CARGO_PKG_VERSION").to_string());
    if let Some(port) = web_port {
        txt.insert("web".to_string(), port.to_string());
    }
    txt
}

/// A running advert. Dropping it takes the service down and stops the
/// daemon.
pub(super) struct Advert {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Advert {
    /// Fullname of the service instance, as the network sees it.
    #[cfg(test)]
    fn fullname(&self) -> &str {
        &self.fullname
    }
}

/// Starts the advert for a gRPC listener bound to `bound`, unless the config
/// turned it off or the listener is not reachable from the network. A
/// failure is logged once and gives `None`: it never stops the controller.
pub(super) fn start(
    advertise: bool,
    bound: SocketAddr,
    instance: &str,
    web_port: Option<u16>,
) -> Option<Advert> {
    let scope = scope_for(bound.ip());
    if !advertise || scope == Scope::Nowhere {
        return None;
    }
    // Nothing mDNS-related exists until here: no daemon, no socket.
    let daemon = match ServiceDaemon::new() {
        Ok(daemon) => daemon,
        Err(e) => {
            warn!(error = %e, "Could not start the mDNS advert; phones will not find this player by name");
            return None;
        }
    };
    start_on(daemon, scope, bound.port(), instance, web_port)
}

/// [`start`], on a daemon the caller made (a test uses another mDNS port).
fn start_on(
    daemon: ServiceDaemon,
    scope: Scope,
    port: u16,
    instance: &str,
    web_port: Option<u16>,
) -> Option<Advert> {
    match register(&daemon, scope, port, instance, web_port) {
        Ok(fullname) => {
            info!(service = %fullname, port, "Advertising the gRPC controller over mDNS");
            Some(Advert { daemon, fullname })
        }
        Err(e) => {
            warn!(error = %e, "Could not advertise over mDNS; phones will not find this player by name");
            // Nothing left to keep the daemon for.
            let _ = daemon.shutdown();
            None
        }
    }
}

fn register(
    daemon: &ServiceDaemon,
    scope: Scope,
    port: u16,
    instance: &str,
    web_port: Option<u16>,
) -> Result<String, mdns_sd::Error> {
    // What the daemon answers on, and so where it can put an address from.
    // Without this the crate follows every interface, loopback included
    // (`enable_addr_auto` alone advertised 127.0.0.1 and ::1 in the spike).
    let info = match scope {
        Scope::Nowhere => return Err(mdns_sd::Error::Msg("nothing to advertise".to_string())),
        Scope::EveryInterface => {
            daemon.disable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])?;
            let mut info = ServiceInfo::new(
                SERVICE_TYPE,
                instance,
                &host_label(instance),
                "",
                port,
                txt_records(web_port),
            )?
            .enable_addr_auto();
            info.set_interfaces(vec![IfKind::Predicate(IfPredicate::new(
                |interface: &Interface| is_advertised_address(interface.ip()),
            ))]);
            info
        }
        Scope::Only(ip) => {
            daemon.disable_interface(IfKind::All)?;
            daemon.enable_interface(IfKind::Addr(ip))?;
            let mut info = ServiceInfo::new(
                SERVICE_TYPE,
                instance,
                &host_label(instance),
                ip,
                port,
                txt_records(web_port),
            )?;
            info.set_interfaces(vec![IfKind::Addr(ip)]);
            info
        }
    };
    let fullname = info.get_fullname().to_string();
    // Events read on a thread of their own, which ends when the daemon does.
    let events = daemon.monitor()?;
    std::thread::Builder::new()
        .name("mdns-advert-events".to_string())
        .spawn(move || watch_events(events))
        .map_err(|e| mdns_sd::Error::Msg(format!("could not start the event thread: {e}")))?;
    daemon.register(info)?;
    Ok(fullname)
}

/// Reports the daemon's trouble: a name it had to change (another host
/// claimed ours) and its errors, each kind once, however often it retries.
fn watch_events(events: mdns_sd::Receiver<DaemonEvent>) {
    let mut warned_error = false;
    let mut warned_rename = false;
    while let Ok(event) = events.recv() {
        match event {
            DaemonEvent::Error(e) => {
                if !std::mem::replace(&mut warned_error, true) {
                    warn!(error = %e, "The mDNS advert hit an error; it keeps trying, and says no more of it");
                }
            }
            DaemonEvent::NameChange(change) => {
                if !std::mem::replace(&mut warned_rename, true) {
                    warn!(
                        original = %change.original,
                        new = %change.new_name,
                        "Another host on the network has the mDNS name this player advertises; it is advertised under a new one"
                    );
                }
            }
            other => debug!(?other, "mDNS advert event"),
        }
    }
}

impl Drop for Advert {
    fn drop(&mut self) {
        // Goodbye first, so phones drop the player now rather than when its
        // records expire, and before a restart's advert says hello: a late
        // goodbye would cancel it. The waits are bounded and the daemon
        // answers in milliseconds, so this does not hold up the runtime.
        match self.daemon.unregister(&self.fullname) {
            Ok(done) => {
                let _ = done.recv_timeout(STOP_WAIT);
            }
            Err(e) => debug!(error = %e, "Could not unregister the mDNS advert"),
        }
        match self.daemon.shutdown() {
            Ok(status) => {
                if !matches!(status.recv_timeout(STOP_WAIT), Ok(DaemonStatus::Shutdown)) {
                    debug!("The mDNS daemon did not confirm it shut down");
                }
            }
            Err(e) => debug!(error = %e, "Could not shut down the mDNS daemon"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn host_label_is_never_the_machines_own_name() {
        for hostname in [
            "mtrack",
            "btrackplayer",
            "BTrackPlayer",
            "mtrack-mtrack",
            "a",
            "x.local",
            "mtrack.example.com",
            "",
            "-",
            "my host_1",
            "ünïcode",
            &"long".repeat(40),
        ] {
            let label = host_label(hostname);
            assert_ne!(label, format!("{hostname}.local."), "{hostname:?}");
            assert_ne!(
                label.to_lowercase(),
                format!("{}.local.", hostname.to_lowercase()),
                "{hostname:?}"
            );
            assert!(label.ends_with("-mtrack.local."), "{label}");
            let host = label.strip_suffix(".local.").unwrap();
            assert!(host.len() <= MAX_LABEL, "{label}");
            assert!(
                host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "{label}"
            );
        }
        assert_eq!(host_label("mtrack"), "mtrack-mtrack.local.");
        assert_eq!(host_label("btrackplayer"), "btrackplayer-mtrack.local.");
        assert_eq!(host_label("pi.lan"), "pi-mtrack.local.");
        assert_eq!(host_label(""), "mtrack-mtrack.local.");
    }

    #[test]
    fn txt_records_hold_the_version_and_the_web_port_only() {
        let txt = txt_records(None);
        assert_eq!(txt.len(), 1);
        assert_eq!(txt["v"], env!("CARGO_PKG_VERSION"));

        let txt = txt_records(Some(8080));
        assert_eq!(txt.len(), 2);
        assert_eq!(txt["v"], env!("CARGO_PKG_VERSION"));
        assert_eq!(txt["web"], "8080");
    }

    #[test]
    fn scope_follows_the_bind_address() {
        assert_eq!(scope_for(ip("0.0.0.0")), Scope::EveryInterface);
        assert_eq!(scope_for(ip("::")), Scope::EveryInterface);
        assert_eq!(scope_for(ip("127.0.0.1")), Scope::Nowhere);
        assert_eq!(scope_for(ip("127.1.2.3")), Scope::Nowhere);
        assert_eq!(scope_for(ip("::1")), Scope::Nowhere);
        assert_eq!(
            scope_for(ip("192.168.1.20")),
            Scope::Only(ip("192.168.1.20"))
        );
        assert_eq!(scope_for(ip("2001:db8::7")), Scope::Only(ip("2001:db8::7")));
    }

    #[test]
    fn loopback_and_scopeless_addresses_are_not_advertised() {
        assert!(!is_advertised_address(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(!is_advertised_address(ip("127.0.1.1")));
        assert!(!is_advertised_address(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(!is_advertised_address(ip("fe80::1")));
        assert!(!is_advertised_address(ip("0.0.0.0")));
        assert!(!is_advertised_address(ip("224.0.0.251")));
        assert!(is_advertised_address(ip("192.168.1.20")));
        assert!(is_advertised_address(ip("10.0.0.5")));
        assert!(is_advertised_address(ip("169.254.3.4")));
        assert!(is_advertised_address(ip("2001:db8::7")));
        assert!(is_advertised_address(ip("fd00::7")));
    }

    #[test]
    fn nothing_starts_when_off_or_on_loopback() {
        let any: SocketAddr = "0.0.0.0:43234".parse().unwrap();
        let loopback: SocketAddr = "127.0.0.1:43234".parse().unwrap();
        // Off: no daemon, so no socket and no thread.
        assert!(start(false, any, "mtrack", None).is_none());
        assert!(start(true, loopback, "mtrack", None).is_none());
        assert!(start(true, "[::1]:43234".parse().unwrap(), "mtrack", None).is_none());
    }

    /// Finds the advert again by browsing for it with the same crate. Both
    /// daemons use a private mDNS port, so avahi or Bonjour on the machine is
    /// not involved, but multicast still has to work on a real interface:
    /// not so on every CI runner or container, so this is run by hand
    /// (`cargo test advert -- --ignored`).
    #[test]
    #[ignore = "needs working multicast on a non-loopback interface, which CI runners and containers do not reliably have"]
    fn the_advert_can_be_browsed_and_goes_away_when_dropped() {
        const MDNS_TEST_PORT: u16 = 15353;
        let name = format!("advert-test-{}", std::process::id());

        // Start, stop, start again: a restart leaves one advert, not two.
        for _ in 0..2 {
            let daemon = ServiceDaemon::new_with_port(MDNS_TEST_PORT).unwrap();
            let advert = start_on(daemon, Scope::EveryInterface, 43999, &name, Some(8080))
                .expect("advertised");
            let fullname = advert.fullname().to_string();

            let browser = ServiceDaemon::new_with_port(MDNS_TEST_PORT).unwrap();
            let found = browser.browse(SERVICE_TYPE).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let resolved = loop {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                match found.recv_timeout(left) {
                    Ok(mdns_sd::ServiceEvent::ServiceResolved(r)) if r.fullname == fullname => {
                        break r
                    }
                    Ok(_) => {}
                    Err(e) => panic!("the advert was not found: {e}"),
                }
            };
            assert_eq!(resolved.get_port(), 43999);
            assert_eq!(
                resolved.get_property_val_str("v"),
                Some(env!("CARGO_PKG_VERSION"))
            );
            assert_eq!(resolved.get_property_val_str("web"), Some("8080"));
            assert_eq!(resolved.host, host_label(&name));
            assert!(!resolved.get_addresses().is_empty());
            assert!(
                resolved.get_addresses().iter().all(|a| {
                    let ip = a.to_ip_addr();
                    !ip.is_loopback()
                }),
                "{:?}",
                resolved.get_addresses()
            );

            drop(advert);
            let gone = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                let left = gone.saturating_duration_since(std::time::Instant::now());
                match found.recv_timeout(left) {
                    Ok(mdns_sd::ServiceEvent::ServiceRemoved(_, n)) if n == fullname => break,
                    Ok(_) => {}
                    Err(e) => panic!("the advert was not withdrawn: {e}"),
                }
            }
            let _ = browser.shutdown();
        }
    }
}
