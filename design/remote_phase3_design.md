# Phone remote, phase 3: playlist settings, songs, Wi-Fi

*Design doc, draft 1 — 2026-11-11. Follows the light view
(`design/bluetooth_light_view_design.md`, PRs #717–#725), which Mike ran on
his Pixel, plot and 3D. He asked for three things next: the playlist editor
to reach the settings the web UI's does (transitions, ambient light), the
song library with enough editing to debug on the fly (looping first), and
Wi-Fi beside Bluetooth. This doc says what each needs, in what order, and
what is his to decide (§7).*

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
MIDI, failed to load). The list is sent gzipped in one answer; production
has about 50 songs, a few kB.

**Writing** is a short list of named settings, not the song file:

```proto
rpc UpdateSongSettings(UpdateSongSettingsRequest) returns (GetSongResponse);
```

The first version carries looping: `loop_playback`, `loop_crossfade`, and
the tracks that cut over (`loop_crossfade_cut`). Each is optional in the
request; what is absent is left alone.

- **Why not the whole file.** The web UI's song editor writes YAML, and a
  phone that can do that can break a song in the middle of a show. A named
  setting is checked by the player and cannot damage what it does not name.
- **The file keeps its shape.** The player changes the setting in the file
  it has and writes it back the way the web UI's save does, then reloads the
  songs.
- **While playing.** The REST save reloads songs at once. Whether a change
  to the song that is playing takes hold now, at the next pass, or at the
  next play needs checking against the player before the screen promises
  anything; the app will say which.
- Refused while locked.

What else belongs in the list is Mike's to say (§7). Candidates the player
already has as plain settings: sections (add, move, rename), the metronome
on or off, a track's output. Each is its own field and can come later.

## 4. Wi-Fi

### 4.1 Who may connect

Decided (Mike, 2026-11-11): **match the web UI**. Anyone on the player's
network can use the app against it, as anyone there can open the web UI.
The player is meant to sit on its own network, away from a venue's.
Bluetooth keeps its pairing; it is the radio that reaches into a room.

### 4.2 How the app talks

The app's seam is lower than it looked. `PlayerTransport` moves the
Bluetooth envelope's bytes: requests out, answers, status and stream
messages in. `PlayerClient` and everything above it know nothing of the
radio.

Two ways to add Wi-Fi:

| | A: the envelope over a websocket | B: a gRPC client in the app |
|---|---|---|
| Player | One new route on the web UI's port, feeding the dispatcher Bluetooth already uses | Nothing |
| App | One new transport; no screen or client code changes | A second client for every call, a second status mapping, the gRPC library |
| Status | Pushed, the same compact message | Polled, a different message |
| Streams | The same code, without the pace | gRPC's own |
| What the phone can call | The Bluetooth allowlist | Everything gRPC has |

**Recommended: A.** One protocol, two carriers. Every feature built for
Bluetooth works over Wi-Fi the day it lands and the other way round, and
the two cannot drift. The route lives on the web UI's port (8080), so there
is no new port to open or document.

Over the websocket:

- One websocket message is one envelope message; nothing is cut into
  packets.
- Streams run unpaced. The socket pushes back when the phone is slow, which
  Bluetooth cannot do, and newest-wins still drops stale lighting frames.
- The stage can ask for the full 20 frames a second; assets arrive in a
  blink.
- The lock applies as it does over Bluetooth.

### 4.3 Finding the player

- **Told over Bluetooth.** A phone connected by Bluetooth asks the player
  for its addresses and the web port, and remembers them. This needs no
  discovery protocol and works on networks that block one.
- **By name.** The Pi image already answers to `mtrack.local`. The app's
  connect screen gains "Enter an address", which takes a name or an IP.
- **mDNS browsing** (the player advertising a service the app lists) is the
  nicest and the most work: a service advert on the player, a plugin in the
  app, the local-network permission prompt on iOS. Left for after the first
  two have been used.

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

## 5. Order

| Phase | What | Size |
|---|---|---|
| P1 | Playlist settings: messages, player, the app's sheets | Medium |
| P2 | Songs: list, a song's page, loop settings | Medium |
| P3 | Wi-Fi: the websocket route, the transport, the address told over Bluetooth, "Enter an address" | Medium |
| P4 | mDNS browsing | Small, optional |

P1 and P2 are useful over Bluetooth today. P3 touches neither, so it can be
built beside them.

## 6. What is not known

- How a song edit reaches a song that is playing (§3).
- Whether the web UI's server can hold the websocket route without the
  origin checks it applies to its own (§4.2): a phone app has no origin.
- iOS, throughout.
- Wi-Fi where the phone has mobile data and the player's network has no
  internet: Android may route the app's traffic to mobile data unless the
  socket is bound to the Wi-Fi network. To test early in P3.

## 7. Decisions

1. **Wi-Fi access matches the web UI** (§4.1). Decided.
2. **The envelope over a websocket, not a gRPC client** (§4.2).
   Recommended: yes.
3. **Song edits are named settings, not the file** (§3). Recommended: yes.
4. **Which song settings beyond looping** (§3). Open: Mike's list.
5. **Finding the player: told over Bluetooth and by address first, mDNS
   browsing later** (§4.3). Recommended: yes.
