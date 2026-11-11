# Bluetooth Control

The player can be controlled from a phone over Bluetooth Low Energy, with no network between
them: no wifi, no router, no cable. The phone pairs with the player once, and after that it
finds it and reconnects by itself when it is in range. The Bluetooth controller carries the same
`PlayerService` messages as [gRPC](grpc.md), so a phone can do what a gRPC client can do during a
show: transport, playlist, section loops, track levels, the ambient light and status.

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
| `stream` | `6d747261-636b-0005-8000-00805f9b34fb` | notify |

Subscribe to `response` and `status` before sending anything, and to `stream` before opening a
[stream](#streams). The UUIDs are also constants in the source (`SERVICE_UUID` and its siblings
in `src/controller/ble.rs`). A player from before streams has no `stream` characteristic.

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
message Request  { uint32 id = 1; string method = 2; bytes payload = 3; bool cancel = 4; }
message Response { uint32 id = 1; uint32 code = 2; string message = 3; bytes payload = 4; bool more = 5; }
```

`cancel` and `more` are for [streams](#streams) and are never set in a unary call or its
answer, so a client that knows nothing of streams reads and writes exactly what it always did.

`method` is the bare name of a `PlayerService` method (`Play`, `Seek`, ...) and `payload` is that
method's request message from [`player.proto`](grpc.md), serialized. The response carries the
method's response message in `payload`, or, when `code` is not 0, a gRPC status code and message
and no payload. The player's [lock](../interfaces/web-ui.md#lock-mode) applies exactly as it does to gRPC
and the web UI: a locked player refuses what it refuses there.

The calls a phone may make, and no others:

`Play`, `PlayFrom`, `PlaySongFrom`, `Stop`, `Next`, `Previous`, `NavigateToSong`, `Seek`,
`SeekToSection`, `SwitchToPlaylist`, `Status`, `LoopSection`, `StopSectionLoop`, `SectionAck`,
`StopSamples`, `CancelAutoPlay`, `ListPlaylists`, `GetPlaylist`, `GetTrackGains`, `SetTrackGain`,
`SetTrackMute`, `SetAmbientMode`, `SavePlaylist`, `DeletePlaylist`, `SetLock`, and one stream,
`WatchStage` (see [Streams](#streams)).

`SavePlaylist` and `DeletePlaylist` edit a playlist's songs and their order, as the web UI's
playlist editor does with only the songs: each entry keeps its own settings (fades, auto play,
ambient). Set `check_expected` and `expected` on a save to refuse it if the playlist was edited
since the phone read it (`FAILED_PRECONDITION`, "the playlist has changed"). Both are refused
while the player is locked, and a player starts locked. A phone can lift the lock itself with
`SetLock` (`locked: false`), which is open while locked, and put it back when it is done. That
is on purpose: a player with no network, which is what Bluetooth is for, would otherwise never
be editable from a phone. Anyone paired can do it, as anyone paired can already play and stop.

Anything else, every call that changes the configuration included, is refused with
`PERMISSION_DENIED`. A response too large for a frame is replaced by a `RESOURCE_EXHAUSTED`
response with the same `id`.

A notification on a characteristic reaches every phone subscribed to it. So two phones see each
other's responses. **Start your call ids at a random number, and ignore any response whose id you
did not send.**

A phone's requests are answered one at a time, in the order they arrive. A slow call from one
phone does not hold up another phone, or the status. A phone that has more than a handful of
requests waiting has the excess dropped.

### Streams

`WatchStage` is a server stream: one request, answered by many messages. It is the only one a
phone may open. Everything about a stream arrives on the `stream` characteristic, framed and
chunked like a response, and nothing about it arrives on `response`.

**Opening.** Write a `Request` to `request` as for any call: `method` is `WatchStage`, `payload`
its `WatchStageRequest`, and `id` an id you have no stream open under. Each message of the stream
then arrives on `stream` as a `Response` with your `id`, `more` set, `code` 0 and the message
(a `StageUpdate`) in `payload`.

**The end.** A stream ends with one `Response` that has `more` unset. It carries the call's gRPC
status in `code` and `message`, and no payload. On `stream`, a `Response` without `more` always
ends the stream with that `id`. That is how every stream ends that the phone is told about, and
also how one that never opened is refused (its id is then no open stream's, with the one
exception below):

| `code` | Why |
|---|---|
| 0 (`OK`) | The player ended the stream. `WatchStage` does not, short of a failure. |
| 1 (`CANCELLED`) | You cancelled it. |
| 8 (`RESOURCE_EXHAUSTED`) | You have two streams open, which is the most one phone may have; or a message of the stream was too large for a frame, which ends it. |
| any other | The call's own refusal or failure, as gRPC would report it: a `WatchStageRequest` that does not parse, for one. |

A stream is open, and counts toward the two, from its request until its last `Response` has been
sent. So after a cancel, wait for the last `Response` before counting the stream closed or using
its id again.

**An id that is already open** is the one refusal of a stream that comes on `response`: a
request for a stream under an `id` you have a stream open under is answered there, with
`ALREADY_EXISTS` (6), and the stream that is open carries on untouched. On `stream` that answer
would carry the open stream's id with `more` unset, and read as its end. A client that never
reuses an open id never sees this.

So a stream method is answered on `stream`, the refusals of the call itself and of a third
stream included, and everything else is answered on `response`; the only answers to a stream
request that come on `response` are `ALREADY_EXISTS`, above, and an old player's: a player from
before streams does not know `WatchStage`, and answers it like any method it does not allow,
with `PERMISSION_DENIED`. A client that might meet one listens there for that answer, or checks
first whether the service has a `stream` characteristic.

**Cancelling.** Write a `Request` with the stream's `id` and `cancel` set; `method` and `payload`
are ignored. The player drops its side of the stream at once, discards any message it had not
begun to send, finishes one it had, and then sends the last `Response`, with `CANCELLED`.
Nothing answers the cancel itself. A cancel for an id you have no stream open under is ignored,
without an answer: the stream may have ended by itself while the cancel was on its way. One
phone cannot cancel another's stream.

A stream also ends, with no last `Response`, when the phone disconnects, is forgotten, or the
controller stops, and when nobody at all is subscribed to `stream`. The player cannot tell which
phone unsubscribed, so it is the last subscription going that ends the streams, every phone's.
Open a stream only once you are subscribed.

**Newest wins.** A gRPC stream is ordered and loses nothing. Over a radio that would mean a
growing delay whenever the stage changes faster than the link carries it, and a lighting frame
that arrives late is worthless. So the player keeps, for each stream, only what is still worth
sending:

- A `StageUpdate` holding a `frame` is *replaceable*. At most one waits for the radio, and a newer
  frame takes its place. You see fewer frames, never older ones.
- A `StageUpdate` holding a `layout` is never dropped, and layouts arrive in order. When one is
  queued, a frame waiting ahead of it is discarded: it was read against the layout before. So no
  frame for an older layout follows a newer layout. (Each frame names its layout by id all the
  same, and a frame for a layout you do not hold is to be dropped.)
- A message whose first chunk has been sent is sent whole.

A stream never holds up a call. The player sends a stream one chunk at a time, and before each
chunk it sends anything waiting on `response` or `status` first: a Stop is answered ahead of the
rest of a layout, however large the layout.

**The limit of this version.** The player cannot see how fast the link is. The Bluetooth adapter
takes each chunk at once and queues it itself, and says nothing of when it leaves or how many
wait; nothing handed to it can be overtaken or taken back. So the player paces streams at a
fixed rate instead, 16 kB a second across all of them, whatever the phone: newest wins and
calls-go-first both happen in the player, in front of that pacing. On a link that carries at
least that much (the stage needs a fraction of it, and a phone at close range carries several
times it) the adapter's queue stays about a chunk deep and all of the above holds. On a link
slower than what a stream is sending (a phone at the edge of range, say) a backlog can build
inside the adapter, where the player can neither see nor shorten it: frames then arrive late
instead of being replaced, and a call's answer waits behind what is queued. A client that sees
frames falling behind can ask for fewer (`max_rate`), or cancel the stream and open it again.

Like every notification, a stream's reaches every phone subscribed to `stream`: ignore a
`Response` whose id is not a stream of yours. If you subscribe in the middle of a frame, ignore
continuation chunks until the next start.

### Status

The `status` characteristic carries a small `Status` message: the playlist and song names, the
song's length, whether it is playing, the elapsed time, the playlist position, the song
waiting to start by itself, if any, the section a section loop is set on, if any, the ambient
light's mode and state (`ambient_mode`: `auto`, `on` or `off`; `ambient_state`: `playing`,
`pending`, `off` or `unavailable`; read about twice a second, and absent on a player with no
DMX engine) and `gains_revision`. That last is a count the player moves whenever any track's gain or mute changes, from any surface; call
`GetTrackGains` again when it differs from the one you last saw. It wraps, so compare for
inequality. `locked` says whether the player is locked, so a phone shows the lock without
asking, and is pushed when it changes. The message carries no beat grid and no sections; fetch those with the `Status` call when the song changes.

- **Read** returns the latest `Status` as a bare encoded message, not chunked, at most 512
  bytes (a long name is shortened to fit). It uses long reads if the MTU is small.
- **Notify** pushes the same message as a frame, chunked as above, whenever anything but the
  elapsed time changes and about four times a second while a song is playing. The elapsed time
  is as of the message: run your own clock between messages. A notification subscription starts
  with the latest status. If you subscribe in the middle of a frame, you receive continuation
  chunks with no start: ignore them until the next start.

## Phone app

A phone app for this protocol is in [`app/`](https://github.com/mdwn/mtrack/tree/main/app) in the
source repository. It is not released in any store; it builds as an Android APK (the CI run
for a change to `app/` uploads one as an artifact) and as an iOS project that has not been run
on a phone. It finds a player, pairs, and then reconnects to it by itself. Its five tabs show
the song and the transport (play, stop, previous and next), the playlist (tap a song to make it
current; switch to another playlist), the song's sections (tap to jump to one, hold to loop
it), the output tracks' levels and mutes, and the lights: the ambient light's mode, and under it
the stage the web UI draws, as a plot or in 3D, live from the [stage stream](#streams). The 3D
view turns under a finger; its fixture models come from the player with `GetLightingAsset` and
the app keeps them. Stop is a press and hold, and previous, next and choosing a song are off while a song plays.
Until the app is released, any client that follows the protocol above works.
