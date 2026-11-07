# Bluetooth Remote: Controlling mtrack From a Phone With No Network

*Design doc, draft 1 — 2026-11-07.*

A band can't arrive at a venue, plug in a rack, and spend fifteen minutes getting a headless
Pi and a phone onto the same network before they can play. Today every phone surface — the
web UI, gRPC, OSC — needs exactly that. And even where mtrack brings its own network, Android
sends app traffic over one *default* network: join an access point with no internet and
either the phone keeps cellular as its default (and the browser cannot reliably reach the
Pi) or the user tells it to stay on the Wi-Fi (and the phone loses internet until it leaves).
Fixing that means switching airplane mode and mobile data on and off, which is the friction
this design removes.

The goal: **open the app, and it is connected to mtrack — anywhere, whether or not the
phone or mtrack has any network, without touching the phone's radios.** Bluetooth Low
Energy is the only link that runs alongside any Wi-Fi or cellular state on both iOS and
Android and needs no IP addressing, so the transport is BLE. The client is an app-store app
(iOS and Android) that runs mtrack's own web UI, loaded from the mtrack it connects to, so
the phone and the player never disagree about the API.

## 1. Goals & non-goals

**Goals.**

- A phone app (iOS and Android, app stores) that finds and connects to a paired mtrack over
  BLE in seconds, with no network on either side and no radio toggling.
- Show control at the same quality as the web UI's dashboard: transport, song and playlist
  selection, sections and loops, track gains and mutes, ambient mode, auto-play, and the
  live playback state.
- **Versions on each side can drift.** An app updated last night must work with a Pi that has
  not been updated all tour, and the other way round.
- A paired phone, and only a paired phone, can control mtrack. Not a security posture:
  stopping someone else's phone in the room from pressing "next" in the middle of a song.

**Non-goals (initial scope).**

- Song import, file uploads, and the lighting and timeline editors over BLE. They stay on
  Wi-Fi, where the bandwidth is. The full UI may *show* them over BLE; it reports that
  they need a network rather than trying.
- A web-only client. Web Bluetooth does not exist in iOS Safari and, on Android, cannot yet
  reconnect without a device picker each time; it fails "anywhere, reliably".
- Bluetooth audio. Nothing here touches the audio path.
- BLE-MIDI and HID foot controllers. Those are separate, smaller pieces (§13) and share
  only the platform work with this one.
- macOS and other non-Linux hosts for the player side. The BLE server is Linux (BlueZ) only,
  behind a feature flag.

## 2. Settled decisions

| Decision | Choice | Why |
|---|---|---|
| Transport | BLE, L2CAP connection-oriented channel (CoC) first, a GATT "pipe" fallback | The only phone link independent of IP routing. L2CAP CoC is a byte stream with better throughput than GATT; iOS 11+, Android 10+ and BlueZ all support it. The GATT pipe covers phones and stacks where CoC turns out flaky. |
| Client | One app-store app per platform: a thin native shell (Capacitor) that runs mtrack's web UI in a WebView | Reuses the existing Svelte UI; no second UI to keep in step. |
| Where the UI comes from | **The mtrack the shell is connected to**, downloaded over BLE once per mtrack build and cached | Keeps the property that makes the web UI safe today: the UI and the player are always the same build (§6). |
| What the shell must stay compatible with | Only the link/handshake (§4) and the shell–UI bridge (§7) | Everything else is in-build, between a bundle and the player it came from. |
| Fallback client | A built-in minimal remote in the shell, speaking a frozen core protocol (§5) | Works with every mtrack that speaks BLE, with no bundle: older players, an interrupted download, the first connect at a gig. |
| Core vocabulary | `TransportAction` (`src/controller.rs`) plus a minimal status | Already the stable command set MIDI and OSC share. |
| Server side | In-process: BLE requests go straight into the web UI's existing axum `Router` | The router already holds the static UI, `/api`, and gRPC-web `PlayerService` behind one lock (`src/webui/server.rs`). No second API, no second lock check. |
| Trust | OS bonding plus an mtrack-issued per-phone key, issued only while a pairing window is open | The Pi has no screen, so BLE pairing is "Just Works"; the key is what says *this* phone belongs to *this* rig. |
| Lock mode | Applies unchanged | Requests go through the same router and guards as the web UI. |

## 3. Architecture

