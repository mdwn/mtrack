# Player Configuration (YAML)

> **Note:** This page documents the YAML configuration format. For most users, the web UI's
> Config page is the easiest way to configure mtrack.
> See [Hardware Configuration](../getting-started/hardware-config.md).

The player configuration file (`mtrack.yaml`) controls all of mtrack's runtime settings. It is
created with defaults when mtrack starts without one, and can be edited through the web UI or by
hand. The directory holding it is the *project directory* (see
[Project directory](#project-directory) at the end of this page).

`mtrack.yaml` holds the project-wide settings: where the songs and playlists are, samples, and
metronome defaults. Everything that describes a host's hardware — the audio and MIDI devices,
DMX universes, trigger inputs, controllers, track mappings, status events — lives in a
**hardware profile**. Profiles are either files in a `profiles_dir` directory or an inline
`profiles:` list; see [Hardware Profiles](hardware-profiles.md) for how a profile is chosen for
the host.

You start mtrack with `mtrack start` from the project directory, or `mtrack start
/path/to/project` (a directory containing `mtrack.yaml`, or the path of the file itself).

## mtrack.yaml

A fully annotated file (`examples/mtrack.yaml` in the repository is a shorter one of the same
shape):

```yaml
# The directory where all of your songs are located, frequently referred to as the song
# repository. If the path is not absolute, it is relative to the location of this file.
songs: songs

# Directory of playlist files (relative to this file). Each `<name>.yaml` in it is a
# playlist named after the file. Defaults to `playlists/` next to this file.
playlists_dir: playlists/

# The playlist to select at startup, by name. Written back by mtrack when you switch
# playlists from the web UI, gRPC, or OSC. Defaults to "playlist".
active_playlist: playlist

# Hardware profiles, one file per host, sorted by filename. See Hardware Profiles.
profiles_dir: profiles/

# (Optional) Player-wide metronome defaults. See the Metronome section of Song Configuration.
metronome:
  # When true, every song with a `tempo:` block gets a metronome unless it sets
  # `metronome: { enabled: false }`. Defaults to false.
  enabled: false
  # Master click level (0.0-2.0) for songs that set none. Defaults to 1.0.
  volume: 0.8
  # Default click sounds; a song's own `metronome.sounds` overrides these per field.
  sounds:
    accent: { freq: 1600, volume: 1.0 }
    normal: { freq: 1200, volume: 0.8 }

# (Optional) Sample definitions, available in every song. See Samples.
samples:
  kick:
    file: samples/kick.wav
    output_channels: [3, 4]

# (Optional) A separate file holding `samples:` (and `max_sample_voices:`) in the same
# format. Inline definitions above override entries of the same name from this file.
# samples_file: samples.yaml

# (Optional) Maximum number of concurrent sample voices across all samples. Defaults to 32.
max_sample_voices: 32
```

### Top-level keys

| Key | Type | Default | Required |
|-----|------|---------|----------|
| `songs` | path | — | yes (a file created by mtrack uses `.`) |
| `playlists_dir` | path | `playlists/` next to `mtrack.yaml` | no |
| `active_playlist` | string | `playlist` | no |
| `playlist` | path | — | no; a single playlist file, loaded under the name `playlist` when `playlists_dir` holds no `playlist.yaml` |
| `profiles_dir` | path | — | no; see [Hardware Profiles](hardware-profiles.md) |
| `profiles` | list of profiles | — | no; ignored when `profiles_dir` holds any profile |
| `metronome.enabled` | bool | `false` | no |
| `metronome.volume` | float 0.0–2.0 | `1.0` | no |
| `metronome.sounds` | `accent`/`normal`/`half`/`sub` click sounds | synthesized | no |
| `samples` | map of name → sample definition | `{}` | no; see [Samples](samples.md) |
| `samples_file` | path | — | no |
| `max_sample_voices` | integer | `32` | no |

Relative paths are resolved against the directory holding `mtrack.yaml`.

Any of the profile keys below (`audio`, `midi`, `dmx`, `trigger`, `controllers`,
`track_mappings`, `status_events`, `sample_triggers`) can also appear at the top level; that is
the [legacy layout](#legacy-layout).

## A profile file

Each file in `profiles_dir` is one hardware profile. The following file documents every
profile key; `examples/profiles/` in the repository has shorter ones for several host roles.

```yaml
# Identifies this file as a hardware profile.
kind: hardware_profile

# (Optional) Only use this profile on the host with this hostname. A profile without
# a hostname matches any host. Profiles are tried in filename order; the first match wins.
hostname: raspberry-pi-a

# The audio configuration. Omit the whole section on a host without audio output.
audio:
  # This audio device will be matched as best as possible against the devices on your system.
  # Run `mtrack devices` to see a list of the devices that mtrack recognizes.
  device: UltraLite-mk5

  # (Optional) The audio output buffer size in samples. This affects both playback stability
  # and sample trigger latency (see Trigger Configuration, Latency). Smaller values reduce
  # latency but require more CPU. Defaults to 1024 samples. Common values: 128, 256, 512, 1024.
  buffer_size: 256

  # (Optional) The stream (period) size requested from the audio backend: "default" (let the
  # backend choose), "min" (the smallest the device supports), or a number of frames. When
  # unset, buffer_size is used.
  # stream_buffer_size: min

  # (Optional) Worker threads that decode song files ahead of playback. Defaults to 2.
  # buffer_threads: 2

  # (Optional) The sample rate to use for the audio device. Defaults to 44100.
  sample_rate: 44100

  # (Optional) The sample format to use for the audio device: int or float. Defaults to int.
  sample_format: int

  # (Optional) The bits per sample (bit depth) to use for the audio device. Defaults to 32.
  bits_per_sample: 32

  # (Optional) Resampling algorithm used when a file's sample rate differs from the device's.
  # "sinc" (default): High-quality sinc interpolation, higher CPU usage.
  # "fft": FFT-based resampling, considerably faster on low-power hardware (e.g. Raspberry Pi).
  # resampler: fft

  # (Optional) Once a song is started, mtrack will wait this amount before triggering the audio
  # playback. Defaults to 0.
  playback_delay: 500ms

  # Mappings of track names to output channels (1-indexed). Track names are the `name` of
  # each track in song.yaml, plus the virtual tracks (metronome, pilot, mtrack:looping).
  track_mappings:
    click: [1]
    cue: [2]
    backing-track-l: [3]
    backing-track-r: [4]
    keys: [5, 6]

  # (Optional) Per-track gain in dB, -60 to +12. Tracks without an entry play at unity.
  track_gains:
    click: -6.0

# The MIDI configuration. Omit the section on a host without MIDI.
midi:
  # This MIDI device will be matched as best as possible against the devices on your system.
  # Run `mtrack midi-devices` to see a list of the devices that mtrack recognizes.
  device: UltraLite-mk5

  # (Optional) Once a song is started, mtrack will wait this amount before triggering the MIDI
  # playback. Defaults to 0.
  playback_delay: 500ms

  # (Optional) Enable MIDI beat clock output (24 ppqn). When enabled, mtrack sends MIDI System
  # Real-Time messages (Start, Timing Clock, Stop) to synchronize external gear to the song's
  # tempo. The clock follows the tempo events in the song's MIDI playback file; a song whose
  # MIDI file has no tempo events sends no beat clock, and the song-level `tempo:` block in
  # song.yaml does not drive it.
  #
  # The beat clock thread runs at elevated (real-time) thread priority to minimize timing jitter.
  # On Linux, this requires CAP_SYS_NICE (granted by the systemd service unit). On macOS, no
  # special privileges are needed. If real-time scheduling cannot be obtained, the beat clock
  # still functions but may exhibit more jitter under heavy system load. You can tune the thread
  # priority with the MTRACK_THREAD_PRIORITY environment variable (0-99, default 70), or disable
  # real-time scheduling entirely with MTRACK_DISABLE_RT_AUDIO=1.
  beat_clock: true

  # (Optional) Keep the beat clock free-running at the last known tempo once a song stops,
  # until the next song starts. Only meaningful when beat_clock is enabled. When a song ends
  # (or is stopped), mtrack sends Stop and then keeps emitting Timing Clock messages at that
  # song's final tempo, so downstream gear (tempo-synced delays, LFOs, arpeggiators, tempo
  # displays) holds the tempo instead of drifting or resetting during the gap between songs.
  # When the next song begins, its own tempo takes over. Songs without a tempo map keep the
  # previously established tempo alive. Defaults to false: the clock goes silent once a song
  # stops.
  persist_tempo: true

  # (Optional) You can route live MIDI events into the DMX engine with this configuration.
  midi_to_dmx:

  # Watch for each MIDI event in channel 15.
  - midi_channel: 15
    # Route these events to the light-show universe.
    universe: light-show

    # Transform the MIDI events into multiple
    transformers:
    # Maps the input note into the given list of notes. The velocity will be copied to each
    # new note.
    - type: note_mapper
      input_note: 0
      convert_to_notes: [0, 1, 2, 4, 5, 6]

    # Maps the input controller into the given list of controllers. The controller value will
    # be mapped to each new controller.
    - type: control_change_mapper
      input_controller: 0
      convert_to_controllers: [0, 1, 2, 4, 5, 6]

# The DMX configuration. This maps OLA universes to light show names defined within
# song files. Omit the section on a host without lighting.
dmx:
  # The DMX engine in mtrack has a dimming engine that can be issued using MIDI program change (PC) commands.
  # This modifier is multiplied by the value of the PC command to give a dimming duration, e.g.
  # PC1 * 1.0 dim speed modifier = 1.0 second dim time
  # PC1 * 0.25 dim speed modifier = 0.25 second dim time
  # PC5 * 0.25 dim speed modifier = 1.25 second dim time
  # Defaults to 1.0.
  dim_speed_modifier: 0.25

  # (Optional) Once a song is started, mtrack will wait this amount before triggering the DMX
  # playback. This applies to both DMX paths — MIDI-based DMX and DSL `.light` shows.
  #
  # Audio leaves late (decode, the output stream buffer, device latency) while DMX frames go out
  # with only frame time and fixture response in the way, so lights driven straight off the
  # transport clock fire early relative to what the audience hears. This is the knob that pulls
  # them back into line.
  playback_delay: 500ms

  # (Optional) The port olad streams DMX on. Defaults to 9010.
  ola_port: 9010

  # (Optional) The port of olad's web server, which is not olad's streaming port. mtrack asks it,
  # at startup and on every reload, whether each universe below has an output port patched to it,
  # and warns when one does not — olad silently drops frames for a universe nothing is patched to.
  # Defaults to 9090. Nothing on the output path depends on this; if olad's web server is off, the
  # check is skipped.
  ola_http_port: 9090

  # (Optional) When true, mtrack does not connect to olad at all: the lighting engine runs
  # against a no-op client and streams nowhere. Useful for the TUI and for authoring without
  # a rig. Defaults to false.
  null_client: false

  # Universes here map OLA universe numbers into light show names.
  universes:
  # Any songs with a light show with a universe_name "light-show" will be played on OLA universe 1.
  - universe: 1
    name: light-show

  # (Optional) The lighting system: current venue, logical groups, and the directories that
  # hold fixture types and venues. See the Lighting chapter.
  lighting:
    current_venue: main_stage
    directories:
      fixture_types: lighting/fixture_types
      venues: lighting/venues
    groups:
      front_wash:
        name: front_wash
        constraints:
          - AllOf: ["wash", "front"]

# (Optional) Sample trigger inputs: audio (piezo) and MIDI. See Trigger Configuration.
trigger:
  inputs:
    - kind: midi
      event:
        type: note_on
        channel: 10
        key: 60
      sample: kick

# (Optional) MIDI events emitted on a cycle to report the player's state; see Hardware
# Profiles, Status Events.
status_events:
  off_events:
    - { type: control_change, channel: 16, controller: 3, value: 2 }
  idling_events:
    - { type: control_change, channel: 16, controller: 2, value: 2 }
  playing_events:
    - { type: control_change, channel: 16, controller: 2, value: 2 }

# (Optional) Notification sounds for section looping; see Hardware Profiles, Notification Audio.
notifications:
  loop_armed: notifications/loop-armed.wav

# The controller definitions. The valid kinds of controllers are:
# - grpc
# - midi
# - osc
# - mcp
controllers:
# The gRPC server configuration.
- kind: grpc

  # The port the gRPC server should be hosted on. Defaults to 43234.
  port: 43234

# The OSC server configuration.
- kind: osc

  # The port the OSC server should be hosted on. Defaults to 43235.
  port: 43235

  # The addresses that player status should be broadcast to.
  broadcast_addresses:
  - 127.0.0.1:43236

  # Maps player events to arbitrary OSC events. If not specified, these
  # below are the defaults. None of these events require any arguments.
  play: /mtrack/play
  # Pause preserves the current position; the next play resumes from it.
  pause: /mtrack/pause
  prev: /mtrack/prev
  next: /mtrack/next
  stop: /mtrack/stop
  all_songs: /mtrack/all_songs
  playlist: /mtrack/playlist
  # Stops every triggered sample.
  stop_samples: /mtrack/samples/stop

  # The following events will be used by mtrack to report the current
  # player status over OSC. If not specified, these below are the defaults.

  # The current status of the player: whether it's stopped or playing, and the
  # current elapsed time and the song duration. Contains a single string argument.
  status: /mtrack/status

  # The audio output's health verdict, separate from the player status: "healthy",
  # "recovering", "stalled" or "never_started". The player can be playing while the
  # output is stalled, and that combination is the one worth a red light.
  audio_health: /mtrack/audio/health

  # The playlist that is currently being played. Contains a single string argument,
  # though it will be fairly long depending on your playlist.
  playlist_current: /mtrack/playlist/current

  # The song that the playlist is currently pointing to. Contains a single string
  # argument.
  playlist_current_song: /mtrack/playlist/current_song

  # The duration of the time elapsed since a song was playing and the
  # total duration of the song. Contains a single string argument.
  playlist_current_song_elapsed: /mtrack/playlist/current_song/elapsed

  # Numeric playback progress followed by zero or more section triples:
  # elapsed seconds, duration seconds, then repeating name/start/end values.
  # Designed for dynamic TouchOSC and other timeline displays.
  timeline: /mtrack/timeline

  # Section loop control paths. `loop_section` takes a section name string argument.
  section_ack: /mtrack/section_ack
  stop_section_loop: /mtrack/stop_section_loop
  loop_section: /mtrack/loop_section

  # Seek paths. `seek` takes a numeric seconds argument; `seek_section`
  # takes a section name string argument.
  seek: /mtrack/seek
  seek_section: /mtrack/seek_section

  # Per-track gain in dB (float argument). The `*` segment is the track name; the same
  # pattern, with the name filled in, is used for gain feedback broadcasts.
  track_gain: /mtrack/track/*/gain


# The MIDI controller configuration.
- kind: midi

  # When mtrack recognizes this MIDI event, it will play the current song if no other song is
  # currently playing.
  play:
    type: control_change
    channel: 16
    controller: 100
    value: 0

  # When mtrack recognizes this MIDI event, it will navigate to the previous song in the playlist
  # if no other song is currently playing.
  prev:
    type: control_change
    channel: 16
    controller: 100
    value: 1

  # When mtrack recognizes this MIDI event, it will navigate to the next song in the playlist
  # if no other song is currently playing.
  next:
    type: control_change
    channel: 16
    controller: 100
    value: 2

  # When mtrack recognizes this MIDI event, it will stop the currently playing song.
  stop:
    type: control_change
    channel: 16
    controller: 100
    value: 3

  # When mtrack recognizes this MIDI event, it will switch to the playlist of all known songs in
  # your song repository.
  all_songs:
    type: control_change
    channel: 16
    controller: 100
    value: 4

  # When mtrack recognizes this MIDI event, it will switch to the defined playlist.
  playlist:
    type: control_change
    channel: 16
    controller: 100
    value: 5

  # (Optional) Acknowledge the current section, arming a section loop.
  section_ack:
    type: control_change
    channel: 16
    controller: 100
    value: 6

  # (Optional) Break out of the current section loop.
  stop_section_loop:
    type: control_change
    channel: 16
    controller: 100
    value: 7

  # Optional: Morningstar controller integration. When configured, mtrack will
  # automatically update the current bank name on the controller via SysEx
  # whenever the current song changes. This eliminates the need for per-song
  # program change mappings.
  morningstar:
    # The Morningstar controller model. Determines the device ID byte and the
    # required name length in the SysEx message.
    # Supported values: mc3, mc6, mc8, mc6pro, mc8pro, mc4pro
    # For unlisted models, use: { custom: { model_id: <0-127> } }
    model: mc4pro

    # Whether to save the bank name to flash (true) or keep it temporary (false).
    # Temporary names reset on power cycle. Default: false.
    # save: false

# The MCP (Model Context Protocol) server configuration. Exposes mtrack to
# MCP-compatible clients (Claude Desktop, Claude Code, ...) over HTTP at /mcp.
# See the MCP Control interface documentation for details.
- kind: mcp

  # The port the MCP server should be hosted on. Defaults to 43237.
  port: 43237

  # The bind address. Defaults to 127.0.0.1 (localhost-only). Set to 0.0.0.0
  # to expose the server on the network.
  bind_address: 127.0.0.1

  # Optional bearer token. When set, every request must carry an
  # `Authorization: Bearer <token>` header. Recommended whenever bind_address
  # is not localhost.
  # bearer_token: "your-secret-token"

  # Idle session timeout in seconds. Defaults to 14400 (4 hours). Set to null
  # to disable idle eviction.
  # idle_session_timeout_secs: 14400
```

### Profile keys

| Key | Type | Default | Required |
|-----|------|---------|----------|
| `kind` | `hardware_profile` | `hardware_profile` | no |
| `hostname` | string | — (matches any host) | no |
| `audio.device` | string | — | yes, when `audio` is present |
| `audio.buffer_size` | integer (samples) | `1024` | no |
| `audio.stream_buffer_size` | `default`, `min`, or frames | `buffer_size` | no |
| `audio.buffer_threads` | integer ≥ 1 | `2` | no |
| `audio.sample_rate` | integer (Hz) | `44100` | no |
| `audio.sample_format` | `int` or `float` | `int` | no |
| `audio.bits_per_sample` | integer | `32` | no |
| `audio.resampler` | `sinc` or `fft` | `sinc` | no |
| `audio.playback_delay` | duration | `0` | no |
| `audio.track_mappings` | map of track name → channel list | `{}` | playback needs at least one |
| `audio.track_gains` | map of track name → dB | `{}` | no |
| `midi.device` | string | — | yes, when `midi` is present |
| `midi.playback_delay` | duration | `0` | no |
| `midi.beat_clock` | bool | `false` | no |
| `midi.persist_tempo` | bool | `false` | no |
| `midi.midi_to_dmx` | list | `[]` | no |
| `dmx.dim_speed_modifier` | float > 0 | `1.0` | no |
| `dmx.playback_delay` | duration | `0` | no |
| `dmx.ola_port` | integer | `9010` | no |
| `dmx.ola_http_port` | integer | `9090` | no |
| `dmx.null_client` | bool | `false` | no |
| `dmx.universes` | list of `{ universe, name }` | `[]` | no |
| `dmx.lighting` | lighting system | — | no; see [Lighting Configuration](../lighting/configuration.md) |
| `trigger` | trigger inputs | — | no; see [Trigger Configuration](triggers.md) |
| `controllers` | list | `[]` | no |
| `status_events` | `off_events`/`idling_events`/`playing_events` | — | no |
| `notifications` | notification sounds | — | no |

Durations use the `duration_string` format: `500ms`, `1s`, `2m`.

## Legacy layout

`audio`, `midi`, `dmx`, `trigger`, `controllers`, `track_mappings`, `status_events` and
`sample_triggers` are also accepted at the top level of `mtrack.yaml`, with the same content
as inside a profile (`track_mappings` as its own top-level key rather than under `audio`).
A file written this way is loaded as a single profile with no hostname. `sample_triggers`
entries become `kind: midi` trigger inputs in that profile.

When `profiles:` is present or `profiles_dir` holds any profile, each of those top-level keys
is ignored with a warning in the log, except `status_events`, which is used only when the
matched profile has none. `mtrack migrate` rewrites a legacy file into the profile layout.

## Project directory

The project directory is the one holding `mtrack.yaml` — wherever you put it, be that
`/var/lib/mtrack`, `/home/pi/gig`, or a folder on a USB stick. mtrack creates a configured
directory — `songs`, `profiles_dir`, the lighting directories, `playlists_dir` — only when it
resolves to somewhere inside the project directory, and otherwise refuses with an error naming
both paths. A directory that already exists is used wherever it lives, so `songs: /mnt/song-storage`
works once that directory is there: create it yourself (`mkdir -p /mnt/song-storage`).

The refusal exists because a typo would otherwise become an empty directory and a puzzling
"no songs found", and because on removable media, creating a directory under a mount point that
is not currently mounted writes to the underlying disk. It covers only a configured directory
that is **missing** and outside the project. An existing but unmounted mount point
(`songs: /mnt/songs` where `/mnt/songs` is an empty directory awaiting a mount) is accepted and
written to, and a mount point *inside* the project (`songs: /var/lib/mtrack/usb`) is mtrack's to
create. If you rely on a mount, mount it before mtrack starts; the generated systemd unit retries
for about two and a half minutes, which covers a drive that appears a little after boot (see
[Running on Startup](../deployment/systemd.md)).

### Web UI and file management

The web UI's management features (song editing, file uploads, lighting authoring, playlist
management) expect all project files to reside under the project directory. While `mtrack.yaml`
supports absolute paths and references to files on other mounts, the web UI can only manage
files that are within the project root directory.

For best results:
- Keep your `songs` path relative (e.g. `songs: .` or `songs: songs`)
- Store playlists in a `playlists/` subdirectory within the project root
- Store light shows alongside song files in the song directories, and fixtures and venues
  under `lighting/` (see
  [Where the lighting files live](../lighting/configuration.md#where-the-lighting-files-live))
- Ensure mtrack has **write access** to the project root and its contents
