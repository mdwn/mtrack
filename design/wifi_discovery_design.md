# Finding the player over Wi-Fi

*Design doc, draft 1 — 2026-11-14. Follows `design/remote_phase3_design.md`
§4.3, which left mDNS browsing "for after the first two have been used".
Mike has asked for it: "Let's go for wifi in the app without Bluetooth. I'm
specifically interested in mdns and scanning like mixing station does
today."*

## 1. What is asked for

Open the app on a phone that is on the player's network, see the player in
a list, tap it. No Bluetooth, no pairing, no typing an address.

Mixing Station is the model: its start screen searches the network, lists
the mixers it finds as they answer, and keeps a manual address field for the
network where the search finds nothing.

## 2. What is there today

- **Enter an address** on the connect screen takes a host name or an IP and
  makes a Wi-Fi-only player. So Wi-Fi without Bluetooth already works for
  someone who knows the address.
- A phone connected by Bluetooth asks the player for its addresses
  (`GetNetworkInfo`) and uses Wi-Fi from then on.
- The Pi image runs avahi, so the *host* answers to `mtrack.local`. Nothing
  advertises the *player*: there is no service to browse for, and on a
  desktop install there is no avahi at all.
- Typing `mtrack.local` in the app is not known to work on Android, whose
  resolver does not always do mDNS.

What is missing is only the finding. The connection, the status stream, the
fallback to Bluetooth and the remembered player are built.

## 3. Two ways to find a player

Neither covers every network, which is why Mixing Station has both a search
and an address field. The proposal is both, plus the field that exists.

### 3.1 mDNS: the player says it is there

The player advertises a DNS-SD service, `_mtrack._tcp`, on the port its gRPC
controller listens on. The app browses for that type and lists what answers.

- Instance name: the host name (`mtrack`, `btrackplayer`).
- TXT records, small and not secret: `v` (the mtrack version) and `web`
  (the web UI's port, so a later desktop tool could open it). Nothing about
  songs, the lock or paired phones.
- Advertised only when the gRPC controller is bound to something other than
  loopback, and on those interfaces only. A player whose gRPC port a phone
  cannot reach should not be listed.
- On by default, with a switch on the gRPC controller to turn it off
  (`advertise: false`).

**How the player advertises.** Three ways:

| | For | Against |
|---|---|---|
| **A. In the process**, with the `mdns-sd` crate | Works on every install, Pi image or not, Linux and macOS. The port and the bind address come from the config, so the advert is never wrong. No daemon to depend on | A new dependency (it brings `if-addrs` and `socket2`, both already in the lockfile). Shares UDP 5353 with avahi on the Pi image, which the crate is built to do but which has to be seen working |
| B. An avahi service file in the Pi image | No code | Pi image only. The port is written twice, so a changed config advertises the wrong one. Nothing on a desktop or a hand install |
| C. Ask avahi over D-Bus | The system's one responder does the work | Linux with avahi only; macOS needs a second implementation; more code than A |

Recommended: **A**. The sandboxed unit already allows `AF_INET`, `AF_INET6`
and `AF_NETLINK`, which is what it needs.

**Tried on the Pi 4 rig (2026-11-14)**, with a 40-line program using
`mdns-sd` 0.21, run in a transient unit with the service's sandbox, beside
the image's avahi:

- It works: a question for `_mtrack._tcp` from another socket was answered
  with the instance, the port and the TXT records.
- **The advert must not use the host's own `.local` name.** The first run
  named its host `mtrack-harness.local`, which avahi owns. avahi saw a
  second claimant, logged "Host name conflict", and renamed the machine to
  `mtrack-harness-2.local` until it was restarted. That would break
  `http://mtrack.local:8080` on every Pi image.
- With a host name of its own (`mtrack-harness-player.local`) there was no
  conflict over 30 seconds and avahi kept its name.

So the player's advert carries a host label nothing else claims,
`<hostname>-mtrack.local`, with the addresses of the interfaces gRPC is
bound to. The app never shows it: it uses the instance name and the
addresses. D1's test on the rig repeats this check with the real build, for
longer, and across an avahi restart.

**How the app browses.** Through the platform's own browser (`NsdManager`
on Android, Bonjour on iOS) by way of a Flutter plugin (`nsd` or `bonsoir`;
which one is settled by trying both on the Pixel). Not a pure-Dart mDNS
client: on iOS that needs a multicast entitlement Apple grants by
application, and the platform browser needs none.

- Android: no new permission on today's versions. Whether a newer Android
  asks for a local-network permission is in §8.
- iOS: `NSBonjourServices` lists `_mtrack._tcp` in `Info.plist`; the local
  network prompt the app already declares is shown the first time.

### 3.2 Scanning: the app goes looking

mDNS fails on networks that drop multicast, which some access points do
while passing ordinary traffic between clients. For those, the app looks
itself.

Two ways to look:

| | For | Against |
|---|---|---|
| **A. Sweep the subnet.** Try the gRPC port on every address of the phone's own network, a few dozen at a time, and ask each one that answers whether it is a player | Nothing new on the player. Finds players that predate this work. Passes anything that passes the connection itself | Up to 254 connection attempts; on a network someone else runs, that looks like a port scan. Finds only the default port |
| B. A broadcast probe. The app sends one UDP packet to the broadcast address; players answer. This is what Mixing Station does for most mixers | One packet. Finds any port | A new listener on the player, a protocol only mtrack speaks, and it is blocked by much of what blocks mDNS, which it duplicates |

Recommended: **A**, with these limits:

- Only when asked: a **Scan this network** button under the list. mDNS runs
  by itself while the connect screen is open; the sweep does not.
- Only the phone's own Wi-Fi network, only when its address is private
  (10/8, 172.16/12, 192.168/16), and never over mobile data.
- A /24 around the phone's address: at most 254 hosts, about 32 at a time,
  about 400 ms each, so a full sweep is three to four seconds. A bigger
  network is swept only in the phone's own /24.
- Port 43234, and the port of a remembered player if it differs.
- An address that accepts the connection is asked for the standard gRPC
  health check of `player.v1.PlayerService`, then `GetNetworkInfo` for its
  name. Anything that is not a player is dropped without a word.

### 3.3 The address field

Stays as it is, below the list, for everything else: another subnet, a VPN,
a port forward.

## 4. The connect screen

One list, filled from three sources as they answer:

```
Players
  btrackplayer        Wi-Fi          192.168.1.111
  mtrack              Wi-Fi          192.168.1.216
  mtrack              Bluetooth
                                     Searching…
[ Scan this network ]   [ Enter an address ]
```

- A row found by mDNS or by the sweep connects on a tap and is remembered as
  a Wi-Fi player. No pairing, as with a typed address today.
- A row found by Bluetooth pairs as today.
- The same player found both ways is two rows at first. Joining them needs
  the player to say the same name in both places; today the Bluetooth
  advert carries a name of its own. Left as a later nicety (§7, D4).
- Bluetooth off or refused no longer leaves the screen empty: the Wi-Fi rows
  and both buttons are there regardless. Today the screen is about Bluetooth
  and the address is a side door.
- No network at all (mobile data only): the Wi-Fi part says so in a line.

## 5. Staying found

A Wi-Fi player is remembered by the addresses it had. DHCP can move it.

- A player found by mDNS is also remembered by its instance name. When none
  of its addresses answers, the app browses again for that name and takes
  the address it now has, before giving up.
- A player found by the sweep or typed by IP has only the address. When it
  stops answering, the connect screen is where the person ends up, and the
  list finds it again.

## 6. Who can do what

Unchanged, and worth saying plainly because finding gets easier:

- Over Wi-Fi there is no sign-in. Anyone on the player's network can use the
  app against it, as they can the web UI. That was decided in phase 3: the
  player's network is meant to be its own, not the venue's.
- The lock applies on Wi-Fi as everywhere.
- The advert tells the network a player is there, its host name, its
  version and two port numbers. A port scan already finds all of that.
  Someone who does not want it can turn it off, and can bind gRPC to
  loopback (#770), which also turns the advert off.
- The sweep makes the app the thing that scans. It is held to the phone's
  own private /24 and to a button for that reason.

## 7. Steps

| Step | What | Size |
|---|---|---|
| D1 | The player advertises `_mtrack._tcp`. Config switch, docs, the systemd unit checked on the rig beside avahi, seen with `avahi-browse` | Small |
| D2 | The app browses. The connect screen becomes one list with Wi-Fi rows; Bluetooth off no longer empties it | Medium: the screen is reworked |
| D3 | **Scan this network** | Small to medium |
| D4 | Re-finding a remembered player by name (§5). Joining Wi-Fi and Bluetooth rows of one player, if wanted | Small |

D1 and D2 together are the feature. D3 is the fallback. Each is a PR that
leaves the app working.

## 8. What is not known

- **Android's browser with a VPN up.** The Pixel runs Tailscale as a VPN.
  `NsdManager` is the system's and should not care, but it has not been
  seen. The sweep's sockets go where the app's sockets go, which with the
  VPN up worked for a typed address.
- **Newer Android and local network access.** Android has been moving
  toward a permission for reaching local addresses. Whether the Pixel's
  version asks for one, and for which of browsing and connecting, is
  settled by running D2 on it.
- **`mdns-sd` beside avahi** for longer than the half minute tried (§3.1),
  across an avahi restart and a network change, and on macOS beside
  Bonjour, where the same care about the host's name applies.
- **iOS.** Nothing here has run on an iPhone; the local network prompt and
  a refusal of it are untested, as they are for Wi-Fi today.
- **The phone's subnet.** Dart gives the phone's addresses but not the
  network's size, so the sweep assumes /24. On a /16 it sees a corner.

## 9. Decisions

1. **How the player advertises** (§3.1). Recommended: in the process, with
   `mdns-sd`.
2. **Advertising is on by default**, with a switch. Recommended: yes.
3. **How the app scans** (§3.2). Recommended: a subnet sweep, not a new
   broadcast protocol.
4. **The sweep runs on a button**, not by itself. Recommended: yes. The
   other choice is Mixing Station's, which searches as it opens.
5. **One list on the connect screen** for Wi-Fi and Bluetooth (§4).
   Recommended: yes.