```
 phone                                              mtrack (Pi)
┌──────────────────────────────────────┐          ┌──────────────────────────────────────┐
│ Native shell (Capacitor)             │          │ ble::Server (feature "ble", Linux)    │
│  ├ BLE link: L2CAP CoC / GATT pipe   │◄────────►│  ├ link, framing, channel scheduling  │
│  ├ handshake, pairing key store      │   BLE    │  ├ handshake, pairing, key check      │
│  ├ bundle cache (by build + hash)    │          │  ├ core channel ──► TransportAction    │
│  ├ minimal remote (core protocol)    │          │  ├ http channel ──► axum Router        │
│  └ WebView                           │          │  │                  (oneshot, in-proc) │
│     └ mtrack web UI bundle           │          │  └ event channel ◄── broadcast_tx      │
│        └ transport: shell bridge     │          │                     (SocketMessage)   │
└──────────────────────────────────────┘          └──────────────────────────────────────┘
```

On the player, everything after framing reuses what the web UI already has:

- **HTTP channel → the router.** Each request is built into an `http::Request` and passed to
  the web UI's `Router` with `tower::ServiceExt::oneshot`, the way `grpc_web_handler` already
  passes gRPC-web requests to `PlayerService`. Static assets, `/api`, and
  `/player.v1.PlayerService/{method}` all work without a TCP socket. The router has to be
  built once and shared between the TCP listener and the BLE server, instead of being
  built inside the web UI's start function.
- **Event channel ← the websocket feed.** The websocket upgrade cannot go through `oneshot`,
  so the BLE server subscribes to `broadcast_tx` directly and forwards the same
  `SocketMessage` JSON the websocket sends, with the same on-connect snapshot
  (`build_state_json` and friends).
- **Core channel → `TransportAction::run`.** The same function the MIDI and OSC controllers
  call.

The BLE server is a new controller kind: it implements the controller `Driver`, is configured
under `controllers` like `midi` and `osc`, and is started and restarted by `Controller` the same way.

## 4. Link, framing, and handshake (frozen early)

This layer is what an app from 2028 has to speak to an mtrack from 2026. It stays small, and
any change to it only adds something.

**Discovery.** mtrack advertises a fixed 128-bit service UUID, plus a short name (the
hostname, or a configured `ble.name`). The GATT service has:

- `psm` (read, encrypted): the L2CAP PSM to open. BlueZ lets mtrack pick a fixed one, but
  reading it from GATT is how iOS clients expect to find it, and it keeps the PSM free to
  change.
- `pipe_rx` (write without response, encrypted) and `pipe_tx` (notify, encrypted): the GATT
  pipe fallback, carrying the same byte stream as the L2CAP channel.

Every characteristic requires encryption. Touching one is what makes iOS and Android show the
OS pairing prompt the first time (§8). Neither OS lets an app start bonding by itself, but
both prompt when an encrypted characteristic is accessed.

**Framing.** One byte stream, in frames:

```
frame   = length:varint  channel:u8  payload:bytes[length - 1]
```

| Channel | Purpose | Priority |
|---|---|---|
| 0 `control` | hello, pairing, keepalive, flow control, goodbye | highest |
| 1 `core` | the frozen core protocol (§5) | high |
| 2 `events` | `SocketMessage` stream | normal |
| 3 `http` | multiplexed HTTP requests and chunked responses | low |

**Scheduling is part of the design, not an optimisation.** The sender keeps one queue per
channel and always sends from the highest-priority non-empty queue. HTTP bodies are chunked
into frames of at most a few hundred bytes, so a bundle download (§6) running in the background
can never delay a `next` by more than one small frame. A keepalive every second on `control`
lets both sides notice a dead link within about three seconds. The UI shows that the same
way it shows a dropped websocket today.

**Handshake.** The first frame each way is a `Hello` on `control`, encoded as protobuf, with
the same field-evolution rules as `player.v1`: fields are only added, numbers are never
reused, unknown fields are ignored. The messages live in a new `proto/ble/v1/link.proto`.

```proto
message ClientHello {
  uint32 link_min = 1;          // link/handshake protocol range the shell speaks
  uint32 link_max = 2;
  string shell_version = 3;     // app version, for logs and messages
  uint32 bridge_version = 4;    // §7
  bytes  device_key_id = 5;     // empty before pairing (§8)
  bytes  device_proof = 6;      // proof of the key, over the server nonce
  repeated string capabilities = 7;
}

message ServerHello {
  uint32 link_version = 1;      // chosen from the client's range, or 0 = none fits
  string mtrack_version = 2;    // crate version
  string mtrack_build = 3;      // git hash (build.rs already embeds it)
  bytes  bundle_hash = 4;       // §6; empty = this build does not serve its UI over BLE
  uint64 bundle_size = 5;
  uint32 core_version = 6;      // §5
  repeated string capabilities = 7;
  bytes  nonce = 8;
  PairingState pairing = 9;     // paired / unpaired-window-open / unpaired-window-closed
}
```

