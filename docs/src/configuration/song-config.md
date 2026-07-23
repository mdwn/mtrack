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

# (Optional) The song's tempo map. When present, this is the canonical source
# of the beat grid — it takes precedence over click track analysis. BPM is in
# quarter notes per minute.
tempo:
  bpm: 118
  time_signature: 4/4
  # Seconds of lead-in before measure 1 beat 1 (optional).
  start: 0.35
  # Tempo and/or time signature changes at measure positions (optional).
  changes:
    - measure: 33
      bpm: 126
      # Ramp gradually over 2 measures; omit `transition` for an instant change.
      transition: { measures: 2 }
    - measure: 65
      bpm: 96
      time_signature: 6/8

# (Optional) A generated metronome click track, derived from the beat grid
# (tempo map or click analysis). It appears as a virtual output track —
# route it via track_mappings in the profile like any other track.
metronome:
  track: metronome          # output track name (default "metronome")
  accent: [3, 2, 2]         # optional accent grouping within a measure
  sounds:                   # optional; synthesized clicks by default
    accent: { freq: 1600, volume: 1.0 }
    normal: { file: clicks/lo.wav }

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

## Tempo Map

The optional `tempo:` block describes the song's tempo and meter explicitly. When present it
is the canonical source of the song's beat grid, used for section resolution, the beat/measure
display, and (in the future) metronome generation — taking precedence over click track
analysis.

```yaml
tempo:
  bpm: 152            # quarter notes per minute
  time_signature: 7/8 # odd meters welcome; the grid gets one beat per eighth
  start: 0.2          # optional lead-in seconds before measure 1 beat 1
  changes:            # optional tempo/time-signature changes
    - measure: 33
      bpm: 126
      transition: { measures: 2 }  # linear ramp; or { beats: 4 }; omit to snap
    - measure: 65
      time_signature: 6/8
```

- `bpm` — Initial tempo in quarter-note beats per minute (required)
- `time_signature` — Initial meter as `numerator/denominator` (default `4/4`)
- `start` — Offset in seconds of the first downbeat within the audio (default 0)
- `changes` — List of changes in ascending measure order. Each entry names a `measure`
  (1-indexed, with optional fractional `beat` in quarter-note units) and provides a new
  `bpm` and/or `time_signature`. An optional `transition` ramps the tempo linearly over
  `{ beats: N }` or `{ measures: N }`; without it the change is instant.

The generated beat grid emits one beat per denominator note — a 7/8 song gets seven beats per
measure, matching how a metronome would click it.

A tempo map can be added in the web UI's Sections tab, including a one-click "detect" that
prefills it from the song's MIDI file or an analyzed click track.

Songs with a DSL light show but no `tempo {}` block in the `.light` file automatically use
the song's tempo map for measure-based cues and beat-based effect parameters.

## Metronome

The optional `metronome:` block generates a click track from the song's beat grid — no click
audio file needed. It replaces external metronome tools: the clicks always stay locked to the
song (including seeks and section loops), follow tempo changes and odd meters from the tempo
map, and route anywhere.

```yaml
metronome:
  track: metronome    # the output track name (default "metronome")
  accent: [3, 2, 2]   # optional accent grouping (7/8: accents on beats 1, 4, 6)
  sounds:
    accent: { freq: 1600, volume: 1.0 }   # synthesized click, or:
    normal: { file: clicks/lo.wav }       # a sample file (relative to the song dir)
```

- The metronome is a **virtual track**: add its track name to the profile's `track_mappings`
  to route it (e.g. to your in-ear mix), and adjust its level in the track gains mixer.
  Without a mapping it is silent and costs nothing.
- One click per denominator note: a 7/8 song clicks seven eighths per measure. Accents fall
  on beat 1, or on each group start when `accent` is set.
- Sounds default to short synthesized sine clicks (accent 1600 Hz, normal 1200 Hz); each can
  be overridden with a sample file.
- Player-wide default sounds can be set once in `mtrack.yaml` (see the player configuration);
  a song then just needs `metronome: {}` to enable the click with your preferred sound.
  Song-level sound fields override the defaults per field.
- The `metronome` track name must not collide with a real track. A beat grid (tempo map or
  analyzed click track) is required.

## Sections

Sections define named regions of a song by measure boundaries. They enable section looping
during playback — activating a section loop causes playback to repeat that section until
stopped.

Sections require a beat grid — from an explicit `tempo:` block or from click track
analysis — so that measure boundaries can be resolved to audio positions.

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

A song's beat grid comes from the explicit `tempo:` block when one is configured. Otherwise,
when a song has a track named "click", mtrack analyzes it offline to detect beat positions and
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
