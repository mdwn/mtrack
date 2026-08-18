# Player Configuration (YAML)

> **Note:** This page documents the YAML configuration format. For most users, the web UI's
> Config page is the easiest way to configure mtrack.
> See [Hardware Configuration](../getting-started/hardware-config.md).

The player configuration file (`mtrack.yaml`) controls all of mtrack's runtime settings. It is
created automatically when mtrack starts, and can be edited through the web UI or by hand.

**Directories outside the project are yours to create.** The *project directory* is simply the one
holding this `mtrack.yaml` — wherever you put it, be that `/var/lib/mtrack`, `/home/pi/gig`, or a
folder on a USB stick. mtrack makes a configured directory — `songs`, `profiles_dir`, the lighting
directories, `playlists_dir` — only when it resolves to somewhere inside that directory, and
otherwise refuses with an error naming both paths.

A directory that already exists is used wherever it lives, so pointing `songs` at
`/mnt/song-storage` works exactly as before once that directory is there. Only creating one
elsewhere is refused, for two reasons:

- A typo would otherwise become an empty directory and a puzzling "no songs found" rather than an
  error naming the path you actually typed.
- On removable media, creating a directory under a mount point that is not currently mounted writes
  to the underlying disk instead. Everything appears to work until the drive is mounted, at which
  point the files vanish behind it. This is easy to hit on a Pi with songs on a USB stick or SD
  card. Refusing to create the directory avoids it for any mount point outside the project — note
  that a mount point *inside* the project, such as `songs: /var/lib/mtrack/usb`, is still mtrack's
  to create and so is still exposed to this. Mount before starting mtrack if you rely on one.

If you keep songs on a separate drive, create the directory once (`mkdir -p /media/usb/songs`) and
mtrack will use it from then on.

