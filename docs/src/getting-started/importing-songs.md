# Importing Songs

The web UI's **Songs** page is the easiest way to get songs into mtrack.

## Creating a New Song

Click **New Song** and enter a name. You can use a path like `Artist/Song Name` to create
nested directory structures. mtrack creates the directory and an empty `song.yaml` for you.

From the song detail page, you can then upload audio files, assign tracks to output channels,
add MIDI playback, and configure lighting.

## Importing from the Filesystem

If you already have audio files on disk, click **Import** to browse the server filesystem.
When you select a directory, mtrack auto-detects:

- **Audio files** (WAV, FLAC, MP3, OGG, AAC, M4A, AIFF) as tracks
- **MIDI files** as MIDI playback
- **`.light` files** as lighting shows
- **`dmx_` prefixed MIDI files** as MIDI-based DMX light shows

A `song.yaml` is generated automatically from the detected files.

## Bulk Import

To import many songs at once, use the **bulk import** option. Select a parent directory and
mtrack imports all subdirectories as songs. The scan is recursive, so nested structures like
`artist/album/song` are handled automatically.

## Editing a Song

Click any song to open its detail page, where you can:

- Add, remove, or reorder audio tracks
- Upload new audio or MIDI files
- Configure MIDI playback and channel exclusions
- Edit lighting shows with the visual timeline editor or raw DSL
- Configure per-song samples

## How Songs Are Stored

Each song is a directory containing a `song.yaml` file alongside its audio, MIDI, and lighting
files. For full details on the YAML format, see [Song Configuration (YAML)](../configuration/song-config.md).
