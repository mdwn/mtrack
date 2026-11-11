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

```proto
message PlaylistEntry {
  string song_name = 1;
  google.protobuf.Duration duration = 2;
  EntrySettings settings = 3;      // absent from an older player
}

message EntrySettings {
  Fade fade_out = 1;
  Fade fade_in = 2;
  AutoPlay auto_play = 3;
  AmbientSetting ambient = 4;
}

message SavePlaylistRequest {
  // 1–4 as today
  repeated EntrySettings settings = 5;   // one per song, in order
  bool set_settings = 6;                 // absent: keep them, as today
  AmbientSetting ambient = 7;
  bool set_ambient = 8;
}
```

- **An older app is safe.** It does not set `set_settings`, and the player
  does what it does today: each entry's settings stay with its song.
- **An older player is visible.** `GetPlaylist` answers with no `settings`,
  and the app leaves the controls out and says the player is older, as it
  does for the Lights tab.
- **Someone else's edit** is caught as it is today (`expected`), widened to
  cover settings: the app sends back what it last saw.
- **Fade details the app does not show** (curve, excluded tracks, a
  separate length for lights) travel through untouched: the message carries
  them, the first version of the screen edits the length only.
- Refused while locked, like every playlist write.

The app gets a settings sheet per song in the editor, and one for the
playlist. Ambient needs the list of `.light` files to pick a show from;
`ListPlaylists` is not the place, so a small `ListAmbientShows` or a field
on the playlist answer. To settle while building.

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
