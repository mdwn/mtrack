# Bluetooth remote, phase 2: playlists, levels, lights

*Design doc, draft 1 — 2026-11-10. Follows the first version of the app
(`design/bluetooth_remote_design.md`, PRs #700–#709), which Mike ran on a Pixel
against the Pi 4 rig the same day. He asked for four things next: playlist
editing, track levels, ambient lighting controls, and a view of the lights. This
doc says what each needs, in what order, and what is his to decide (§9).*

## 1. Where things stand

The app can do what the Bluetooth allowlist lets it: transport, playlist
navigation, sections. Everything a phone calls goes through the in-process
gRPC `PlayerService`, under the same lock as every other surface.

What the player already has for the four features differs a great deal:

| Feature | On the player today | Missing |
|---|---|---|
| Track levels | `GetTrackGains`, `SetTrackGain`, `SetTrackMute` over gRPC, open while locked | Only the allowlist entry |
| Ambient light | `SetAmbientMode` (auto / on / off), `AmbientStatus` in the gRPC status | Allowlist entry; the phone's status has no ambient field |
| Playlist editing | REST and MCP only; refused while locked | Any gRPC write; a way round the lock (§4) |
| Light view | A JSON websocket for the web UI, up to 20 frames a second | Any gRPC source; anything small enough for the radio |

So the order is levels and ambient first (small), then playlists (new RPCs and
one real decision), then the light view (new player code, and the one that may
not be worth it).

## 2. Track levels

A **Levels** screen: one row per output track with a fader and a mute.

- Range is the player's: −60 dB (silent) to +12 dB, in dB. The fader is
  detented at 0 dB.
- The allowlist gains `GetTrackGains`, `SetTrackGain`, `SetTrackMute`.
- Gains are per hardware profile and the player saves them itself, 750 ms after
  the last change. Mute is not saved: a restart unmutes. The app says so
  nowhere; it is how the web UI behaves too.
- While a fader moves, the app sends at most one `SetTrackGain` per track every
  100 ms and always the last value. The player's answer carries the gain it
  applied, which is what the fader shows on release.
- Another surface can move a gain. The phone's status gains a small
  `gains_revision` counter the player bumps on any gain or mute change; the app
  fetches the gains again when it moves and the screen is open.

Not in this phase, because the player has none of them: live meters, a master
fader, solo.

## 3. Ambient light

The player's ambient control is a three-way mode, and that is what the app
gets: **Auto** (plays between songs), **On** (plays now, over a song too),
**Off** (dark between songs).

- The allowlist gains `SetAmbientMode`.
- The phone's status gains `ambient_mode` and `ambient_state` (playing,
  pending, off, unavailable), a few bytes, so the control shows the truth
  without a call. When no ambient show is configured the control is disabled
  and says why.
- It lives on the Now screen's settings sheet for now, and moves to the Lights
  tab if §5 is built.
- The mode is not saved by the player: it starts in Auto. Unchanged.

There is no brightness, no choice of show at run time and no fade override in
the player. Each would be an engine feature first. See decision 3.

## 4. Playlist editing

### What the app does

- Make a playlist, delete one, and edit one: add songs from the library, remove
  them, drag to reorder.
- The library is `GetPlaylist("all_songs")`, which is already allowed: name and
  duration per song, about 40 bytes each, so some 1,500 songs fit one frame.
- Not in this phase: per-entry fades, auto-play and ambient overrides. An edit
  keeps whatever a file already has for each song, as the web UI's save does.
  Renaming is not there either; the player has no rename.

### What the player needs

Two RPCs, on the existing playlist write path (one write lock, validation,
atomic file write, `reload_songs`):

```proto
rpc SavePlaylist(SavePlaylistRequest) returns (GetPlaylistResponse);
rpc DeletePlaylist(DeletePlaylistRequest) returns (DeletePlaylistResponse);

message SavePlaylistRequest {
  string name = 1;
  repeated string songs = 2;
  // The songs the caller last saw, in order. When set and the file differs,
  // the save is refused (FAILED_PRECONDITION): someone else edited it.
  repeated string expected = 3;
  bool create = 4;  // refuse if the playlist exists
}
```

