# Bluetooth Control

The player can be controlled from a phone over Bluetooth Low Energy, with no network between
them: no wifi, no router, no cable. The phone pairs with the player once, and after that it
finds it and reconnects by itself when it is in range. The Bluetooth controller carries the same
`PlayerService` messages as [gRPC](grpc.md), so a phone can do what a gRPC client can do during a
show: transport, playlist, section loops and status.

This page covers setting the controller up, how pairing works, and the protocol, for anyone
writing a client.

## Configuration

Add a `ble` controller to the profile's `controllers` (see
[Player Configuration](../configuration/player-config.md) for where controllers live):

```yaml
controllers:
  - kind: ble
    # The name the player advertises. Optional; defaults to the machine's hostname.
    name: mtrack
```

There is one Bluetooth adapter, so a configuration with a second `ble` controller is refused.

The controller needs:

- **A Bluetooth adapter, powered and not soft-blocked.** mtrack does not unblock the radio for
  you. While the adapter is blocked or missing, the rest of the player runs as usual, the web
  UI's Bluetooth card says why Bluetooth is not running, and the controller tries again every
  few seconds, so it starts by itself once the adapter is there. Check with
  `rfkill list bluetooth` and unblock it with `sudo rfkill unblock bluetooth`. The
  [Raspberry Pi image](../getting-started/raspberry-pi.md) unblocks Bluetooth once, the first
  time it boots with an adapter, because Raspberry Pi OS otherwise blocks every radio until a
  wifi country is set.
- **`bluetoothd` (BlueZ) running.** Without it the controller waits the same way. BlueZ ships
  with Raspberry Pi OS and most desktop distributions.
- **A build with Bluetooth support.** Release builds for Linux have it. A build without it (macOS,
  or built with `--no-default-features`) reports "this build has no Bluetooth support" for a
  configured `ble` controller and starts the player normally. The `.deb` depends on `libdbus-1-3`.

The stock [systemd unit](../deployment/systemd.md) needs no change.

## Pairing

Pairing is the only thing that stands between a stranger's phone and the Stop button, so it is
closed by default.

- Every characteristic that controls or reports on the player requires an encrypted link, so a
  phone that has not paired can see the advertisement and nothing else.
- The adapter accepts new pairings only while the **pairing window** is open. The player has no
  screen and nobody at it, so it accepts every pairing a phone starts while the window is open,
  and refuses every one while it is closed. Your phone may show a six-digit code and ask you to
  confirm it: confirm it. The player does not check the code, so the link is encrypted but the
  moment of pairing is not authenticated. Whoever pairs while the window is open is trusted from
  then on. That is why the window is closed except when you open it.
- **A fresh player claims its first phone.** When the controller starts and no phone is paired,
  the window opens for five minutes, so the first phone can pair with a player that has no
  network and no other way to be configured. Once any phone is paired, the controller starts with
  the window closed.
- **Unpaired devices are dropped.** While the window is closed, a device that connects without
  being paired is disconnected after two seconds. While the window is open it is left alone, and
  when the window closes, any device still connected without having paired is dropped.
- **A device you trust in BlueZ is left alone.** Phones are paired through the pairing window.
  Other Bluetooth devices can share the adapter, a BLE MIDI pedal you paired with `bluetoothctl`
  and marked `trust`, say. BlueZ's *Trusted* flag is the line: a trusted device is never
  disconnected by the rules above, does not count as a paired phone (so it does not stop a fresh
  player opening its first window), and is not listed in the web UI or forgotten from it. The
  player never marks a device trusted itself.

### Pairing a phone from the web UI

Open **Config**, the profile, then **Controllers**: the Bluetooth card has the pairing controls.
(The card needs the configuration saved, so that the controller is running.)

1. Click **Pair a phone**. The card says *Pairing open* with the time left; the window stays open
   for two minutes and closes by itself. **Stop pairing** closes it sooner.
2. On the phone, connect to the name the player advertises (the card's *Advertised name*, or the
   hostname). The phone may show a code to confirm.
3. The phone appears under *Paired phones*, with a *connected* marker while it is connected.

To remove a phone, click **Forget** beside it and confirm; it must then pair again, through the
window. Opening or closing the window and forgetting a phone change who can control the player,
so the web UI refuses them while the player is [locked](web-ui.md): unlock it first. The same
operations are the REST endpoints `GET /api/bluetooth`, `POST /api/bluetooth/pairing`
(`{"open": true, "seconds": 120}` or `{"open": false}`; the time is kept between 10 and 600
seconds) and `DELETE /api/bluetooth/devices/{address}`.

### Without a network

