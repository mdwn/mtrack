# Importing Songs

The web UI's **Songs** page is the easiest way to get songs into mtrack. Importing writes
files, so [unlock the web UI](quick-start.md#3-unlock-the-web-ui) first.

## Creating a New Song

Click **New Song** and enter a name. You can use a path like `Artist/Song Name` to create
nested directory structures. mtrack creates the directory and an empty `song.yaml` for you.

From the song detail page, you can then upload audio files, name the tracks, add MIDI
playback, and configure lighting. Which output each track plays on is set in your
[hardware profile](hardware-config.md#track-mappings).

## Importing from the Filesystem

If you already have audio files on disk inside the project directory, click **Import from
Filesystem** on the **Songs** page. The file browser is limited to the project directory (the
one containing `mtrack.yaml`), so copy your files there first. Navigate to the song's
directory and click **Use This Directory**, check the song name, and click **Create Song**.
mtrack auto-detects:

- **Audio files** (WAV, FLAC, MP3, OGG, AAC/M4A, AIFF) as tracks
- **MIDI files** as MIDI playback
- **`.light` files** as lighting shows
- **`dmx_` prefixed MIDI files** as MIDI-based DMX light shows

A `song.yaml` is generated automatically from the detected files.

## Naming your tracks

Each audio file becomes one track, named after the file with the extension removed and
converted to lowercase words joined by hyphens: `Backing Track.wav` becomes `backing-track`.
Stereo files are split into two tracks, `<name>-l` and `<name>-r`, and files with more than two
channels into `<name>-1`, `<name>-2`, and so on. If two tracks end up with the same name, the
later ones get a `-2`, `-3` suffix.

These names matter in two places:

- **Track mappings.** The names in your profile's track mappings must match the song's track
  names exactly, or the track is silent. See
  [Track Mappings](hardware-config.md#track-mappings).
- **The click track.** A track named exactly `click` is special: mtrack analyzes it to find the
  beats and measures of the song (the beat grid), unless the song defines its own `tempo:`. A
  stereo file would be split into `click-l` and `click-r`, so use a mono file named
  `click.wav`, or rename the track on the song's detail page.

Name your files for what they are (`click`, `drums`, `bass`, `backing`) before you import, or
rename the tracks afterwards on the song's detail page.

## Bulk Import

To import many songs at once, navigate to a parent directory and click **Import All
Subdirectories**. mtrack imports every subdirectory as a song, skipping any that already have
a `song.yaml`. The scan is recursive, so nested structures like `artist/album/song` are
handled automatically.

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