```yaml
# The directory where all of your songs are located, frequently referred to as the song repository.
# If the path is not absolute, it will be relative to the location of this file.
# mtrack creates this directory for you only when it is inside the project directory (the one
# holding this file). An absolute path elsewhere, like the one below, is yours to create.
songs: /mnt/song-storage

# The path to the playlist file.
playlist: /mnt/playlist.yaml

# (Optional) Player-wide metronome defaults. Songs enable the metronome with
# a `metronome:` block in song.yaml; the volume and any sounds not overridden
# there fall back to these, so `metronome: {}` is enough per song once this is
# configured.
metronome:
  volume: 0.8 # master click level (0.0-2.0) for songs that set none
  sounds:
    accent: { freq: 1600, volume: 1.0 }
    normal: { freq: 1200, volume: 0.8 }

# The audio configuration for mtrack.
audio:
  # This audio device will be matched as best as possible against the devices on your system.
  # Run `mtrack devices` to see a list of the devices that mtrack recognizes.
  device: UltraLite-mk5

  # (Optional) The audio output buffer size in samples. This affects both playback stability
  # and MIDI-triggered sample latency. Smaller values reduce latency but require more CPU.
  # Trigger latency is approximately 2x this value (e.g., 256 samples = ~11.6ms at 44.1kHz).
  # Defaults to 1024 samples. Common values: 128, 256, 512, 1024.
  buffer_size: 256

  # (Optional) The sample rate to use for the audio device. Defaults to 44100.
  sample_rate: 44100

  # (Optional) The sample format to use for the audio device. Defaults to int.
  sample_format: int

  # (Optional) The bits per sample (bit depth) to use for the audio device. Defaults to 32.
  bits_per_sample: 32

  # (Optional) Resampling algorithm used when source and output sample rates differ.
  # "sinc" (default): High-quality sinc interpolation, higher CPU usage.
  # "fft": FFT-based resampling, considerably faster on low-power hardware (e.g. Raspberry Pi).
  # resampler: fft

  # (Optional) Once a song is started, mtrack will wait this amount before triggering the audio playback.
  playback_delay: 500ms

# The MIDI configuration for mtrack.
midi:
  # This MIDI device will be matched as best as possible against the devices on your system.
  # Run `mtrack midi-devices` to see a list of the devices that mtrack recognizes.
  device: UltraLite-mk5

  # (Optional) Once a song is started, mtrack will wait this amount before triggering the MIDI playback.
  playback_delay: 500ms

  # (Optional) Enable MIDI beat clock output (24 ppqn). When enabled, mtrack sends MIDI System
  # Real-Time messages (Start, Timing Clock, Stop) to synchronize external gear to the song's
  # tempo. Beat clock is only sent for songs whose MIDI files contain explicit tempo change events;
  # songs without a tempo map do not emit beat clock, leaving musicians free to control their own
  # tempo.
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
  # previously established tempo alive. Defaults to false, which restores the original
  # behavior where the clock goes silent once a song stops.
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

# The DMX configuration for mtrack. This maps OLA universes to light show names defined within
# song files.
dmx:
  # The DMX engine in mtrack has a dimming engine that can be issued using MIDI program change (PC) commands.
  # This modifier is multiplied by the value of the PC command to give a dimming duration, e.g.
  # PC1 * 1.0 dim speed modifier = 1.0 second dim time
  # PC1 * 0.25 dim speed modifier = 0.25 second dim time
  # PC5 * 0.25 dim speed modifier = 1.25 second dim time
  dim_speed_modifier: 0.25

  # (Optional) Once a song is started, mtrack will wait this amount before triggering the DMX
  # playback. This applies to both DMX paths — MIDI-based DMX and DSL `.light` shows.
  #
  # Audio leaves late (decode, the output stream buffer, device latency) while DMX frames go out
  # with only frame time and fixture response in the way, so lights driven straight off the
  # transport clock fire early relative to what the audience hears. This is the knob that pulls
  # them back into line.
  playback_delay: 500ms

  # Universes here map OLA universe numbers into light show names.
  universes:
  # Any songs with a light show with a universe_name "light-show" will be played on OLA universe 1.
  - universe: 1
    name: light-show

# Status events are now configured per-profile in the hardware profile configuration.
# See the Hardware Profiles documentation for details.
# Legacy top-level status_events are automatically normalized into the profile at startup.

# The controller definitions. As of now, the valid kinds of controllers are:
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
  prev: /mtrack/prev
  next: /mtrack/next
  stop: /mtrack/stop
  all_songs: /mtrack/all_songs
  playlist: /mtrack/playlist

  # The following events will be used by mtrack to report the current
  # player status over OSC. If not specified, these below are the defaults.

  # The current status of the player: whether it's stopped or playing, and the
  # current elapsed time and the song duration. Contains a single string argument.
  status: /mtrack/status

  # The playlist that is currently being played. Contains a single string argument,
  # though it will be fairly long depending on your playlist.
  playlist_current: /mtrack/playlist/current

  # The song that the playlist is currently pointing to. Contains a single string
  # argument.
  playlist_current_song: /mtrack/playlist/current_song

  # The duration of the time elapsed since a song was playing and the
  # total duration of the song. Contains a single string argument.
  playlist_current_song_elapsed: /mtrack/playlist/current_song/elapsed

  # Section loop control paths.
  section_ack: /mtrack/section_ack
  stop_section_loop: /mtrack/stop_section_loop
  loop_section: /mtrack/loop_section

  # Seek paths. `seek` takes a numeric seconds argument; `seek_section`
  # takes a section name string argument.
  seek: /mtrack/seek
  seek_section: /mtrack/seek_section


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

# Mappings of track names to output channels.
track_mappings:
  click:
  - 1
  cue:
  - 2
  backing-track-l:
  - 3
  backing-track-r:
  - 4
  keys:
  - 5
  - 6
```

You can start `mtrack` as a process with `mtrack start /path/to/player.yaml`, or
simply `mtrack start` from the project directory.

## Web UI and File Management

The web UI's management features (song editing, file uploads, lighting authoring, playlist
management) expect all project files to reside under a single directory — the directory
containing `mtrack.yaml`. While `mtrack.yaml` supports absolute paths and references to files
on other mounts, the web UI can only manage files that are within the project root directory.

For best results:
- Keep your `songs` path relative (e.g. `songs: .` or `songs: songs`)
- Store playlists in a `playlists/` subdirectory within the project root
- Store lighting files alongside song files in the song directories
- Ensure mtrack has **write access** to the project root and its contents