A player with no network has no web UI to reach. With no phone paired, the window opens for five
minutes each time the controller starts (restart the `mtrack` service), so a player whose phones
are all gone can be claimed again. To get there from a shell, forget the phone in BlueZ with
`bluetoothctl remove <address>` (`bluetoothctl devices Paired` lists the addresses), then restart
the service and pair the new phone inside that window.

## The service

The controller registers one primary GATT service and advertises its UUID, with the name from the
configuration (or the hostname), as discoverable.

| What | UUID | Properties |
|---|---|---|
| Service | `6d747261-636b-0001-8000-00805f9b34fb` | primary |
| `request` | `6d747261-636b-0002-8000-00805f9b34fb` | write, write without response; encrypted link |
| `response` | `6d747261-636b-0003-8000-00805f9b34fb` | notify |
| `status` | `6d747261-636b-0004-8000-00805f9b34fb` | read (encrypted link), notify |

Subscribe to `response` and `status` before sending anything. The UUIDs are also constants in
the source (`SERVICE_UUID` and its siblings in `src/controller/ble.rs`).

### Framing

A message can be larger than one ATT packet, and phones negotiate different MTUs, so every write
and every notification is a **chunk** of a **frame**:

- A chunk's first byte is a header: `0x01` begins a frame, `0x00` continues one. Any other
  header is an error.
- The first chunk goes on with the frame's length as a protobuf varint, then the first bytes of
  the frame. Each later chunk is the header and more bytes of the frame. The frame ends when it
  has as many bytes as the length said.
- A frame is at most 65,536 bytes. A frame with no bytes is one chunk, `01 00`.
- A start chunk while a frame is in progress drops that frame and begins the new one. A chunk
  that makes no sense (no frame in progress, more bytes than declared, a bad header) is dropped,
  along with the frame it broke.

The player cuts what it sends to the smallest MTU of the phones it knows about, minus the 3 bytes
ATT keeps, so every connected phone can receive it. Write chunks no larger than the MTU you
negotiated, minus 3. Send a request's chunks in order, and prefer *write with response*: it keeps
the chunks in order end to end, and a write without response pipelined faster than the player
handles it can arrive out of order, which drops the frame.

A frame is a protobuf message from [`ble.proto`](https://github.com/mdwn/mtrack/blob/main/src/proto/ble/v1/ble.proto).

### Requests and responses

Write a `Request` frame to `request`; the matching `Response` frame comes back on `response`.

```proto
message Request  { uint32 id = 1; string method = 2; bytes payload = 3; }
message Response { uint32 id = 1; uint32 code = 2; string message = 3; bytes payload = 4; }
```

`method` is the bare name of a `PlayerService` method (`Play`, `Seek`, ...) and `payload` is that
method's request message from [`player.proto`](grpc.md), serialized. The response carries the
method's response message in `payload`, or, when `code` is not 0, a gRPC status code and message
and no payload. The player's [lock](../interfaces/web-ui.md) applies exactly as it does to gRPC
and the web UI: a locked player refuses what it refuses there.

The calls a phone may make, and no others:

`Play`, `PlayFrom`, `PlaySongFrom`, `Stop`, `Next`, `Previous`, `Seek`, `SeekToSection`,
`SwitchToPlaylist`, `Status`, `LoopSection`, `StopSectionLoop`, `SectionAck`, `StopSamples`,
`CancelAutoPlay`, `ListPlaylists`, `GetPlaylist`.

Anything else, every call that changes the configuration included, is refused with
`PERMISSION_DENIED`. A response too large for a frame is replaced by a `RESOURCE_EXHAUSTED`
response with the same `id`.

A notification on a characteristic reaches every phone subscribed to it. So two phones see each
other's responses. **Start your call ids at a random number, and ignore any response whose id you
did not send.**

A phone's requests are answered one at a time, in the order they arrive. A slow call from one
phone does not hold up another phone, or the status. A phone that has more than a handful of
requests waiting has the excess dropped.

### Status

The `status` characteristic carries a small `Status` message: the playlist and song names, the
song's length, whether it is playing, the elapsed time, the playlist position, and the song
waiting to start by itself, if any. It carries no beat grid and no sections; fetch those with the
`Status` call when the song changes.

- **Read** returns the latest `Status` as a bare encoded message, not chunked, at most 512
  bytes (a long name is shortened to fit). It uses long reads if the MTU is small.
- **Notify** pushes the same message as a frame, chunked as above, whenever anything but the
  elapsed time changes and about four times a second while a song is playing. The elapsed time
  is as of the message: run your own clock between messages. A notification subscription starts
  with the latest status. If you subscribe in the middle of a frame, you receive continuation
  chunks with no start: ignore them until the next start.
