# Playlists

## Playlist Files

Playlists are YAML files that define an ordered list of songs to play:

```yaml
songs:
- Sound check
- A really cool song
- Another cool song
- The slow one
- A really fast one
- Outro tape
```

Song names must match the `name` field in each song's `song.yaml`.

## Multiple Playlists

mtrack supports multiple playlists stored as individual YAML files in a `playlists/` directory
(configurable via `playlists_dir` in `mtrack.yaml`). Each playlist is named after its filename
stem — `my_setlist.yaml` becomes the playlist "my_setlist".

The `all_songs` playlist is always present and auto-generated from the song repository,
sorted alphabetically. It cannot be deleted or manually edited.

## Active Playlist

The active playlist determines which songs are available for playback. Switch between playlists
using:

- The **dashboard playlist dropdown** in the web UI
- The **playlist editor's Activate button**
- The `SwitchToPlaylist` gRPC RPC
- MIDI/OSC `Playlist` and `AllSongs` events

The active playlist choice is persisted in `mtrack.yaml` (via the `active_playlist` field)
and restored on restart. Switching to `all_songs` is session-only — on restart, the player
returns to the last persisted playlist.

## Legacy Playlist

For backward compatibility, mtrack also supports a single legacy `playlist.yaml` file
(specified via command line or the `playlist` field in `mtrack.yaml`). This is loaded as
the playlist named "playlist" if no file with that name exists in the `playlists/` directory.

If no playlist file is provided at all, mtrack falls back to the `all_songs` playlist.

## Managing Playlists

Playlists can be created, edited, and deleted from the web UI's **Playlist Editor** page.
The editor provides drag-and-drop song ordering and a searchable list of available songs.

When a song is deleted from the repository, it is automatically removed from all playlist
files on disk to prevent broken references.
