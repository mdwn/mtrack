# Bluetooth: a phone remote that needs no network

*Design doc, draft 1 — 2026-11-09. Follows a spike on the Pi 4 rig the same
day. Decided with Mike: the goal is remote control from a phone where the Pi is
on no network, with headless setup as the follow-on; iOS and Android both; the
app is written in Flutter. This doc fixes the architecture and the build order
and records the decisions (§13).*

## 1. Why

mtrack's remote surfaces all ride on IP: the web UI, gRPC, OSC, MCP. At a
venue the Pi is often on no network at all, and the phone in the player's
pocket cannot reach it. The existing answers are a travel router or a laptop on
a cable, and both are more kit to carry and to fail.

Every Pi mtrack runs on has a Bluetooth radio. A phone can talk to it with no
infrastructure, without leaving whatever wifi or cellular it is on, and it
reconnects by itself when it comes back in range.

The web UI cannot be the client. Web Bluetooth does not exist on iOS and needs
a secure context on Android, which `http://mtrack.local:8080` is not, and a
page that loaded from the Pi already had a network. So this is a native app.

## 2. Goals and non-goals

**Goals.**

- A phone app, iOS and Android, that controls a running player over Bluetooth
  Low Energy: transport, playlist, section loops, status.
- No second API. The app speaks the `PlayerService` protobuf messages that
  gRPC already carries.
- A stranger in the room cannot press Stop. Only phones the owner has paired
  can control the player.
- Nothing changes for an install that does not turn it on, and the macOS build
  is unaffected.

**Non-goals.**

- **Not the web UI over Bluetooth.** BLE is a control channel. Song import,
  waveforms, the lighting editor and config editing stay on the web UI.
- **No Bluetooth audio.** The latency and jitter are wrong for click and
  tracks.
- **BLE MIDI controllers are a separate, smaller question** (§10). They need
  no app and probably no mtrack code.
- **Headless setup (wifi credentials over BLE) is phase B3**, named here so the
  protocol leaves room for it, and designed in its own round.

## 3. What the spike showed

A throwaway GATT server (`bluer` 0.17, about 200 lines) ran on the rig and was
driven from nRF Connect on an Android phone, next to the rig.

| Question | Result |
|---|---|
| Does BlueZ come with the image? | Yes: BlueZ 5.82, `bluetoothd` running. |
| Does it work inside the service sandbox? | Yes, unchanged. It ran as the `mtrack` user under every restriction `mtrack.service` carries, with no extra group. |
| Is the adapter usable out of the box? | **No.** The image ships Bluetooth soft-blocked (saved rfkill state). Root has to unblock it; the `mtrack` user cannot. |
| Command latency | `play` and `stop` took 21–22 ms from the BLE write arriving to mtrack finishing, and that includes spawning the `mtrack` CLI. In-process it will be less. |
| Notification stream | 1,694 notifications at 5 Hz over about five and a half minutes; none took longer than 50 ms to hand to the stack. |
| Payload size | The phone negotiated an ATT MTU of 517. |

Not measured yet: range, behaviour through a room of people, reconnect after
walking away, and anything at all on iOS.

## 4. Architecture

```
 phone app (Flutter)                         Pi
 ┌───────────────────────┐   BLE GATT   ┌───────────────────────────────┐
 │ screens               │              │ bluetoothd (BlueZ)            │
 │ player client         │◄────────────►│   ▲ D-Bus                     │
 │ frame codec + protobuf│              │ mtrack: controller/ble.rs     │
 └───────────────────────┘              │   frame codec                 │
                                        │   → PlayerService (in-process)│
                                        └───────────────────────────────┘
```

**Player side.** A fifth controller driver, `src/controller/ble.rs`, beside
gRPC, OSC, MIDI and MCP, implementing the same `Driver` trait. It registers one
GATT service and an advertisement with BlueZ over D-Bus and holds the same
`PlayerServer` the gRPC driver serves, calling its methods directly. Nothing
goes over a loopback socket.

**Library.** `bluer`, the BlueZ project's Rust bindings, on tokio. It links the
system `libdbus-1`, which every Debian install already has; the release
workflow builds arm64 on a native arm runner, so this is one more `apt-get`
package (`libdbus-1-dev`), not a cross-compile problem. The dependency sits
behind `cfg(target_os = "linux")` and a cargo feature.

