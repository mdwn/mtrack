# Playlists

## Managing Playlists

Playlists can be created, edited, and deleted from the web UI's **Playlist Editor** page.
The editor provides drag-and-drop song ordering and a searchable list of available songs.

The `all_songs` playlist is always present and auto-generated from the song repository,
sorted alphabetically. It cannot be deleted or manually edited.

## Active Playlist

The active playlist determines which songs are available for playback. In the web UI, switch
between playlists with the **dashboard playlist dropdown** or the **playlist editor's Activate
button**. Other control interfaces can switch too; see [See also](#see-also).

The active playlist choice is persisted in `mtrack.yaml` (via the `active_playlist` field)
and restored on restart. Switching to `all_songs` is session-only — on restart, the player
returns to the last persisted playlist.

## Song Deletion

When a song is deleted from the repository, it is automatically removed from all playlist
files on disk to prevent broken references.

## Playlist File Format

Playlists are YAML files stored in a `playlists/` directory (configurable via `playlists_dir`
in `mtrack.yaml`). Each playlist is named after its filename stem — `my_setlist.yaml` becomes
the playlist "my_setlist".

```yaml
kind: playlist
songs:
- Sound check
- A really cool song
- Another cool song
- The slow one
- A really fast one
- Outro tape
```

Song names must match the `name` field in each song's `song.yaml`.

### Legacy Playlist

For backward compatibility, mtrack also supports a single legacy `playlist.yaml` file
(specified via command line or the `playlist` field in `mtrack.yaml`). This is loaded as
the playlist named "playlist" if no file with that name exists in the `playlists/` directory.

If no playlist file is provided at all, mtrack falls back to the `all_songs` playlist.

## Transitions

Breaking out of a looping song (Play, Next or Previous) normally cuts it in 10 ms. A playlist
entry can instead have its song play on and fade out under the song it gives way to -- a prelude
carrying the room into the song it introduces -- and an entry can have its song fade in.

```yaml
kind: playlist
songs:
- song: Walk-in loop
  fade_out:
    length: 2measures
    curve: linear
    exclude: [click, cues]
    lights: 1measure
- song: Opener
  fade_in: {length: 4beats, exclude: [click]}
- The slow one
```

An entry is a song's name, or a map with `song` and its own `fade_out` and `fade_in`; an entry
without them cuts and starts as usual. The same song can fade differently in different
playlists, or at different places in one. A misspelt setting is an error, as is an `exclude`
naming a track the entry's song does not have.

- **`fade_out`** applies when a loop break leaves the entry's song: the next song starts at once,
  as always, while the looping song plays on from where it was and fades out underneath it.
  It applies only to loop breaks; a song that ends by itself just ends.
- **`fade_in`** applies when the entry's song starts under a song fading out: it has no effect
  otherwise.

Either can be a length alone or a map:

- **`length`**: `3s`, `500ms`, `6beats` or `1measure`. A `fade_out`'s beats and measures count on
  the **next** song's grid from its first bar -- any lead-in before it included -- so `1measure`
  fades out exactly over the next song's count-in, whatever its tempo and meter. A song with no
  grid of its own (a prelude usually has none) needs the next song to have one; failing both,
  the song cuts. A `fade_in`'s count on
  its own song's grid.
- **`curve`**: `equal_power` (the default) holds the level up and falls away late on a
  fade-out, and comes up quickly on a fade-in; `linear` is a straight ramp.
- **`exclude`**: tracks that do not fade. On a fade-out they cut at once; on a fade-in they start
  at full level -- usually the click and cues, so the count-in is heard once and clearly. A
  `fade_out` defaults to the song's `loop_crossfade_cut` tracks, a `fade_in` to none.
- **`lights`** (`fade_out` only): how long the lights take to cross from the looping song's show
  into the next song's, `0s` to switch at once. Defaults to the length.

A source part way through a loop crossfade fades out from where it is. A fade that outlasts the
pass it began in carries on into as many more passes as it takes, crossing pass to pass over the
song's `loop_crossfade` as its loop does. MIDI from the looping song stops
at the break. Stop or a seek cuts everything, the fading song included.

The web UI's playlist editor keeps a playlist's transitions when it saves, and will not overwrite
a playlist file that does not read; transitions are edited in the file.

## Ambient light

The profile's ambient light -- the show the rig plays between songs, see
[Lighting Configuration](../lighting/configuration.md#ambient-light) -- can be set per playlist
with a top-level `ambient:`. Its settings override the profile's one by one, so a playlist
can swap the show and keep the timings:

```yaml
kind: playlist
ambient:
  show: lighting/festival_ambient.light
  fade_in: 6s
songs:
- Opener
- The slow one
```

`ambient: false` turns it off for the playlist. Switching playlists while nothing plays crosses
straight into the new playlist's ambient light, or fades the old one out.

## See also

Playlists can also be switched from outside the web UI:

- [gRPC Control](../interfaces/grpc.md) — the `SwitchToPlaylist` RPC and the `mtrack` CLI client
- [OSC Control](../interfaces/osc.md) — OSC `playlist` and `all_songs` events
- [Hardware Profiles](../configuration/hardware-profiles.md) — mapping MIDI `playlist` and `all_songs` events in a MIDI controller
