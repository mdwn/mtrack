# Song Repository

## Song Repository

The song repository is a location on disk that houses your backing tracks, MIDI files, and song
definitions. mtrack recursively scans the repository for `song.yaml` files, so songs can be
organized in any directory structure — flat, by artist, by album, or any other scheme.

The repository path is configured via the `songs` field in `mtrack.yaml`. For zero-config
startup (no existing `mtrack.yaml`), it defaults to `.` (the project root directory).

## Songs

A song comprises of:

- One or more audio files.
- An optional MIDI file.
- One or more light shows (using `.light` DSL files, or MIDI files interpreted as DMX).
- A song definition (`song.yaml`).

The audio files must all be the same sample rate. They do not need to be the same length. mtrack
will play until the last audio (or MIDI) file is complete.

Supported audio formats: WAV, FLAC, MP3, OGG, AAC, M4A, AIFF.

A song is defined in a `song.yaml` file:

```yaml
# Identifies this file as a song configuration.
kind: song

# The name of the song. This name is primarily used when constructing
# playlists for mtrack.
name: The Song Name

# An optional MIDI event to emit when the song is selected on the
# playlist. This will occur even if the song is not playing. This is
# useful to trigger events on a remote device, such as a MIDI controller.
midi_event:
  type: program_change
  channel: 16
  program: 3

# Light shows using the new DSL format (.light files).
# These files use the lighting DSL and can reference logical groups from mtrack.yaml.
lighting:
  - file: lighting/main_show.light
  - file: lighting/outro.light

# An optional MIDI playback configuration.
midi_playback:
  file: Song Automation.mid

  # MIDI channels from the MIDI file to exclude.
  exclude_midi_channels:
  - 15

# The tracks associated with this song.
tracks:
- name: click
  file: click.wav
- name: cue
  file: /mnt/song-storage/cue.wav
- name: backing-track-l
  file: Backing Tracks.wav
  file_channel: 1
- name: backing-track-r
  file: Backing Tracks.wav
  file_channel: 2
- name: keys
  file: Keys.wav
  file_channel: 1
```

We can test our song repository with the `mtrack songs` command:

```
$ mtrack songs /mnt/song-storage
Songs (count: 23):
- Name: The first really cool song
  Duration: 5:10
  Channels: 11
  ...
```

## Generating Default Song Configurations

Song configurations can be generated using the `songs` command:

```
$ mtrack songs --init /mnt/song-storage
```

This creates a `song.yaml` in each subfolder of `/mnt/song-storage`. The name of the
subfolder determines the song's name. Audio files are used as tracks (stereo and multichannel
files are split into per-channel tracks). MIDI files are used as MIDI playback, and files
prefixed with `dmx_` are treated as MIDI DMX light shows. `.light` files are auto-detected as
DSL lighting shows.

## Directory Structure

Songs can be organized in any directory structure. mtrack recursively scans for `song.yaml`
files:

```
songs/
├── Song One/
│   ├── song.yaml
│   ├── click.wav
│   └── backing.flac
├── Artist/
│   ├── Album/
│   │   ├── Song Two/
│   │   │   ├── song.yaml
│   │   │   └── tracks.wav
│   │   └── Song Three/
│   │       ├── song.yaml
│   │       └── tracks.wav
│   └── Single/
│       ├── song.yaml
│       └── audio.mp3
└── Covers/
    └── Cover Song/
        ├── song.yaml
        └── audio.wav
```

## Managing Songs via the Web UI

The web UI song browser provides a complete management interface:

- **Create** — Create new songs with a name or nested path (e.g. `Artist/Song`).
- **Import** — Browse the server filesystem and import existing song directories. Audio, MIDI,
  and lighting files are auto-detected and the `song.yaml` is generated automatically.
- **Bulk import** — Import all subdirectories of a directory at once. The scan is recursive,
  so nested structures (artist/album/song) are handled.
- **Edit** — Modify track assignments, upload audio and MIDI files, edit lighting shows
  visually or as raw DSL.
- **Delete** — Remove a song from the registry by deleting its `song.yaml`. Audio and other
  files are preserved. The song is automatically removed from any playlists that reference it.

When uploading a file that already exists in the song directory, you'll be prompted to confirm
the replacement.