**The codec is separate from the transport.** Framing and dispatch live in a
module with no BlueZ types in it, so they are unit-tested without a radio and
fuzzed like the other parsers.

## 5. The GATT protocol

One primary service, three characteristics. The UUIDs are mtrack's own
128-bit values, fixed in the doc when B1 lands.

| Characteristic | Properties | Carries |
|---|---|---|
| `request` | write | Framed requests from the phone. |
| `response` | notify | Framed responses, matched to requests by id. |
| `status` | read, notify | The player's status, pushed on change and four times a second while playing. |

**Envelope.** A request names a `PlayerService` method and carries that
method's own request message; the response carries its response message or a
status code.

```proto
message BleRequest  { uint32 id = 1; string method = 2; bytes payload = 3; }
message BleResponse { uint32 id = 1; uint32 code = 2; string message = 3; bytes payload = 4; }
```

A new RPC then needs no change to the BLE layer. The driver keeps an
**allowlist** of methods it will dispatch. B1 allows the performance set: Play,
PlayFrom, Stop, Next, Previous, Seek, SeekToSection, SwitchToPlaylist, Status,
LoopSection, StopSectionLoop, SectionAck, StopSamples, CancelAutoPlay. The
config-mutating RPCs are refused over BLE.

**Framing.** A message can be larger than one ATT packet, and iOS negotiates
smaller MTUs than Android. Each write or notification is a chunk whose first
byte says whether it starts a frame; a frame is a varint length followed by the
envelope. The receiver reassembles, and drops a partial frame on disconnect or
when a new start arrives. Frames are capped at 64 KiB (a long playlist has to fit) and the
buffer is sized from the cap, never from the length on the wire.

**Status.** `StatusResponse` as it stands, sent whole when anything but the
elapsed time changes, and at 4 Hz while playing. The app runs its own clock
between updates so the display does not stutter at the notification rate.

**What gRPC lacks today.** `PlayerService` has no RPC that returns the songs in
a playlist or the list of playlists; the web UI reads those from `/api`. B1
adds them to the proto, which gRPC clients gain as well.

**The lock.** The player's lock policy gains a `Surface::Ble`, and the BLE
driver asks it the same question the web API does, so a locked player refuses
the same things on every surface.

## 6. Pairing and security

The threat is simple and real: a phone in the audience with a BLE scanner. The
spike had no protection at all and could be driven by anyone in range.

A headless Pi has no screen for a passkey, so BLE pairing can only be "Just
Works": the link is encrypted but the pairing moment itself is not
authenticated. The design makes that moment rare and owner-initiated.

- **Bonded devices only.** Every characteristic requires an encrypted link, so
  an unpaired phone can see the advertisement and nothing else.
- **Pairing is closed by default.** The adapter is not pairable except during
  a pairing window.
- **A pairing window opens three ways:** from the web UI (a button, two
  minutes); from an already-paired phone; and automatically for a few minutes
  after start while *no* phone is paired, so the first phone can claim a fresh
  Pi with no network. That last one is trust on first use.
- **The owner can see and revoke.** The web UI and the app list paired phones
  and can remove one.

BlueZ does the cryptography and stores the bonds; mtrack registers a pairing
agent, opens and closes the window, and never handles key material.

The alternative is an app-level secret (a code or QR from the web UI, then a
challenge on each connection) over an unencrypted link. It avoids the OS
pairing dialogs and their well-known quirks, at the cost of writing our own
authentication. Decision 2 in §13.

## 7. Configuration

A new controller kind, off unless configured:

```yaml
controllers:
  - kind: ble
    name: mtrack        # advertised name; defaults to the hostname
```

With the adapter blocked or absent the driver logs one clear warning and the
player starts normally, the same stance the other subsystems take.

## 8. Packaging

- **Unblock the radio.** The Pi image stops shipping Bluetooth soft-blocked.
  For `.deb` installs on other systems the docs say how; the package does not
  reach into rfkill by itself.
- **The systemd unit needs no change** (§3). `mtrack systemd` output stays as
  it is.
- **Build dependency:** `libdbus-1-dev` in CI and the release workflow;
  runtime `libdbus-1-3` in the `.deb`'s `Depends`.