`capabilities` are strings ("core", "http", "events", "pairing.qr", …): new features show
up as new capabilities, not as protocol-version bumps.

## 5. The core protocol (frozen)

The minimal remote built into the shell speaks only this, and mtrack keeps answering it
forever. It is deliberately the set of things a foot controller can already do:

```proto
message CoreCommand {
  uint32 id = 1;
  oneof action {
    Empty play = 2;  Empty stop = 3;  Empty next = 4;  Empty prev = 5;
    Empty section_ack = 6;  Empty stop_section_loop = 7;
    AmbientMode ambient_mode = 8;  Empty cancel_auto_play = 9;
    Empty all_songs = 10;  Empty playlist = 11;
  }
}

message CoreStatus {           // sent on change and once a second
  bool   playing = 1;
  string song = 2;
  uint64 elapsed_ms = 3;
  uint64 duration_ms = 4;
  string playlist = 5;
  uint32 playlist_index = 6;
  uint32 playlist_len = 7;
  string next_song = 8;
  string section = 9;          // current section, if any
  string loop_state = 10;      // offered / armed / looping / break_requested / ""
  bool   audio_ok = 11;        // the audio-health signal OSC already broadcasts
  bool   locked = 12;
}
```

The command set maps one-to-one onto `TransportAction::ALL`. A new `TransportAction` may be
added to `CoreCommand` as a new `oneof` field. An existing one is never renamed, renumbered,
or given new meaning. Every `CoreCommand` is answered with a `CoreAck { id, ok, message }`,
so the remote can show that a press arrived. A controller has nowhere to report a failure
today; BLE does.

## 6. Serving the UI from the player

The web UI is safe today because a browser always gets the UI built alongside the player it
talks to (the bundle is embedded with `rust-embed`). The shell keeps that property instead of
shipping its own copy of the UI.

**The bundle.** At build time, mtrack packs the embedded UI into one compressed archive, and
the archive's hash is `bundle_hash`. It is served at `GET /ble/bundle` on the HTTP channel,
supports `Range` so an interrupted download resumes, and is answered `304` when the
client already has that hash.

**The shell's cache** is keyed by `bundle_hash`, keeps the last few bundles, and is never
invalidated by an app update. On connect:

1. Cached → start the WebView on it at once.
2. Not cached → open the minimal remote immediately, download in the background on the
   lowest-priority channel, switch to the full UI when the download finishes (and only if
   the player is stopped, or the user taps to switch).
3. `bundle_hash` empty (an mtrack that predates this, or built without the UI) → minimal
   remote only.

**Prefetch, so BLE is rarely the path.** Release builds' bundles are also published with
each GitHub release. When the phone has internet, the shell fetches the bundles for the
mtrack versions it has paired with, plus the latest release, and checks them against
`bundle_hash` on connect. Over BLE, a download only happens for unreleased or
locally built players.

**Size — to measure first (§12).** The bundle includes three.js and the rules WebAssembly
module, and has not been measured. At realistic BLE throughput (tens of KB/s up to a little
over 100 KB/s), a few MB means roughly half a minute to a few minutes. If that is too slow,
split it: a show-mode bundle (dashboard, playlists, songs list) that is always served over
BLE, and an editor bundle loaded only over a network.

**Assets** are loaded in the WebView from a custom scheme (`mtrack-bundle://<hash>/…`), which
the shell serves from the cache: `WKURLSchemeHandler` on iOS, `WebViewAssetLoader` or
`shouldInterceptRequest` on Android. That is how Capacitor serves its own built-in assets
already. The shell only serves GETs this way; API calls go through the bridge (§7), which
avoids depending on how each platform passes request bodies through a scheme handler.

**App Store review.** Apple's rules allow web content downloaded at runtime to run in WebKit,
provided it does not change the app's main purpose. Live-update tools for Capacitor and
React Native depend on this, and a device serving its own control UI is the case it fits
best. Say so plainly in the review notes; it is still a review risk to plan for.

## 7. The shell–UI bridge (the second stable contract)

The UI already reaches the player through three small modules: `ws/connection.ts` (the
websocket), `grpc/client.ts` (gRPC-web), and `api/rest.ts` (plus the per-area API files).
Each gets a transport chosen at startup:

- **browser** (today's behaviour, unchanged)
- **shell**, when `window.mtrackShell` exists

The bridge the shell injects is the entire contract between an app version and a bundle
version:

```ts
interface MtrackShell {
  readonly bridgeVersion: number;
  readonly capabilities: readonly string[];   // e.g. "events.filter", "share.file"
  info(): Promise<{ shellVersion: string; mtrackVersion: string; link: "l2cap" | "gatt" }>;

  // HTTP into the player's router. gRPC-web rides on this as it does in a browser.
  request(req: {
    method: string; path: string; headers: [string, string][]; body?: Uint8Array;
  }): Promise<{ status: number; headers: [string, string][]; body: Uint8Array }>;

  // The websocket feed. `types` narrows what is sent over BLE (§9).
  events(types: readonly string[], onMessage: (json: string) => void,
         onStatus: (connected: boolean) => void): () => void;

  pairing(): Promise<PairingInfo>;            // §8
}
```

Rules that keep it compatible:

- The bundle feature-detects (`capabilities.includes(...)`), never compares
  `bridgeVersion` for equality, and degrades when something is missing.
- The shell only ever adds members. Changing a member's meaning means adding a new member
  with a new name.
- `bridgeVersion` goes in the `ClientHello`, so mtrack can log it, and in the future refuse a
  shell too old for its UI with a clear message instead of a broken screen.

The Home Assistant companion app does the same with its "external bus": a native shell
loads the server's frontend, and a small versioned bridge connects them.

## 8. Pairing and trust

The Pi has no screen and no buttons, so BLE pairing is "Just Works": it encrypts the link but
cannot by itself tell the band's phone from anyone else's in the room. mtrack adds its own
key.

- **Pairing window.** mtrack accepts new phones only while a pairing window is open, for two
  minutes. It opens from:
  - the web UI (the Bluetooth section of the config page, over Wi-Fi at home);
  - the TUI;
  - a new `TransportAction` (`BlePairing`), so a MIDI or OSC button can open it at a venue;
  - optionally, every boot for the first two minutes (`ble.pair_on_boot`), for players with no
    other control surface.
- **Key exchange.** Inside the window, a new phone gets a per-device key over the encrypted
  link, and mtrack stores `(key id, key hash, device name, first seen)` in its config
  directory. On every later connect, the phone proves it holds the key over the server's
  nonce (`device_proof`).
- **QR pairing (optional).** The web UI can show a QR code with a one-time code, for
  "pair this phone" without opening a window anyone in range could use.
- **Managing phones.** The web UI and gRPC list and revoke paired phones. Revoking a phone also
  removes its BlueZ bond.
- **Lock mode** applies unchanged. Unpaired connections get only the `control` channel.

## 9. What crosses the link

| Traffic | Over BLE | Notes |
|---|---|---|
| Core channel | always | tiny |
| `SocketMessage::Playback` | yes | about 5/s today (`src/webui/state.rs`); measure the JSON size |
| `Metadata`, `Reload` | yes | rare |
| `State` (lighting stage frames) | off unless asked for | the lighting views request it with `events(["state", …])` |
| `Waveform` | off unless asked for | large; prefer the waveform cache over HTTP when a song detail asks |
| `Logs` | off unless asked for | |
| `/api` reads, gRPC-web | yes | per request |
| Uploads, imports | refused with a clear error | unless measured to be tolerable for small files |

The filter is per connection and set by `events(types)`. The server applies it before
serialising, so filtered messages cost nothing on the link. If measurement shows `Playback`
dominates, a compact binary encoding of it (on a capability) is the first optimisation, not a
change to the websocket.

## 10. Version policy

| Situation | Result |
|---|---|
| Link ranges overlap, bundle cached or downloadable | full UI from that mtrack's own bundle |
| Link ranges overlap, no bundle (older mtrack, download failed, still downloading) | minimal remote |
| mtrack's link version newer than the shell's | minimal remote if `core` is offered, plus "update the app" |
| mtrack too old for the shell's oldest link version | "update mtrack"; the shell drops a link version only after it has been unused for a long, documented period |
| Bundle needs a bridge capability the shell lacks | the bundle degrades (hides that feature) and says why |

mtrack's own compatibility rules stay as they are: a bundle and its player are one build,
so the `/api`, gRPC-web and `SocketMessage` shapes can change between releases as they do today.
The stable contracts are only §4, §5 and §7.

## 11. Player platform work

- **BlueZ over D-Bus** through the `bluer` crate: advertising, GATT server, L2CAP CoC
  listener, bond management. Feature `ble`, Linux only, on by default in the Pi image.
- **The systemd unit blocks Bluetooth sockets today.** The unit `mtrack` generates
  (`src/cli.rs`) sets `RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX`. D-Bus to BlueZ
  works over `AF_UNIX`, but the L2CAP socket needs `AF_BLUETOOTH`. Add it, and add the
  `mtrack` user to whichever group BlueZ's D-Bus policy allows to register adverts and GATT
  services on the image (`bluetooth` on Debian; verify on Raspberry Pi OS). The same applies
  to the Debian package's unit.
- **Pi image (`packaging/pi-gen`).** Bluetooth enabled and the adapter powered on at boot,
  not discoverable as a classic device, and no other services advertised over BLE.
- **Radio sharing.** On the Pi 3B+, 4, 5 and Zero 2 W, Bluetooth shares a chip and antenna
  with 2.4 GHz Wi-Fi. Measure BLE throughput and latency with the Pi also running its own
  access point. A USB Bluetooth adapter is the documented fallback if they interfere.
- **The audio path** is untouched; the BLE server runs on the async runtime like the other
  controllers, at normal priority.

## 12. Testing

- **Link and framing**: unit tests and a fuzz target (`fuzz/` exists) for the frame parser and
  scheduler, including a background bundle download sharing the link with core commands
  (core commands keep their latency bound).
- **A Rust test client** (`mtrack ble-remote`, or a `harness` mode) that speaks §4/§5 over
  BlueZ. It proves the protocol on a real Pi before any phone code exists, and doubles as a
  debugging tool.
- **In-process tests without radios**: the BLE server's channels over an in-memory duplex
  stream, exercising the HTTP-into-router path and the event filter in CI.
- **Compatibility matrix in CI**: the current shell's protocol code (shared as a library
  between the Rust test client and the shell's native layer, or mirrored with conformance
  vectors) run against the last *N* mtrack releases — core and full UI paths — and the last *N*
  shell releases run against the current mtrack. Recorded `Hello` exchanges from every release
  are kept as fixtures, so an incompatible handshake change fails a test.