`expected` is the same guard `NavigateToSong` has: two people editing, or the
web UI and a phone, do not silently overwrite each other.

Editing while a song plays is allowed, as it is from the web UI. The playing
song stays current even if the edit removes it.

### The lock

This is the real question. Playlist writes are refused while the player is
locked, the player starts locked, and only the web UI can unlock it. On a Pi
with no network, which is what this app is for, a phone could therefore never
edit a playlist.

Three ways out:

1. **The phone can lock and unlock** (recommended). A `SetLock` RPC on the
   allowlist, and a lock control in the app that takes a deliberate press. The
   lock keeps its meaning on every surface, and a paired phone is already
   trusted to stop the show.
2. **Playlist edits from a phone ignore the lock.** Simpler, but the lock stops
   meaning "nothing changes during the show" for one surface.
3. **Leave it.** Editing from the phone only when someone unlocked from the web
   UI first. Honest, but useless at a venue.

## 5. A view of the lights

### Does it fit the radio?

The web UI's feed does not: JSON, every channel of every fixture by name, up to
20 frames a second.

A reduced feed does. If the player works out each fixture's colour itself and
sends three bytes per fixture:

| | Size | At 10 frames/s |
|---|---|---|
| 50 fixtures | 150 B a frame | 1.5 kB/s |
| 200 fixtures | 600 B a frame | 6 kB/s |
| Layout, once: name and position per fixture | about 40 B each | n/a |

These are estimates from the message shapes, not measurements. The link at the
interval the app now asks for carries tens of kB/s on Android, less on iOS, and
shares it with status and answers. Fifty fixtures at ten frames a second is
comfortable; two hundred wants five.

### What it would be

A top-down plot: one dot per fixture at its venue position (or the web UI's tag
layout when a venue has no positions), filled with its current colour and
brightness. Sent only while the Lights tab is open, only on change, on a fourth
characteristic so a slow phone drops frames instead of delaying a Stop.

No beams, no pan and tilt, no 3D, no per-cell pixels, no scenery.

### What it costs

This is the expensive one, and all of it is player work:

- The colour of a fixture is worked out in the browser today (`rig.ts`). It
  would be written again in Rust and kept in step with the web UI.
- A new notify characteristic and its pump, a layout call, and tests for both.

It is a monitor, not a control. Its use is seeing from the stage that the rig
is doing what the show says, without a laptop. Whether that is worth the work
is decision 4. Dropping it costs nothing else in this doc.

## 6. The app's shape

Three tabs become four or five: **Now**, **Playlist**, **Sections**,
**Levels**, and **Lights** if §5 is built. Playlist editing is an Edit mode on
the Playlist tab, not a tab of its own. Five tabs is the most a phone's bar
holds well; mockups will settle whether Sections folds into Now.

## 7. Protocol changes

- Allowlist: `GetTrackGains`, `SetTrackGain`, `SetTrackMute`, `SetAmbientMode`,
  `SavePlaylist`, `DeletePlaylist`, and `SetLock` under decision 1.
- `ble.v1.Status`: `gains_revision`, `ambient_mode`, `ambient_state`, `locked`.
  All small, all optional, so the first version of the app ignores them.
- An old app against a new player, and a new app against an old player, both
  keep working: a refused method is already reported as "not allowed from a
  phone", and the app hides what the player does not offer.

## 8. Phasing

| Phase | What | Player work |
|---|---|---|
| C1 | Levels and ambient | Allowlist, three status fields |
| C2 | Playlist editing | Two RPCs, the lock decision |
| C3 | Light view | Colour reduction in Rust, a characteristic |

Each is a player PR and an app PR, mockups first for C1 and C2 together.

## 9. Decisions

1. **The lock** (§4). Recommended: the phone can lock and unlock.
2. **Playlist editing scope.** Recommended: songs and order only in this
   phase; fades, auto-play and per-entry ambient later.
3. **Ambient scope.** Recommended: the three-way mode only. Brightness or
   choosing a show would be an engine feature first; say if either is what
   "ambient controls" meant.
4. **The light view.** Build the reduced plot (§5), or leave it out.
   Recommended: decide after C2, when the cheap things are in hand.