## 9. The app

**Flutter**, one codebase for both platforms, with `flutter_blue_plus` for BLE
and Dart protobuf generated from the same `.proto`. The look is mtrack's own:
dark, large transport controls, readable at arm's length on a dim stage. Stock
platform widgets are not the aim; smoothness, haptics, a screen that stays
awake and a connection that comes back by itself are.

**Screens, first version.**

1. **Connect** — scan, pair, remember; reconnects to the last player on launch.
2. **Now playing** — play/stop, previous/next, elapsed and length, what is
   next, auto-play countdown and cancel, audio health.
3. **Playlist** — the songs, jump to one, switch playlist.
4. **Sections** — the song's sections, loop one, stop the loop.
5. **Settings** — paired player, open a pairing window, about.

**Where it lives.** In this repository under `app/`, so the proto and the frame
test vectors are shared by path. Its CI is a separate workflow filtered to
`app/**` and the proto, so Rust PRs do not pay for it. Decision 3.

**What cannot be verified here.**

- BLE does not work in the iOS simulator or the Android emulator. Real testing
  needs real phones.
- There is an Android phone and a Mac, and **no iPhone**. The iOS app can be
  built on the Mac and will ship untested on hardware until someone runs it on
  a device. iOS is where BLE background behaviour and pairing differ most.
- Store distribution needs an Apple developer account and a Play Console
  account, and both stores review.

## 10. BLE MIDI controllers

Separate from everything above. BlueZ on the image is built with MIDI support,
so a paired BLE MIDI pedal should appear as an ALSA sequencer port, which
`midir` already lists. If that holds, the MIDI controller and MIDI triggers
work with a BLE pedal as they do with a USB one, and the work is a docs page
plus a way to pair a pedal without ssh. Untested: it needs a pedal. BLE's
connection interval puts several milliseconds of jitter on it, fine for
play/next, wrong for anything timing-critical.

## 11. Testing

- **Codec:** unit tests and a fuzz target for the frame reassembler and the
  envelope; a set of golden frames that both the Rust tests and the Dart tests
  read, so the two ends cannot drift.
- **Driver:** dispatch, allowlist and lock policy tested against the in-process
  `PlayerServer` with the GATT transport faked.
- **Hardware:** the built-in radio is the one under test, and it cannot connect
  to itself. With a second adapter on the rig (a USB one), a `hardware_e2e`
  check connects from one to the other as a real central: pair, play, stop,
  status, refuse when unpaired. Opt-in like the trigger checks. Without a
  second adapter this layer is checked by hand with a phone (§13).
- **App:** widget and client tests against a fake transport in CI; the radio
  path by hand on real phones.

## 12. Phasing

| Phase | What ships |
|---|---|
| **B0** | The spike (done) and this doc. |
| **B1** | The BLE controller: codec, driver, pairing window, lock surface, playlist RPCs, config, docs, image unblock. Usable from nRF Connect and a small test client before any app exists. |
| **B2** | The app: connect, now playing, playlist, sections. Android first on real hardware; iOS built alongside. |
| **B3** | Headless setup: wifi credentials, the Pi's address and health, over the same service. Needs a privileged path to NetworkManager, designed then. |
| **B4** | Later, as wanted: track gains, ambient light mode, the lighting-only controls. |

## 13. Decisions

Settled with Mike, 2026-11-09:

1. **Scope of B1's allowlist.** The performance set in §5; no config changes
   over BLE.
2. **Pairing model.** OS-level bonding with a pairing window and trust on first
   use (§6), not an app-level secret.
3. **Where the app lives.** `app/` in this repository with its own
   path-filtered CI.
4. **On by default on the Pi image.** Not in B1 (opt-in controller). B3's
   headless setup only makes sense if a fresh image advertises with no config,
   so this is revisited there.
5. **iOS without an iPhone.** Build iOS from the start so it never rots; hold
   the App Store release until it has run on a real device.

Open:

6. **A second radio for the rig.** The Pi's built-in Bluetooth is all the
   feature needs. A second adapter is only for testing: a radio cannot connect
   to itself, so an automated check of the real GATT path needs something else
   to play the phone. A USB adapter on the rig is the cheapest way; without one
   that layer is checked by hand with a phone.