- **Bridge contract tests**: the UI's shell transport against a fake `mtrackShell`,
  including missing capabilities.
- **On devices, before each app release**: an iPhone and two Android phones from different
  manufacturers. Pairing, reconnect after the Pi reboots, reconnect after the phone sleeps,
  cellular on/off, Wi-Fi joined to an unrelated network — the radio states this design exists
  to make irrelevant.

**Measure first** (P0): bundle size, `Playback` message size and rate, L2CAP and GATT
throughput on a Pi 4 and Pi 5 with the access point on, and reconnect time from app launch.

## 13. Phasing

| Phase | Delivers | Depends on |
|---|---|---|
| **P0 — measure** | the numbers in §12; the systemd and image changes in §11; a spike that opens an L2CAP channel from an iPhone and an Android phone to `bluer` | — |
| **P1 — player link + core** | `ble` controller: advertising, framing, handshake, pairing, core channel; Rust test client | P0 |
| **P2 — minimal remote app** | shell with BLE link, pairing, the built-in minimal remote; TestFlight / internal testing | P1 |
| **P3 — HTTP + events channels** | the router shared between TCP and BLE; the event filter; the UI's shell transport (no visible change in a browser) | P1 |
| **P4 — full UI in the shell** | `/ble/bundle`, the shell's bundle cache and WebView, release-bundle prefetch, version policy; app-store release | P2, P3 |
| **P5 — polish** | QR pairing, paired-phone management in the web UI, show-mode/editor bundle split if P0's numbers call for it | P4 |

The pieces that share the platform work (§11) can run alongside: **BLE-MIDI foot
controllers** (BlueZ exposes them as ALSA sequencer ports midir already sees; the work is
pairing and a control-input MIDI device separate from the player's MIDI output), and an
**evdev input controller** for Bluetooth and USB page-turner pedals and keypads mapped
through `TransportBindings`.

## 14. Open questions

1. **Bundle size.** Is one bundle fast enough over BLE with prefetch covering releases, or is
   the show-mode/editor split needed from the start? Decided by P0's numbers.
2. **L2CAP on Android.** CoC support varies by manufacturer more than the API level suggests.
   Is the GATT pipe a rarely used fallback or a common path? Also decided by P0's numbers.
3. **More than one phone at once.** The band's phone and the MD's phone, both connected.
   BlueZ allows several central connections. The protocol has no per-connection state that
   prevents it; the question is the radio budget.
4. **Shared protocol code.** Is the native layer's link code written twice (Swift and Kotlin)
   against conformance vectors, or once in Rust and bound to both through UniFFI? One
   implementation is attractive for a protocol meant to stay compatible for years.
5. **Opening the pairing window at a venue** for a new phone, with no network and no foot
   controller: is the boot-time window enough?
