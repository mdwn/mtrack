# Phone remote, phase 3: playlist settings, songs, Wi-Fi

*Design doc, draft 1 — 2026-11-11. Follows the light view
(`design/bluetooth_light_view_design.md`, PRs #717–#725), which Mike ran on
his Pixel, plot and 3D. He asked for three things next: the playlist editor
to reach the settings the web UI's does (transitions, ambient light), the
song library with enough editing to debug on the fly (looping first), and
Wi-Fi beside Bluetooth. This doc says what each needs, in what order, and
what is his to decide (§8).*

## 1. Where things stand

| Feature | On the player today | Missing |
|---|---|---|
| Playlist settings | The playlist file carries each entry's fades, auto play and ambient light, and a playlist's own ambient. `SavePlaylist` keeps them with their songs but cannot change them | A way to read and write them over gRPC |
| Songs | REST only: the whole song file, read and written as YAML. gRPC has no song call but what a playlist lists | Any gRPC read or write |
| Wi-Fi | A gRPC port and a web UI, both open to the network with no sign-in | A way for the app to reach either; a way to find the player |

Everything the phone calls goes through the in-process gRPC `PlayerService`
under the player's lock, and that stays true for all three.

## 2. Playlist settings

The app's editor does songs and order. It gains what the web UI's editor
has:

- **Per song:** fade out, fade in, auto play (off, on, or a gap), and
  ambient light in the gap before it.
- **Per playlist:** ambient light (off, the player's, or settings of its
  own).

On the wire, lengths travel as the player writes them (`"2beats"`, `"1s"`),
and the player checks them, so the app does not grow a second parser.

As built:

```proto
message PlaylistEntry {
  string song_name = 1;
  google.protobuf.Duration duration = 2;
  EntrySettings settings = 3;   // always sent, empty for none
}

message EntrySettings {
  Fade fade_out = 1;
  Fade fade_in = 2;
  AutoPlay auto_play = 3;       // absent: waits for Play
  AmbientSetting ambient = 4;   // absent: the playlist's
}

message Fade {
  string length = 1;
  optional string curve = 2;
  TrackNames exclude = 3;       // absent: the default; empty: none
  optional string lights = 4;
}

message TrackNames { repeated string names = 1; }

message AutoPlay { string gap = 1; }   // empty: at once

message AmbientSetting {
  bool on = 1;                  // false: none
  optional string show = 2;
  optional string delay = 3;
  optional string fade_in = 4;
  optional string fade_out = 5;
}

message GetPlaylistResponse {
  // 1–3 as before
  AmbientSetting ambient = 4;   // absent: the profile's
  string revision = 5;          // empty from an older player
}

message SavePlaylistRequest {
  // 1–5 as before (5 is create)
  repeated EntrySettings settings = 6;   // one per song, in order
  bool set_settings = 7;                 // unset: keep them, as before
  AmbientSetting ambient = 8;            // absent: the profile's
  bool set_ambient = 9;
  string expected_revision = 10;
}

rpc ListLightShows(ListLightShowsRequest) returns (ListLightShowsResponse);
message ListLightShowsResponse { repeated string paths = 1; }
```

- **An older app is safe.** It does not set `set_settings`, and the player
  does what it did: each entry's settings stay with its song. A test holds
  it.
- **An older player is visible.** Its playlists come with no `revision`,
  and the app leaves the controls out and says the player is older, as it
  does for the Lights tab. Every entry from a newer player carries
  `settings`, empty when it has none, so the two cannot be confused.
- **Someone else's edit** is caught by a revision, not by sending the
  settings back. `revision` is the first 64 bits of a SHA-256 over the
  songs and the settings as `GetPlaylist` reports them. A save with
  `check_expected` and an `expected_revision` reads the file, works out its
  revision the same way (auto play read as the player loads it, so `1.5s`
  and `1500ms` are the same), and refuses a mismatch as "the playlist has
  changed". It costs 16 bytes each way instead of every setting twice. An
  older app sends no revision and is checked by names alone, as before.
- **Fade details the app does not show** (curve, excluded tracks, lights)
  travel through untouched: the message carries them, the screen edits the
  length only, and a length changed keeps the rest.
- **Checked as the web UI's save is.** Settings go through
  `validate_for_write`; one it refuses is `INVALID_ARGUMENT` in its words
  (a length it cannot read, a fade longer than its song, a track the song
  does not have, a show outside the project). The app shows that message
  in the editor until something changes.
- **Only what changed is sent.** The app sets `set_settings` only when an
  entry's settings were edited, and `set_ambient` only when the
  playlist's was, so a reorder leaves the file's spellings alone.
- Refused while locked, like every playlist write.

The shows to pick from come from `ListLightShows`, open while locked and on
the Bluetooth allowlist: the `.light` files under the songs directory (the
web UI's lighting list) and in the project's `lighting/`, as
project-relative paths, which is how an ambient show is written. The web
UI's own ambient field is free text; the app's is too, with the list
beside it.

Sizes: fifty songs with every setting set and names padded long are about
24 kB answered and 25 kB sent, under half the 64 KiB frame. A test holds it.

The app's editor gains a settings button on each row, a line under the
song's name saying what it sets, an "Ambient light" row above the songs,
and a sheet for each, worded as the web UI's transitions panel.

## 3. Songs

A **Songs** screen: the library, searchable, and a song's page.

**Reading** is new but plain:

```proto
rpc ListSongs(ListSongsRequest) returns (ListSongsResponse);
rpc GetSong(GetSongRequest) returns (GetSongResponse);
```

A list row is a name, a duration and a few flags (loops, has lights, has
MIDI, failed to load). The list was to be sent gzipped in one answer;
it is paged instead (see "As built"). Production has about 50 songs, a few
kB.

**Writing** is a short list of named settings, not the song file:

```proto
rpc UpdateSongSettings(UpdateSongSettingsRequest) returns (GetSongResponse);
```

What it carries (Mike, 2026-11-11: "at least tracks, loops, crossfading,
samples"; the timeline and lighting are "too far"):

| Group | Settings | Notes |
|---|---|---|
| Looping | `loop_playback`, `loop_crossfade`, the tracks that cut over (`loop_crossfade_cut`) | Lengths as the player writes them, as in §2 |
| Tracks | Each track's name, file and file channel; add and remove | The name is what routes a track to an output, so renaming is how a track is moved. A file is picked from the song's own directory: the phone uploads nothing |
| Samples | The song's own samples: file, output, what a release and a retrigger do, the voice limit, the fade time; which MIDI event triggers which | Velocity layers are shown and kept, not edited |

Not carried: the tempo map, sections, the metronome, pilot cues, MIDI
playback, lighting shows, the timeline. The page shows that a song has
them.

Each group is its own optional part of the request; what is absent is left
alone, and what the phone does not edit inside a group (velocity layers)
travels through untouched. The answer carries a revision, as playlists do
(§2), so an edit made elsewhere is caught.

- **Why not the whole file.** The web UI's song editor writes YAML, and a
  phone that can do that can break a song in the middle of a show. Typed
  settings are checked by the player and cannot damage what they do not
  name.
- **The file keeps its shape.** The player changes the settings in the file
  it has and writes it back the way the web UI's save does, then reloads the
  songs.
- **While playing.** The REST save reloads songs at once. Whether a change
  to the song that is playing takes hold now, at the next pass, or at the
  next play needs checking against the player before the screen promises
  anything; the app will say which. (Checked: see "As built" below.)
- Refused while locked.

As built:

```proto
rpc ListSongs(ListSongsRequest) returns (ListSongsResponse);
rpc GetSong(GetSongRequest) returns (GetSongResponse);
rpc UpdateSongSettings(UpdateSongSettingsRequest) returns (GetSongResponse);

message ListSongsRequest { uint32 start = 1; }
message ListSongsResponse {
  repeated SongListing songs = 1;   // name, duration, loops, has_lighting,
                                    // has_midi, has_samples, failure,
                                    // base_dir (8)
  optional uint32 next = 2;         // more: ask again from here
  uint32 total = 3;
}

message GetSongResponse {
  string name = 1;
  google.protobuf.Duration duration = 2;
  optional string failure = 3;
  optional string read_only = 4;    // no file, or one that does not read
  SongLooping looping = 5;          // loop_playback, crossfade, cut
  SongTracks tracks = 6;            // name, file, file_channel
  SongSamples samples = 7;          // see below
  repeated SongTrigger triggers = 8;  // in words, read only
  SongOverview overview = 9;        // what the phone does not change
  repeated SongFile files = 10;     // audio files beside it, with channels
  string revision = 11;
  bool playing = 12;
  optional string base_dir = 13;    // as SongListing's
}

message SongSample {
  string name = 1;
  string original_name = 2;         // the name it was read as; empty: new
  optional string file = 3;
  repeated uint32 output_channels = 4;
  optional string output_track = 5;
  string release_behavior = 6;
  string retrigger = 7;
  optional uint32 max_voices = 8;
  optional uint32 fade_time_ms = 9;
  SampleVelocity velocity = 10;     // reported, ignored in an update
}

message UpdateSongSettingsRequest {
  string name = 1;
  string expected_revision = 2;
  SongLooping looping = 3;          // each group: absent, left alone
  SongTracks tracks = 4;
  SongSamples samples = 5;
}
```

- **Paged, not gzipped.** The Bluetooth envelope carries no compressed
  message, and gzip guarantees no size. A page holds at most 48 KiB of
  the 64 KiB frame; 200 songs with long names, each two long-named
  folders down, are 47.6 kB, one page (22 kB before `base_dir`). A test
  holds both. A bigger library takes more pages, not a bigger one.
- **Folders, as the web UI has them.** The web UI's Songs page
  (`SongList.svelte`) groups songs by `base_dir` from the song list (`ListSongs`)
  (the song's directory relative to the songs directory): each song goes
  under its `base_dir` less the last part, so `Live/2026/Encore` is under
  `Live/2026` and `Encore` is at the top. Every folder is one flat group
  labelled with its whole path, not a tree; the top comes first with no
  heading, the rest sorted by path (`localeCompare`); each heading shows
  the path in capitals and a count, and closes and opens its group, kept
  until the page goes. `ListSongs` and `GetSong` send `base_dir` as the
  web UI computes it (one function, `songs_api::base_dir`, for both),
  absent when the player has no songs directory to say it under.
- **The file keeps its shape.** The player reads the file as a YAML tree,
  changes only the keys whose values differ from what the file has, and
  writes it back through the web UI's song save, under the lock the song
  saves share, from the read to the reload. Key order, every other key,
  and a sample's velocity come through; comments do not, as they do not
  through the web UI's editor. A test round-trips a song with every
  feature (tempo, sections, metronome, pilot, lighting, MIDI, notification
  audio, velocity layers).
- **A sample is matched by the name it was read as**, so a rename keeps
  its velocity and anything else the phone does not show. Triggers are
  shown in the web UI's words and not changed: the web UI's song editor
  does not change them either.
- **Files.** A track or a sample may point at a file the song already
  names, or at an audio file in its folder, as `GetSong` lists them;
  anything else is refused, so there is nothing to traverse.
- **Checked as the player loads.** Besides the save's own checks, a song
  that loads is loaded as it would be before it is written (a crossfade
  it cannot read, a channel the file does not have, a file with several
  channels and none named, a cut track it does not have), and each
  playlist that plays it is checked to still load: a fade that leaves out
  a track renamed away would drop the playlist on reload. A song that
  failed to load already is written as asked, so it can be fixed from the
  phone. Refusals are `INVALID_ARGUMENT` in the loader's words.
- **While playing**, checked against the player: the web UI's save does
  not refuse a song that is playing (only its delete does), and neither
  does this. The file is written and the songs reloaded at once. The
  playback holds the song it started with, so the pass playing keeps its
  tracks, files, loop and crossfade (the audio engine takes them at play),
  and samples load when a song is selected or played. What changes at
  once is the transport: Play, Next and Stop ask the reloaded song whether
  it loops. Turn looping off on a looping song and Next no longer breaks
  out of it (Stop still ends it); turn it on on a song playing and Next
  breaks out of it as if it looped. A player test holds the first. The
  answer's `playing` says the song is the one playing, and the app says
  when a change takes effect.
- Reads are open while locked; the update is not.

The app's second tab is the **Library**: a **Playlist | Songs** switch at
its top, as the Lights tab switches between the plot and 3D, with the
playlist on one side and the song library on the other. A sixth tab would
shrink every label, and the library is the playlist's raw material: it is
where the editor's "add songs" picks from. (It was first a **Songs**
button beside **Edit**; Mike found a button in the playlist wonky.) The
switch keeps the side last shown across launches (`library.view`, as
`lights.view`), each side keeps its scroll while the other shows, and
while a playlist is edited the editor covers the tab, switch and all. The
Songs side groups songs as the web UI does, with folders that close and
open (kept until the player is forgotten); a search lists the songs that
match flat, each with its folder under its name; a player that sends no
`base_dir` shows one list. The editor's "add songs" picker stays a flat,
searched list, as the web UI's playlist editor's is. A song's page shows
its folder under its name, then what it has first,
then Looping (the switch on the page, the crossfade and its cut tracks in
a sheet), Tracks and Samples, each item a sheet, worded as the web UI's
song editor; one Save sends the groups changed, with the revision.

## 4. Wi-Fi

### 4.1 Who may connect

Decided (Mike, 2026-11-11): **match the web UI**. Anyone on the player's
network can use the app against it, as anyone there can open the web UI.
The player is meant to sit on its own network, away from a venue's.
Bluetooth keeps its pairing; it is the radio that reaches into a room.

### 4.2 How the app talks

Decided (Mike, 2026-11-11): **the app speaks gRPC over Wi-Fi.**

The messages the app sends over Bluetooth are already the gRPC messages,
inside an envelope that names the method. So the app's seam moves up one
level, from "send these bytes" to "call this method with these bytes", and
a second transport makes the call for real on the player's gRPC port.
Streams map the same way. Nothing above the seam changes, and the player
needs no new route.

The one gap is status. Over Bluetooth the player pushes a compact status
message. The gRPC `Status` is another message, and lacks four things the
app uses: the playlist position, the looping section, the gains revision
and the lock. So:

```proto
rpc WatchStatus(WatchStatusRequest) returns (stream RemoteStatus);
```

`RemoteStatus` is the message Bluetooth pushes today, moved to the player's
own proto so both carriers send one thing. It is sent when it changes and
at least once a second while playing, for the clock. The app keeps one
status path and polls nothing.

What differs from Bluetooth:

- Streams run unpaced. gRPC pushes back when the phone is slow, which
  Bluetooth cannot do. The stage can ask for the full 20 frames a second;
  assets arrive in a blink.
- The phone can call all of gRPC, not the Bluetooth allowlist. That matches
  §4.1; the app uses what it uses over Bluetooth.
- The lock applies as everywhere.

The alternative considered was the Bluetooth envelope over a websocket on
the web UI's port: no change to the app above its transport, but a new
route on the player to secure and explain, and a protocol only the app
speaks. Not chosen.

### 4.3 Finding the player

- **Told over Bluetooth.** A phone connected by Bluetooth asks the player
  for its addresses and the gRPC port, and remembers them. This needs no
  discovery protocol and works on networks that block one.
- **By name.** The Pi image already answers to `mtrack.local`. The app's
  connect screen gains "Enter an address", which takes a name or an IP.
- **mDNS browsing** (the player advertising a service the app lists) is the
  nicest and the most work: a service advert on the player, a plugin in the
  app, the local-network permission prompt on iOS. Left for after the first
  two have been used.

### 4.2.1 As built (player side)

- `RemoteStatus` is in `player.proto` with the field numbers Bluetooth used for
  `ble.v1.Status`, which is gone from `ble.proto`; the bytes on the air are
  unchanged. `WatchStatusRequest` has no fields.
- One reader serves both carriers: `controller/remote_status.rs` builds the
  status from the player and decides when one is due. Bluetooth sends at
  250 ms while playing, `WatchStatus` at one second. Each stream polls for
  itself and keeps only the newest due status (a watch channel), so a slow
  client skips ahead and never holds the player up. The poll ends when the
  stream is dropped.
- `WatchStatus` is open while locked and not on the Bluetooth allowlist.
- `GetNetworkInfo(GetNetworkInfoRequest) returns (GetNetworkInfoResponse)`:
  `hostname = 1`, `repeated string addresses = 2` (IPv4 first, then IPv6; no
  loopback, link-local or multicast), `optional uint32 grpc_port = 3`. The
  port is the one the gRPC controller bound, recorded on the player while it
  listens and cleared when it stops, so a port of 0 in the config reports
  what the OS gave. It is on the allowlist and open while locked. The
  addresses come from the `if-addrs` crate.
- Not done: a Wi-Fi interface is not told apart from any other, and the host
  name is the one the config uses for profiles (`MTRACK_HOSTNAME` wins).

### 4.2.2 As built (the app)

- The seam is `PlayerChannel` (`app/lib/src/protocol/player_channel.dart`): `call(method,
  requestBytes, {timeout}) -> responseBytes`, `openStream(method, requestBytes)`, `statusStream` and
  `readStatus()`, the link's `state`, `hasStreams`, `callsInFlight`, `carrier` and `dispose()`.
  `EnvelopeChannel` is the old `PlayerClient` engine, unchanged in behaviour; `GrpcChannel` is the
  new carrier; `PlayerClient` keeps the typed calls over either. Errors are `PlayerException`s with
  the same gRPC codes on both, so the screens' messages did not change.
- `GrpcChannel` is `package:grpc` 5.2.0 (Apache-2.0; it brings `http2`, BSD-3-Clause), plaintext,
  with no generated stubs: a method is `/player.v1.PlayerService/<Name>` and its bytes. It opens
  by taking the first message of `WatchStatus` (two seconds); that stream then stays open as the
  app's one status path. gRPC's own keepalive was tried and left out: in 5.2.0 a ping timeout
  `finish()`es the connection, which waits for the open streams that never end, so a dead link was
  not noticed. A heartbeat takes its place (`GetNetworkInfo` every 8 s, 5 s to answer; a timeout
  or a broken connection takes the channel down).
- `RememberedPlayer` gained `hosts`, `port`, `hostname` and `bluetooth` (false for a player known
  only by an address, whose id is `wifi:<host>:<port>`). Connecting tries up to three hosts for two
  seconds each, then Bluetooth; a Bluetooth connection asks `GetNetworkInfo` and rewrites the hosts
  (the player's numbers replace the old ones; a name the person typed stays, first). Every Wi-Fi
  connection compares the reported host name with the remembered one and closes a mismatch. A
  player that answers `UNIMPLEMENTED` is marked old for the run and not tried again. While on
  Bluetooth with hosts known, a probe every 20 s connects in the background and, if it answers and
  nothing is in flight (up to 10 s of patience), swaps the connection over as a reconnect.
- The stage asks for the player's own rate over Wi-Fi (`max_rate` 0) and 10 over Bluetooth.
  The bar shows a Wi-Fi or Bluetooth mark; the settings sheet says "Connected over ...".
- Not done: binding to the Wi-Fi network on Android (§7); mDNS browsing; nothing here has run on a
  phone.

### 4.4 Which radio

The app prefers Wi-Fi when the player answers there, and falls back to
Bluetooth when it does not, without the user choosing. The status dot says
which is in use. A player known only by address (never paired) is Wi-Fi
only.

Switching mid-session is a reconnect like any other: the app already
survives those.

### 4.5 The phone

- Android: the app has the INTERNET permission since the Lights tab. Dart's
  own sockets are not bound by the cleartext rule that limits the web view
  to `127.0.0.1`.
- iOS: needs the local-network usage string. Unverified, like everything on
  iOS.

## 5. The web UI and gRPC

Mike asked whether the web UI should use gRPC too. Part of it does: the
web server answers gRPC-Web on its own port, and the transport buttons,
seek, section loops, track gains and the ambient mode already call
`PlayerService` through it. What does not:

| | Today | Over gRPC? |
|---|---|---|
| Live feeds: status, the stage | JSON on a websocket | **Yes.** `WatchStatus` and `WatchStage` are the same facts in one encoding, and the stage page already reads `WatchStage`. This retires the JSON twins the light view doc named as a debt |
| Other websocket traffic: meters, logs, waveforms | JSON on the websocket | Later, one by one, when each has a reason |
| Editing: config, songs, playlists, lighting files, uploads (67 REST routes) | REST | **No.** It works, it is tested, files and uploads suit HTTP, and a rewrite buys nothing a user would see |

So: move the live feeds once `WatchStatus` exists, and leave the editing
API alone. It is its own piece of work (P5) and nothing on the phone waits
for it.

**As built: the live stage feed has moved.** The web UI's stage (plot and 3D)
reads `WatchStage` over gRPC-Web, and the websocket's JSON `metadata` and
`state` messages are gone, on the player and in the web UI. The stream is one
per page, opened when something first shows the stage and closed five seconds
after the last thing that does lets go; it asks for the player's own rate. The
venues directory the JSON named (`dir`) is not on the stream, and nothing
needs it: every venue save reads and writes the file through the REST API,
which defaults to the configured directory. Geometry arrives as 32-bit floats
(shown as the shortest decimal that is the float), so the editors take the
numbers they write from the file they just read, not from the stage. Measured
before the switch, side by side in one browser for 50 s with a show running:
763 frames on each feed, a median gap of 50 ms on both, and the stream's 763
messages arrived in 767 network chunks (no batching). What remains on the
websocket is the status, logs, waveforms and reload notices (the rest of this
section).

## 6. Order

| Phase | What | Size |
|---|---|---|
| P1 | Playlist settings: messages, player, the app's sheets | Medium |
| P2 | Songs: list, a song's page, loop settings | Medium |
| P3 | Wi-Fi: `WatchStatus`, the app's call-level seam and gRPC transport, the address told over Bluetooth, "Enter an address" | Medium |
| P5 | The web UI's live feeds over gRPC (§5) | Medium, separate |
| P4 | mDNS browsing | Small, optional |

P1 and P2 are useful over Bluetooth today. P3 touches neither, so it can be
built beside them.

## 7. What is not known

- ~~How a song edit reaches a song that is playing (§3).~~ Checked: at
  the next play, but its loop flag at once for the transport (§3, as
  built).
- iOS, throughout.
- Wi-Fi where the phone has mobile data and the player's network has no
  internet: Android keeps mobile data as the default network until the
  user chooses to stay connected to the Wi-Fi. Decided (Mike, 2026-11-12):
  that choice is the answer, and the app and docs say so; the app does not
  bind its sockets. Not yet run on such a network.

## 8. Decisions

1. **Wi-Fi access matches the web UI** (§4.1). Decided.
2. **The app speaks gRPC over Wi-Fi, with a `WatchStatus` stream** (§4.2).
   Decided.
3. **Song edits are named settings, not the file** (§3). Recommended: yes.
4. **Which song settings** (§3): looping and its crossfade, tracks,
   samples; not the timeline or lighting. Decided.
5. **How far the web UI moves to gRPC** (§5). Open: Mike wants to talk it
   through before deciding.
6. **Finding the player: told over Bluetooth and by address first, mDNS
   browsing later** (§4.3). Recommended: yes.
