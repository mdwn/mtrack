# Song Configuration (YAML)

> **Note:** This page documents the YAML format for song definitions. For most users, the web
> UI is the easiest way to create and manage songs.
> See [Importing Songs](../getting-started/importing-songs.md).

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

## song.yaml Format

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

# (Optional) Loop the song indefinitely. Audio crossfades seamlessly at loop
# boundaries. Press Play or Next to break out and advance the playlist.
loop_playback: true

# (Optional) Named sections defined by measure boundaries. Used for section
# looping during playback. Measure numbers are 1-indexed; end_measure is exclusive.
sections:
  - name: verse
    start_measure: 1
    end_measure: 17
  - name: chorus
    start_measure: 17
    end_measure: 25
  - name: bridge
    start_measure: 33
    end_measure: 41

# (Optional) Per-song notification audio overrides. These override the
# profile-level notification sounds for this song only.
notification_audio:
  loop_armed: notifications/loop-armed.wav
  break_requested: notifications/break.wav
  loop_exited: notifications/exit.wav
  section_entering: notifications/section.wav
  # Per-section-name overrides:
  sections:
    chorus: notifications/chorus-entering.wav
    bridge: notifications/bridge-entering.wav
```

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

## Song Looping

Setting `loop_playback: true` causes the song to loop indefinitely when it reaches the end:

- **Audio** crossfades seamlessly at loop boundaries (100ms linear fade)
- **MIDI** restarts from the beginning
- **Lighting/DMX** timelines reset cleanly

During a looping song, pressing Play or Next breaks out of the loop, advances the playlist,
and auto-plays the next song. Stop cancels everything as usual.

## Sections

Sections define named regions of a song by measure boundaries. They enable section looping
during playback — activating a section loop causes playback to repeat that section until
stopped.

Sections require a beat grid (from a click track) or a tempo map (from MIDI) so that measure
boundaries can be resolved to audio positions.

```yaml
sections:
  - name: intro
    start_measure: 1
    end_measure: 5
  - name: verse
    start_measure: 5
    end_measure: 21
```

- `name` — Display name for the section (must not be empty)
- `start_measure` — Start measure, 1-indexed, inclusive
- `end_measure` — End measure, 1-indexed, exclusive (must be greater than `start_measure`)

Sections can also be created visually in the web UI's Sections tab with a canvas-based
timeline editor that supports drag-to-create, resize, move, rename, and delete.

## Beat Grid and Song Analysis Cache

When a song has a track named "click", mtrack analyzes it offline to detect beat positions and
measure boundaries. The result is a `BeatGrid` with absolute beat times and accented-beat
indices. Beat grid data is exposed via gRPC and displayed in the web UI (measure/beat position
during playback, beat/measure counts in song detail).

Computed song data (waveform peaks, beat grids) is persisted to `.mtrack-cache.json` in each
song's directory. The cache uses file mtime+size for invalidation — if an audio file changes,
its cached data is recomputed on next access.

## CLI: Listing Songs

```
$ mtrack songs /mnt/song-storage
Songs (count: 23):
- Name: The first really cool song
  Duration: 5:10
  Channels: 11
  ...
```

## CLI: Generating Default Song Configurations

Song configurations can be generated using the `songs` command:

```
$ mtrack songs --init /mnt/song-storage
```

This creates a `song.yaml` in each subfolder of `/mnt/song-storage`. The name of the
subfolder determines the song's name. Audio files are used as tracks (stereo and multichannel
files are split into per-channel tracks). MIDI files are used as MIDI playback, and files
prefixed with `dmx_` are treated as MIDI DMX light shows. `.light` files are auto-detected as
DSL lighting shows.
