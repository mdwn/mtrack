# mtrack

[![Actions Status](https://github.com/mdwn/mtrack/actions/workflows/mtrack.yaml/badge.svg)](https://github.com/mdwn/mtrack/actions)
[![codecov](https://codecov.io/gh/mdwn/mtrack/graph/badge.svg?token=XWEK2BIPZL)](https://codecov.io/gh/mdwn/mtrack)
[![Crates.io Version](https://img.shields.io/crates/v/mtrack)](https://crates.io/crates/mtrack)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-2.1-4baaaa.svg)](code_of_conduct.md)

`mtrack` is a multitrack player intended for running on small devices like the Raspberry Pi. It can output
multiple tracks of audio as well as MIDI out via class compliant interfaces. The general intent here is to
allow `mtrack` to be controlled remotely from your feet as opposed to needing to drive a computer or tablet
on stage.

## Hands free multitrack playing

The idea behind `mtrack` is to provide a way to play multitracks in a live situation without using your hands.
In live situations, I frequently found myself babysitting a DAW while performing. The point of `mtrack` is to
avoid this situation by providing a very simple mechanism for playing back songs.

`mtrack` can read from multiple audio files and rearrange and combine the channels present in those files into
a singular audio stream that is routed to a class complaint audio interface. Additionally, `mtrack` can
simultaneously play back a MIDI file along with your audio, which allows for automation of on stage gear. `mtrack`
can also emit MIDI events on song selection, as well as listen for MIDI events in order to control the `mtrack`
player.

### The general behavior of mtrack

`mtrack` intends to have the following behavior loop:

1. `mtrack` starts on the first item in the user defined playlist. The item is selected, but not playing.
2. While no song is playing, the user can select a song on the playlist by using the `next` and `previous`
   events. `next` and `previous` are inactive when a song is playing.
3. The user can start a song using the `play` event and stop a currently playing song using the `stop` event.
   While a song is playing, `play` will perform no action, and while a song is not playing, `stop` will
   perform no action.
4. If a user needs to play a song not represented in their playlist, the user can use the `all_songs`
   event to move to a playlist that comprises a sorted list of all songs in a user's song repository. If the
   user would like to use their original playlist, the `playlist` event can be used.

The events listed above can be triggered using MIDI messages.

## Installation

`mtrack` can be installed through cargo:

```
$ cargo install mtrack --locked
```

If you want to use `mtrack` on startup, I recommend copying it to `/usr/local/bin`:

```
$ sudo cp ~/.cargo/bin/mtrack /usr/local/bin/mtrack
```

## Figuring out what devices are supported

You can figure out what audio devices `mtrack` recognizes by running `mtrack devices`:

```
$ mtrack devices
Devices:
- UltraLite-mk5 (Channels=22) (Alsa)
- bcm2835 Headphones (Channels=8) (Alsa)
```

The name prior to the parentheses is the identifier for use by `mtrack`. So when referring to the first
audio device, you would use the string `UltraLite-mk5`.

You can also figure out what MIDI devices are supported by running `mtrack midi-devices`:

```
$ mtrack midi-devices
Devices:
- Midi Through:Midi Through Port-0 14:0 (Input/Output)
- UltraLite-mk5:UltraLite-mk5 MIDI 1 28:0 (Input/Output)
```

The name prior to the first colon is the identifier for use by `mtrack`. When referring to the second
MIDI device, you would use the string `UltraLite-mk5`.

## File formats

### Configuration files

`mtrack` now uses [config-rs](https://github.com/rust-cli/config-rs) for configuration parsing, which
means we should support any of the configuration file formats that it supports. Testing for anything
other than YAML is limited at the moment.

### Audio files

`mtrack` supports a wide variety of audio formats through the [symphonia](https://github.com/pdeljanov/Symphonia) library. Supported formats include:

- **WAV** (PCM, various bit depths)
- **FLAC** (Free Lossless Audio Codec)
- **MP3** (MPEG Audio Layer III)
- **OGG Vorbis**
- **AAC** (Advanced Audio Coding)
- **ALAC** (Apple Lossless, in M4A containers)

All audio files are automatically transcoded to match your audio device's configuration (sample rate, bit depth, and format). Files can be mixed and matched within a song - for example, you can use a WAV file for your click track and an MP3 file for your backing track.

## Structure of an mtrack repository and supporting files

### Song repository

The song repository is a location on disk that houses both your backing tracks, MIDI files, and song
definitions. The song repository does not have to be in any particular layout, as `mtrack` will attempt
to parse any/all config files supported by `config-rs` it finds to look for song definitions.

### Songs

A song comprises of:

- One or more audio files.
- An optional MIDI file.
- One or more light shows (using `.light` DSL files, or legacy MIDI files interpreted as DMX).
- A song definition.

The audio files must all be the same bitrate. They do not need to be the same length. mtrack player will
play until the last audio (or MIDI) file is complete.

A song is defined in any of the formats supported by `config-rs`.

```yaml
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

# Legacy MIDI-based light shows (still supported for backward compatibility).
# light_shows:
# - universe_name: light-show
#   dmx_file: DMX Light Show.mid
#   midi_channels:
#   - 15

# An optional MIDI playback configuration.
midi_playback:
  file: Song Automation.mid

  # MIDI channels from the MIDI file to exclude. Useful if you want to do things like
  # exclude lighting data from MIDI playback.
  exclude_midi_channels:
  - 15


# The tracks associated with this song.
tracks:
# The click track only has one channel, so we can just indicate which output channel
# we want directly.
- name: click
  file: click.wav # File paths are relative to the song config file.
# Similarly, our cue only has one channel.
- name: cue
  file: /mnt/song-storage/cue.wav # Or file paths can be absolute.
# Our backing track file has two channels, so we have to specify `file_channel` to let
# mtrack know which channel from the file to use.
- name: backing-track-l
  file: Backing Tracks.wav
  file_channel: 1
# We can re-use our backing track file and specify the other channel if we'd like to do
# stereo.
- name: backing-track-r
  file: Backing Tracks.wav
  file_channel: 2
# Our keys file has two channels, but we're only interested in one.
# Note: You can use any supported audio format (WAV, MP3, FLAC, OGG, AAC, ALAC, etc.)
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
  Sample Rate: 44100
  Midi Message: Some(Midi { channel: u4(15), message: ProgramChange { program: u7(0) } })
  Midi File:None
  Tracks: click, cue, backing-track-l, backing-track-r, keys
- Name: The next really cool song
  ...
```

You can play individual songs by using `mtrack play-direct`:

```
$ mtrack play-direct -m my-midi-device -s 0.25 -d universe=1,name=light-show my-audio-device click=1,cue=2 /mnt/song-storage "My cool song"
2024-03-22T21:24:25.588828Z  INFO emit (midir): mtrack::midi::midir: Emitting event. device="my-midi-device:my-midi-device MIDI 1 28:0" event="Midi { channel: u4(15), message: ProgramChange { program: u7(3) } }"
2024-03-22T21:24:25.589420Z  INFO player: mtrack::player: Waiting for song to finish. song="My cool song"
2024-03-22T21:24:25.589992Z  INFO play song (rodio): mtrack::audio::rodio: Playing song. device="my-audio-device" song="My cool song" duration="4:14"
2024-03-22T21:24:25.591112Z  INFO play song (dmx): mtrack::dmx::engine: Playing song DMX. song="My cool song" duration="4:14"
2024-03-22T21:24:25.676452Z  INFO play song (midir): mtrack::midi::midir: Playing song MIDI. device="my-midi-device:my-midi-device MIDI 1 28:0" song="My cool song" duration="4:14"
```

#### Generating default song configurations

Song configurations can be generated using the `songs` command as follows:


```
$ mtrack songs --init /mnt/song-storage
```

This will create a file called `song.yaml` in each subfolder of `/mnt/storage`. The name of the
subfolder determines the song's name. Audio files (WAV, MP3, FLAC, OGG, AAC, ALAC, etc.) are used as tracks. The track's name is
determined using the file name and the number of channels within the file. MIDI files are used as
MIDI playback, MIDI files that start with `dmx_` will be used as light shows. You can edit the generated files to refine the settings to your needs. 

### Playlists

The playlist definition is a pretty simple config file:

```yaml
# This is a simple file that contains, in order, the names of all songs
# that mtrack should play. The names of the songs are defined in the
# song repository, which can be found in the mtrack config file.
songs:
- Sound check
- A really cool song
- Another cool song
- The slow one
- A really fast one
- Outro tape
```

This would play the given songs in that order, waiting for you to trigger the song.

### mtrack process definition

To start mtrack as a standalone player that's controllable by MIDI, you'll need to create a
player config file:

```yaml
# The directory where all of your songs are located, frequently referred to as the song repository.
# If the path is not absolute, it will be relative to the location of this file.
songs: /mnt/song-storage

# The path to the playlist file.
playlist: /mnt/playlist.yaml

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

  # (Optional) Once a song is started, mtrack will wait this amount before triggering the audio playback.
  playback_delay: 500ms

# The MIDI configuration for mtrack.
midi:
  # This MIDI device will be matched as best as possible against the devices on your system.
  # Run `mtrack midi-devices` to see a list of the devices that mtrack recognizes.
  device: UltraLite-mk5

  # (Optional) Once a song is started, mtrack will wait this amount before triggering the MIDI playback.
  playback_delay: 500ms

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

  # (Optional) Once a song is started, mtrack will wait this amount before triggering the DMX playback.
  playback_delay: 500ms

  # Universes here map OLA universe numbers into light show names.
  universes:
  # Any songs with a light show with a universe_name "light-show" will be played on OLA universe 1.
  - universe: 1
    name: light-show

# Status events are emitted to the controller while mtrack is running. This is largely done
# in order to confirm that mtrack is connected to the controller and operating properly.
# The statuses are emitted periodically in the following timeline:
# - Off (1 second)
# - On (250 milliseconds, either idling or playing)
# - Off (1 second)
# - On (250 milliseconds, either idling or playing)
# - ...
status_events:
  # Off events are emitted, in order, when trying to return the status indicator to "normal."
  # If your MIDI controller has LEDs, for example, this would be to turn the LED off.
  off_events:
  - type: control_change
    channel: 16
    controller: 3
    value: 2
  # Idling events are emitted, in order, when trying to indicate that the player is connected,
  # but not currently doing anything.
  idling_events:
  - type: control_change
    channel: 16
    controller: 2
    value: 2
  # Playing events are emitted, in order, when trying to indicate that the player is connected,
  # and actively playing.
  playing_events:
  - type: control_change
    channel: 16
    controller: 2
    value: 2

# The controller definitions. As of now, the valid kinds of controllers are:
# - grpc
# - midi
# - osc
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

You can start `mtrack` as a process with `mtrack start /path/to/player.yaml`.

### Hardware profiles

If you have multiple devices or run `mtrack` on multiple hosts sharing the same config file,
you can use hardware profiles instead of the flat `audio:` / `midi:` / `track_mappings:` sections.
Profiles are tried in list order; the first one whose device is available wins. Each profile can
optionally be restricted to a specific hostname.

```yaml
# Audio profiles tried in priority order. First match wins.
audio_profiles:
  # Raspberry Pi A: Behringer WING with FOH channel mapping
  - hostname: raspberry-pi-a
    device: "Behringer WING"
    sample_rate: 48000
    sample_format: int
    bits_per_sample: 32
    buffer_size: 1024
    playback_delay: 500ms
    track_mappings:
      click: [1]
      cue: [2]
      backing-track-l: [3]
      backing-track-r: [4]
      keys: [5, 6]

  # Raspberry Pi B: Same WING but different channel mapping (monitors)
  - hostname: raspberry-pi-b
    device: "Behringer WING"
    sample_rate: 48000
    track_mappings:
      click: [11]
      cue: [12]
      backing-track-l: [13]
      backing-track-r: [14]
      keys: [15, 16]

  # Fallback: built-in audio on any host
  - device: "Built-in Audio"
    track_mappings:
      click: [1]
      backing-track-l: [1]
      backing-track-r: [2]

# MIDI profiles tried independently (separate priority order).
midi_profiles:
  - hostname: raspberry-pi-a
    device: "Behringer WING"
    playback_delay: 500ms
    midi_to_dmx:
      - midi_channel: 15
        universe: light-show

  # Fallback: generic USB MIDI on any host
  - device: "USB MIDI Interface"
    playback_delay: 200ms

# When true (default), the player proceeds without MIDI if no device is
# available instead of retrying forever. Set to false to require a MIDI device.
midi_optional: true
```

Profiles with a `hostname` constraint only apply on hosts whose hostname matches. Profiles
without a hostname constraint match any host. Set the `MTRACK_HOSTNAME` environment variable
to override the system hostname (useful for testing or when the OS hostname differs from
what you want).

The `dmx:` section also supports hostname filtering via `enabled_hostnames`. When set,
DMX is only initialized on hosts in the list. Hosts not in the list skip DMX entirely:

```yaml
dmx:
  enabled_hostnames:
    - raspberry-pi-a
  universes:
    - universe: 1
      name: light-show
```

Omitting `enabled_hostnames` enables DMX on all hosts (existing behavior).

The existing flat format (`audio:` + `track_mappings:` + `midi:`) continues to work
unchanged. At startup, legacy fields are automatically normalized into single-entry
profiles, so all internal code paths use the same profile-based logic.

### mtrack on startup

To have `mtrack` start when the system starts, first create a dedicated system user for the service:

```
$ sudo useradd --system --no-create-home --shell /usr/sbin/nologin mtrack
$ sudo usermod -aG audio mtrack
```

The `audio` group grants access to ALSA sound cards and MIDI devices. If your DMX USB adapter
requires a specific group (e.g. `plugdev` or `dialout`), add that as well:

```
$ sudo usermod -aG plugdev mtrack
```

Next, generate and install the systemd service file:

```
$ sudo mtrack systemd > /etc/systemd/system/mtrack.service
```

The service expects that `mtrack` is available at the location `/usr/local/bin/mtrack`. It also
expects you to define your player configuration in `/etc/default/mtrack`. This file
should contain one variable: `MTRACK_CONFIG`:

```
# The configuration for the mtrack player.
MTRACK_CONFIG=/mnt/storage/mtrack.yaml
```

Make sure the `mtrack` user can read your configuration and song files:

```
$ sudo chown -R mtrack:mtrack /mnt/storage
```

Once that's defined, you can start it with:

```
$ sudo systemctl daemon-reload
$ sudo systemctl enable mtrack
$ sudo systemctl start mtrack
```

It will now be running and will restart when you reboot your machine. You'll be able to view the logs
for `mtrack` by running:

```
$ journalctl -u mtrack
```

### Service hardening

The generated systemd service includes security hardening that runs `mtrack` with minimal
privileges. This is the recommended configuration for production deployments.

**User isolation**: The service runs as the unprivileged `mtrack` user instead of root. The
`audio` supplementary group provides access to ALSA and MIDI devices under `/dev/snd/`.

**Real-time audio scheduling**: `AmbientCapabilities=CAP_SYS_NICE` allows the `mtrack` user
to set elevated thread priorities and use `SCHED_FIFO` real-time scheduling for the audio
callback thread, without requiring root. `CapabilityBoundingSet=CAP_SYS_NICE` ensures this
is the only capability the process can ever acquire.

**Filesystem restrictions**: `ProtectSystem=strict` makes the entire filesystem hierarchy
read-only, which is sufficient since `mtrack` does not write to disk (logs are emitted to
stdout/stderr and captured by journald). `ProtectHome=true` makes `/home`, `/root`, and
`/run/user` completely inaccessible. `PrivateTmp=true` provides an isolated temporary
directory.

**Kernel restrictions**: The service cannot modify kernel tunables (`ProtectKernelTunables`),
load kernel modules (`ProtectKernelModules`), access the kernel log buffer
(`ProtectKernelLogs`), or modify control groups (`ProtectControlGroups`).

**Additional hardening**: The service is further restricted with `NoNewPrivileges` (cannot
gain new privileges via setuid/setgid binaries or filesystem capabilities),
`MemoryDenyWriteExecute` (no writable-executable memory pages), `SystemCallArchitectures=native`
(only native architecture syscalls), `LockPersonality` (cannot change execution domain),
`RestrictNamespaces` (cannot create user/network/mount namespaces), and
`RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX` (only IPv4, IPv6, and Unix socket access).

**Troubleshooting**: If `mtrack` cannot access your audio or MIDI devices after setup, verify
group membership with `groups mtrack` and check device permissions with
`ls -la /dev/snd/`. If you encounter permission errors related to a specific restriction,
you can override individual directives by creating a drop-in:

```
$ sudo systemctl edit mtrack
```

```ini
# For example, to disable memory execution restrictions if a dependency requires it:
[Service]
MemoryDenyWriteExecute=false
```

### Supported MIDI events

As of now, the following MIDI events can be defined as part of the controller and song emit features:

```yaml
# The note_off MIDI event acts as if a note was released.
midi_event:
  type: note_off
  channel: 5 # Channels are expected to be from 1-16.
  note: 5
  velocity: 127
---
# The note_on MIDI event acts as if a note was pressed.
midi_event:
  type: note_on
  channel: 5
  note: 5
  velocity: 127
---
# The aftertouch MIDI event acts as if an aftertouch MIDI event was sent.
midi_event:
  type: aftertouch
  channel: 5
  note: 5
  velocity: 127
---
# The control_change MIDI event can controller values.
midi_event:
  type: control_change
  channel: 5
  controller: 12
  value: 27
---
# The program_change MIDI event can change banks and instruments on various devices.
midi_event:
  type: program_change
  channel: 5
  program: 20
---
# The aftertouch MIDI event acts as if a channel aftertouch MIDI event was sent.
midi_event:
  type: channel_aftertouch
  channel: 5
  velocity: 127
---
# The pitch bend MIDI event acts as if a pitch bend MIDI event was sent.
midi_event:
  type: pitch_bend
  bend: 1234
```

There are more that can be implemented, but these are just the ones that came to me at the moment.
If you'd like to add any particular ones, please file an issue. Otherwise I'll add them in as they
strike me.

## gRPC Control

The player can now be controlled using gRPC calls. The definition for the service can be found
[here](src/proto/player/v1/player.proto). By default, this runs on port 43234.

The `mtrack` command itself supports several subcommands for gRPC interaction of the running
player:

```
$ mtrack play
$ mtrack play --from "1:23.456"  # Start playback from a specific time
$ mtrack previous
$ mtrack next
$ mtrack stop
$ mtrack switch-to-playlist all_songs|playlist
$ mtrack status
$ mtrack active-effects  # Print all active lighting effects
$ mtrack cues  # List all cues in the current song's lighting timeline
```

This will allow for multiple, arbitrary connections to the player, potentially from clients
outside the device the player is running on. It should also be handy for "oh crap" moments at
gigs when your MIDI controller isn't behaving well. While not ideal, you'll still at least
be able to control the player.

One note: there is no security on this at present. I don't advise running `mtrack` on a public
network to begin with, but I would advise disabling the gRPC server if for some reason the
network the player is running on is wide open.

## OSC Control

The player can also be controlled using arbitrary OSC commands. This is configurable in the OSC
controller configuration section. This allows you to define OSC addresses that will map to
player events (play, previous, next, stop, all_songs, playlist). Refer to the example
configuration above for the exact name of these events.

Additionally, information can be reported back to a fixed list of clients from the OSC server.
This will allow OSC clients to display things like the current song the playlist is pointing to,
whether or not the player is currently playing, how much time has elapsed, and the contents of
the playlist. Again, refer to the example configuration above for the defaults for these events.

An starting TouchOSC file has been supplied [here](touchosc/mtrack.tosc).

## MIDI-Triggered Samples

`mtrack` supports triggering audio samples via MIDI events. This is useful for playing one-shot sounds like clicks, cues, sound effects, or drum samples during a performance. Samples are preloaded into memory and transcoded at startup for low-latency playback. Trigger latency is approximately 2x the audio buffer size (e.g., ~11.6ms at 256 samples/44.1kHz).

### Global vs Per-Song Samples

Samples can be configured at two levels:

1. **Global samples** - Defined in the main `mtrack.yaml` configuration file. These are available throughout the entire session.
2. **Per-song samples** - Defined in individual song configuration files. These override or extend the global configuration when that song is selected.

### Sample Configuration

Samples are defined in two parts: **sample definitions** (the audio files and their behavior) and **sample triggers** (the MIDI events that play them).

#### Sample Definitions

```yaml
samples:
  # Each sample has a name that can be referenced by triggers.
  kick:
    # The audio file to play. Path is relative to the config file.
    file: samples/kick.wav
    
    # Output channels to route this sample to (1-indexed).
    output_channels: [3, 4]
    
    # Velocity handling configuration.
    velocity:
      # Mode can be: ignore, scale, or layers.
      mode: scale
    
    # Behavior when Note Off is received: play_to_completion, stop, or fade.
    note_off: play_to_completion
    
    # Behavior when retriggered while playing: cut or polyphonic.
    retrigger: cut
    
    # Maximum concurrent voices for this sample (optional).
    max_voices: 4
    
    # Fade time in milliseconds when note_off is "fade" (default: 50).
    fade_time_ms: 100
```

#### Sample Triggers

Triggers map MIDI events to samples. For Note On/Off events, only the channel and key are matched - the velocity from the incoming MIDI event is used for volume scaling or layer selection.

```yaml
sample_triggers:
  # Map a MIDI Note On event to a sample.
  # The velocity from the incoming MIDI event is used for volume/layer selection.
- trigger:
    type: note_on
    channel: 10
    key: 60  # C3
  sample: kick

- trigger:
    type: note_on
    channel: 10
    key: 62  # D3
  sample: snare
```

### Velocity Handling Modes

#### Ignore Mode

Ignores the MIDI velocity and plays at a fixed volume:

```yaml
velocity:
  mode: ignore
  default: 100  # Fixed velocity (0-127), defaults to 100
```

#### Scale Mode

Scales the playback volume based on MIDI velocity (velocity/127):

```yaml
velocity:
  mode: scale
```

#### Layers Mode

Selects different audio files based on velocity ranges. Useful for realistic drum sounds:

```yaml
velocity:
  mode: layers
  # Optional: also scale volume within each layer.
  scale: true
  layers:
  - range: [1, 60]      # Soft hits
    file: samples/snare_soft.wav
  - range: [61, 100]    # Medium hits
    file: samples/snare_medium.wav
  - range: [101, 127]   # Hard hits
    file: samples/snare_hard.wav
```

### Note Off Behavior

Controls what happens when a MIDI Note Off event is received:

- **`play_to_completion`** (default) - Ignores Note Off, lets the sample play to the end.
- **`stop`** - Immediately stops the sample.
- **`fade`** - Fades out the sample over the configured `fade_time_ms`.

### Retrigger Behavior

Controls what happens when a sample is triggered while it's already playing:

- **`cut`** (default) - Stops the previous instance and starts a new one.
- **`polyphonic`** - Allows multiple instances to play simultaneously.

### Voice Limits

To prevent resource exhaustion, you can limit concurrent voices:

```yaml
# Global limit for all samples.
max_sample_voices: 32

samples:
  hihat:
    # Per-sample limit (in addition to global limit).
    max_voices: 8
```

When limits are exceeded, the oldest voice is stopped to make room for new ones.

### Stopping All Samples

All triggered samples can be stopped via:

- **OSC**: Send a message to `/mtrack/samples/stop` (configurable via `stop_samples` in OSC controller config)
- **gRPC**: Call the `StopSamples` RPC method

### Per-Song Sample Overrides

Individual songs can override or extend the global sample configuration:

```yaml
# In a song's configuration file (e.g., songs/my-song/song.yaml)
name: My Song

tracks:
- file: click.wav
  name: click
- file: backing.wav
  name: backing

# Override global samples for this song.
samples:
  kick:
    file: custom_kick.wav  # Use a different kick for this song
    output_channels: [5, 6]

# Add song-specific triggers.
sample_triggers:
- trigger:
    type: note_on
    channel: 10
    key: 64
  sample: kick
```

## Light Show Verification

You can verify the syntax of a light show file using the `verify-light-show` command:

```
$ mtrack verify-light-show path/to/show.light
```

This will check the syntax of the light show file and report any errors. You can also validate
the show against your mtrack configuration to ensure all referenced groups and fixtures exist:

```
$ mtrack verify-light-show path/to/show.light --config /path/to/mtrack.yaml
```

This will verify that:
- The light show syntax is valid
- All referenced fixture groups exist in your configuration
- All referenced fixtures exist in your configuration

## Light shows

Light shows and DMX playback are now supported through the use of the [Open Lighting Architecture](https://www.openlighting.org/).
The lighting system has been significantly enhanced with a new tag-based group resolution system that enables venue-agnostic lighting shows.

### New Lighting System Features

The new lighting system provides:

- **Venue-Agnostic Songs**: Songs use logical groups instead of specific fixture names
- **Tag-Based Group Resolution**: Fixtures are tagged with capabilities and roles
- **Intelligent Selection**: System automatically chooses optimal fixtures based on constraints
- **Venue Portability**: Same lighting show works across different venues
- **Performance Optimization**: Cached group resolutions for fast lookups

### Configuration Structure

The lighting system uses a three-layer architecture:

1. **Configuration Layer**: Define logical groups with constraints in `mtrack.yaml`
2. **Venue Layer**: Tag physical fixtures with capabilities in DSL files
3. **Song Layer**: Reference `.light` DSL files in song YAML files, which use logical groups

#### Main Configuration (`mtrack.yaml`)

```yaml
dmx:
  # ... existing DMX configuration ...
  
  # New lighting system configuration
  lighting:
    # Current venue selection - determines which physical fixtures to use
    current_venue: "main_stage"
    
    # Simple inline fixture definitions (for basic cases)
    # These can be used instead of or alongside venue definitions
    fixtures:
      emergency_light: "Emergency @ 1:500"
    
    # Logical groups with role-based constraints
    groups:
      # Front wash lights - requires wash + front tags, needs 4-8 fixtures
      front_wash:
        name: "front_wash"
        constraints:
          - AllOf: ["wash", "front"]
          - MinCount: 4
          - MaxCount: 8
      
      # Moving head lights - accepts moving_head OR spot tags, prefers premium
      movers:
        name: "movers"
        constraints:
          - AnyOf: ["moving_head", "spot"]
          - Prefer: ["premium"]
          - MinCount: 2
          - MaxCount: 4
      
      # All lights - accepts any light type
      all_lights:
        name: "all_lights"
        constraints:
          - AnyOf: ["wash", "moving_head", "spot", "strobe", "beam"]
          - MinCount: 1
    
    # Directory configuration for DSL files (auto-discovered)
    directories:
      fixture_types: "lighting/fixture_types"
      venues: "lighting/venues"
```

#### Fixture Type Definitions (`lighting/fixture_types/`)

```light
# RGBW Par Can fixture type definition
fixture_type "RGBW_Par" {
  channels: 4
  channel_map: {
    "dimmer": 1,
    "red": 2,
    "green": 3,
    "blue": 4
  }
  special_cases: ["RGB", "Dimmer"]
}

# Moving Head fixture type definition
fixture_type "MovingHead" {
  channels: 16
  channel_map: {
    "dimmer": 1,
    "pan": 2,
    "pan_fine": 3,
    "tilt": 4,
    "tilt_fine": 5,
    "color_wheel": 6,
    "gobo_wheel": 7,
    "gobo_rotation": 8,
    "focus": 9,
    "zoom": 10,
    "iris": 11,
    "frost": 12,
    "prism": 13,
    "effects": 14,
    "strobe": 15,
    "control": 16
  }
  special_cases: ["MovingHead", "Spot", "Dimmer", "Strobe"]
}
```

#### Venue Definitions (`lighting/venues/`)

```light
# Main Stage venue definition
venue "main_stage" {
  # Front wash lights
  fixture "Wash1" RGBW_Par @ 1:1 tags ["wash", "front", "rgb", "premium"]
  fixture "Wash2" RGBW_Par @ 1:7 tags ["wash", "front", "rgb", "premium"]
  fixture "Wash3" RGBW_Par @ 1:13 tags ["wash", "front", "rgb"]
  fixture "Wash4" RGBW_Par @ 1:19 tags ["wash", "front", "rgb"]
  
  # Moving head lights
  fixture "Mover1" MovingHead @ 1:37 tags ["moving_head", "spot", "premium"]
  fixture "Mover2" MovingHead @ 1:53 tags ["moving_head", "spot", "premium"]
  fixture "Mover3" MovingHead @ 1:69 tags ["moving_head", "spot"]
  
  # Strobe lights
  fixture "Strobe1" Strobe @ 1:85 tags ["strobe", "front"]
  fixture "Strobe2" Strobe @ 1:87 tags ["strobe", "back"]
}

# Small Club venue definition (same logical groups work!)
venue "small_club" {
  # Limited front wash (only 2 fixtures)
  fixture "Wash1" RGBW_Par @ 1:1 tags ["wash", "front", "rgb"]
  fixture "Wash2" RGBW_Par @ 1:7 tags ["wash", "front", "rgb"]
  
  # Single moving head
  fixture "Mover1" MovingHead @ 1:13 tags ["moving_head", "spot", "premium"]
  
  # Single strobe
  fixture "Strobe1" Strobe @ 1:29 tags ["strobe", "front"]
}
```

#### Song Lighting Definitions

Lighting shows are defined in separate `.light` files using the DSL format. Songs reference these files:

```yaml
# Example song.yaml file
name: "My Song"
lighting:
  - file: "lighting/main_show.light"  # Path relative to song directory
  - file: "lighting/outro.light"      # Multiple shows can be referenced
tracks:
  - name: "backing-track"
    file: "backing-track.wav"  # Can be WAV, MP3, FLAC, OGG, AAC, ALAC, etc.
```

The `.light` files use the DSL format and can reference logical groups defined in your `mtrack.yaml`:

```light
show "Main Show" {
    # Front wash on - uses logical group from mtrack.yaml
    @00:05.000
    front_wash: static color: "red", dimmer: 80%
    
    # Movers join with color cycle - uses logical group
    @00:10.000
    movers: cycle color: "red", color: "blue", color: "green", speed: 2.0, dimmer: 100%
}
```

See the [Light Show Verification](#light-show-verification) section for information on validating your `.light` files.

## Lighting Effects Reference

The lighting system supports a variety of effect types, each with specific parameters and use cases.

### Effect Types

#### Static Effect

Sets fixed parameter values for fixtures. Useful for solid colors, fixed dimmer levels, or any unchanging state.

**Parameters:**
- `color`: Color name (e.g., `"red"`, `"blue"`), hex (`#FF0000`), or RGB (`rgb(255,0,0)`)
- `dimmer`: Dimmer level (0-100% or 0.0-1.0)
- `red`, `green`, `blue`, `white`: Individual color channel levels (0-100% or 0.0-1.0)
- `duration`: Optional duration after which effect stops (e.g., `5s`, `2measures`)

**Example:**
```light
@00:05.000
front_wash: static color: "red", dimmer: 80%

@00:10.000
back_wash: static red: 100%, green: 50%, blue: 0%, dimmer: 60%, duration: 5s
```

#### Color Cycle Effect

Cycles through a list of colors continuously. Colors transition smoothly or instantly based on transition mode.

**Parameters:**
- `color`: Multiple color values (e.g., `color: "red", color: "green", color: "blue"`)
- `speed`: Cycles per second, or tempo-aware (e.g., `1.5`, `1measure`, `2beats`)
- `direction`: `forward`, `backward`, or `pingpong`
- `transition`: `snap` (instant) or `fade` (smooth)
- `duration`: Optional duration

**Example:**
```light
@00:10.000
movers: cycle color: "red", color: "blue", color: "green", speed: 2.0, direction: forward, transition: fade
```

#### Strobe Effect

Rapidly flashes fixtures on and off at a specified frequency.

**Parameters:**
- `frequency`: Flashes per second (Hz), or tempo-aware (e.g., `8`, `1beat`, `0.5measures`)
- `duration`: Optional duration (e.g., `3s`, `4measures`)

**Example:**
```light
@00:15.000
strobes: strobe frequency: 8, duration: 2s

@01:00.000
all_lights: strobe frequency: 1beat, duration: 4measures
```

#### Pulse Effect

Smoothly pulses the dimmer level up and down, creating a breathing effect.

**Parameters:**
- `base_level`: Base dimmer level (0-100% or 0.0-1.0)
- `pulse_amplitude` or `intensity`: Amplitude of the pulse (0-100% or 0.0-1.0)
- `frequency`: Pulses per second (Hz), or tempo-aware (e.g., `2`, `1beat`)
- `duration`: Optional duration

**Example:**
```light
@00:20.000
front_wash: pulse base_level: 50%, pulse_amplitude: 50%, frequency: 1.5, duration: 5s
```

#### Chase Effect

Moves an effect pattern across multiple fixtures in a spatial pattern.

**Parameters:**
- `pattern`: `linear`, `snake`, or `random`
- `speed`: Steps per second, or tempo-aware (e.g., `2.0`, `1measure`)
- `direction`: `left_to_right`, `right_to_left`, `top_to_bottom`, `bottom_to_top`, `clockwise`, `counter_clockwise`
- `transition`: `snap` or `fade` for transitions between fixtures

**Example:**
```light
@00:25.000
movers: chase pattern: linear, speed: 2.0, direction: left_to_right, transition: fade
```

#### Dimmer Effect

Smoothly transitions dimmer level from start to end over a duration.

**Parameters:**
- `start_level` or `start`: Starting dimmer level (0-100% or 0.0-1.0)
- `end_level` or `end`: Ending dimmer level (0-100% or 0.0-1.0)
- `duration`: Transition duration (e.g., `3s`, `2measures`)
- `curve`: Transition curve - `linear`, `exponential`, `logarithmic`, `sine`, `cosine`

**Example:**
```light
@00:30.000
all_lights: dimmer start_level: 100%, end_level: 0%, duration: 3s, curve: sine
```

#### Rainbow Effect

Generates a continuous rainbow color cycle across the color spectrum.

**Parameters:**
- `speed`: Cycles per second, or tempo-aware (e.g., `1.0`, `1measure`)
- `saturation`: Color saturation (0-100% or 0.0-1.0)
- `brightness`: Overall brightness (0-100% or 0.0-1.0)

**Example:**
```light
@00:35.000
all_lights: rainbow speed: 1.0, saturation: 100%, brightness: 80%
```

### Common Effect Parameters

All effects support these optional parameters for advanced control:

- `layer`: Effect layer - `background`, `midground`, or `foreground` (for layering)
- `blend_mode`: How effect blends with lower layers - `replace`, `multiply`, `add`, `overlay`, `screen`
- `up_time`: Fade-in duration (e.g., `2s`, `1beat`)
- `hold_time`: Duration to hold at full intensity (e.g., `5s`, `4measures`)
- `down_time`: Fade-out duration (e.g., `1s`, `2beats`)

**Example with crossfades:**
```light
@00:05.000
front_wash: static color: "blue", dimmer: 100%, up_time: 2s, hold_time: 5s, down_time: 1s
```

## Cueing Features

Light shows support flexible cueing with time-based and measure-based timing, loops, sequences, and offset commands.

### Time-Based Cues

Cues can be specified using absolute time in two formats:

**Format 1: Minutes:Seconds.Milliseconds**
```light
@00:05.000    # 5 seconds
@01:23.456    # 1 minute, 23.456 seconds
@02:00.000    # 2 minutes
```

**Format 2: Seconds.Milliseconds**
```light
@5.000        # 5 seconds
@83.456       # 83.456 seconds
@120.000      # 120 seconds (2 minutes)
```

**Example:**
```light
show "Time-Based Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 0%
    
    @00:05.000
    front_wash: static color: "blue", dimmer: 100%
    
    @00:10.500
    movers: cycle color: "red", color: "green", speed: 2.0
}
```

### Measure-Based Cues

When a tempo section is defined, cues can use measure/beat notation that automatically adjusts to tempo changes.

**Format: `@measure/beat` or `@measure/beat.subdivision`**
```light
@1/1         # Measure 1, beat 1
@2/3         # Measure 2, beat 3
@4/1.5       # Measure 4, halfway through beat 1
@8/2.75      # Measure 8, three-quarters through beat 2
```

**Example with tempo:**
```light
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Measure-Based Show" {
    @1/1
    front_wash: static color: "red", dimmer: 100%
    
    @2/1
    back_wash: static color: "blue", dimmer: 100%
    
    @4/2.5
    movers: strobe frequency: 1beat, duration: 2measures
}
```

### Tempo Sections

Tempo sections define BPM, time signature, and tempo changes throughout the show.

**Basic tempo:**
```light
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}
```

**Tempo with changes:**
```light
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
        @8/1 { bpm: 140 },                    # Instant change at measure 8
        @16/1 { bpm: 160, transition: 4 },    # Gradual change over 4 beats
        @24/1 { bpm: 180, transition: 2m },   # Gradual change over 2 measures
        @32/1 { time_signature: 3/4 },        # Time signature change
        @40/1 { bpm: 100, transition: snap }  # Instant snap back
    ]
}
```

**Tempo change parameters:**
- `bpm`: New BPM value
- `time_signature`: New time signature (e.g., `3/4`, `6/8`)
- `transition`: Duration of tempo change - number of beats, `Xm` for measures, or `snap` for instant

### Inline Loops

Repeat a block of cues inline without defining a separate sequence.

**Syntax:**
```light
@00:10.000
loop {
    @0.000
    front_wash: static color: "red", dimmer: 100%
    
    @0.500
    front_wash: static color: "blue", dimmer: 100%
    
    @1.000
    front_wash: static color: "green", dimmer: 100%
} repeats: 4
```

Timing inside loops is relative to the loop start time. The example above creates 4 cycles of red-blue-green, each cycle taking 1 second.

### Sequences (Subsequences)

Define reusable cue sequences that can be referenced multiple times.

**Defining a sequence:**
```light
sequence "Verse Pattern" {
    @1/1
    front_wash: static color: "blue", dimmer: 80%
    
    @2/1
    front_wash: static color: "red", dimmer: 100%
    
    @4/1
    front_wash: static color: "blue", dimmer: 80%
}
```

**Referencing a sequence:**
```light
show "Song" {
    @1/1
    sequence "Verse Pattern"
    
    @17/1
    sequence "Verse Pattern"  # Reuse the same pattern
    
    @33/1
    sequence "Verse Pattern", loop: 2  # Loop the sequence twice
}
```

**Sequence parameters:**
- `loop`: Number of times to loop (`once`, `loop` for infinite, or a number)

### Measure Offsets

Shift the measure counter for subsequent cues, useful for complex timing, reusing sequences at different positions, or aligning with composition tools that use repeats.

**Offset command:**
```light
@8/1
offset 4 measures    # Shift measure counter forward by 4 measures
# Next cue at @8/1 will actually be at measure 12

@12/1
reset_measures      # Reset measure counter back to actual playback time
```

**Example use case:**
```light
show "Complex Timing" {
    @1/1
    front_wash: static color: "red", dimmer: 100%
    
    @4/1
    offset 8 measures    # Shift forward 8 measures
    # Now @4/1 actually plays at measure 12
    
    @4/1
    back_wash: static color: "blue", dimmer: 100%  # Plays at measure 12
    
    @8/1
    reset_measures       # Reset counter
    # Now back to actual playback time
    
    @9/1
    movers: strobe frequency: 4  # Plays at actual measure 9
}
```

### Using Composition Tools as Reference

When composing light shows, you can use tools like Guitar Pro, MuseScore, or other notation software as a reference. These tools often use repeat signs that make measure numbers in the score differ from actual playback position.

**The Problem:**
In Guitar Pro, if you have a 4-measure intro that repeats 3 times, the score might show:
- Measures 1-4: Intro (first time)
- Measures 1-4: Intro (repeat 1)
- Measures 1-4: Intro (repeat 2)
- Measure 5: Verse starts

But in actual playback, measure 5 appears at measure 13 (4 + 4 + 4 + 1). If you write your light show using the score's measure numbers, cues won't align with playback.

**The Solution:**
Use `offset` commands to shift the measure counter to match where sections actually play:

```light
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Song with Repeats" {
    # Intro section (measures 1-4, plays 3 times)
    # First time through
    @1/1
    front_wash: static color: "blue", dimmer: 50%
    
    @4/1
    front_wash: static color: "blue", dimmer: 100%
    
    # After first repeat (4 measures later)
    offset 4 measures
    @1/1
    back_wash: static color: "red", dimmer: 50%  # Actually plays at measure 5
    
    @4/1
    back_wash: static color: "red", dimmer: 100%  # Actually plays at measure 8
    
    # After second repeat (8 more measures from start, 4 from previous offset)
    offset 4 measures
    @1/1
    movers: strobe frequency: 2  # Actually plays at measure 9
    
    @4/1
    movers: strobe frequency: 4  # Actually plays at measure 12
    
    # Verse starts at measure 13 (after 3x4 measure intro)
    offset 4 measures
    @1/1
    reset_measures  # Reset to actual playback time
    # Now we're at measure 13 in actual playback
    
    @1/1
    all_lights: static color: "green", dimmer: 100%  # Plays at actual measure 13
    
    @4/1
    all_lights: cycle color: "green", color: "yellow", speed: 2.0  # Plays at measure 16
}
```

**Workflow:**
1. Create your light show using measure numbers from your composition tool (Guitar Pro, etc.)
2. Identify where repeats occur and calculate the cumulative offset
3. Add `offset X measures` commands after each repeat section
4. Use `reset_measures` when you want to return to actual playback time
5. Continue with measure numbers that match actual playback

**Example with Guitar Pro Structure:**
```
Guitar Pro Score Structure:
- Measures 1-4: Intro (repeats 3x)
- Measures 5-12: Verse
- Measures 13-16: Chorus
- Measures 17-20: Verse (repeat)
- Measures 21-24: Chorus (repeat)
- Measure 25: Outro

Actual Playback:
- Measures 1-12: Intro (3x4 measures)
- Measures 13-20: Verse
- Measures 21-24: Chorus
- Measures 25-28: Verse (repeat)
- Measures 29-32: Chorus (repeat)
- Measure 33: Outro
```

```light
show "Guitar Pro Aligned Show" {
    # Intro section (measures 1-4, plays 3 times = 12 measures total)
    @1/1
    front_wash: static color: "blue", dimmer: 30%
    
    @4/1
    front_wash: static color: "blue", dimmer: 100%
    
    # After intro repeats, offset by 12 measures (3 repeats × 4 measures)
    offset 12 measures
    
    # Verse (score shows measures 5-12, actually plays at 13-20)
    @5/1
    reset_measures  # Reset to actual playback (now at measure 13)
    all_lights: static color: "green", dimmer: 80%
    
    @12/1
    all_lights: cycle color: "green", color: "yellow", speed: 1.5
    
    # Chorus (score shows measures 13-16, actually plays at 21-24)
    @13/1
    all_lights: static color: "red", dimmer: 100%
    
    @16/1
    movers: strobe frequency: 8, duration: 1measure
    
    # Verse repeat (score shows measures 17-20, actually plays at 25-28)
    @17/1
    offset 4 measures  # Chorus was 4 measures, so offset by 4
    reset_measures
    all_lights: static color: "green", dimmer: 80%
    
    # Chorus repeat (score shows measures 21-24, actually plays at 29-32)
    @21/1
    offset 4 measures
    reset_measures
    all_lights: static color: "red", dimmer: 100%
    
    # Outro (score shows measure 25, actually plays at measure 33)
    @25/1
    offset 4 measures
    reset_measures
    all_lights: dimmer start_level: 100%, end_level: 0%, duration: 4s
}
```

This approach lets you write light shows using the same measure numbers as your composition tool, making it easier to sync lighting with your musical arrangement.

### Stopping Sequences

Stop a running sequence at a specific cue time.

**Syntax:**
```light
@00:30.000
stop sequence "Verse Pattern"
```

This stops the named sequence if it's currently playing.

### Constraint Types

The system supports several constraint types for group resolution:

- **`AllOf`**: All specified tags must be present (e.g., `["wash", "front"]`)
- **`AnyOf`**: Any of the specified tags must be present (e.g., `["moving_head", "spot"]`)
- **`Prefer`**: Prefer fixtures with these tags (e.g., `["premium"]`)
- **`MinCount`**: Minimum number of fixtures required
- **`MaxCount`**: Maximum number of fixtures allowed
- **`FallbackTo`**: Fallback to another group if primary group fails (e.g., `"all_lights"`)
- **`AllowEmpty`**: Allow group to be empty if no fixtures match (graceful degradation, e.g., `true`)

### Benefits

1. **Venue Portability**: Same lighting show works across different venues automatically
2. **Intelligent Selection**: System prefers premium fixtures when available, falls back to standard
3. **Flexible Constraints**: Support for complex requirement combinations
4. **Clear Error Handling**: Know exactly what's missing when requirements aren't met
5. **Performance**: Cached resolutions for fast lookups
6. **Maintainable**: Easy to add new venues and fixture types

### Migration Path

- **Gradual adoption** - can mix old and new group definitions
- **Venue-defined groups** - venue-defined groups are still supported alongside logical groups

## MIDI-Based DMX System

> **Note**: The new tag-based lighting system (described above) is the recommended approach for most users. The MIDI-based DMX system is still supported for specific use cases requiring direct channel control.

### When to Use Each System

**Use the New Tag-Based System when:**
- You want venue-agnostic lighting shows
- You prefer high-level effect definitions
- You want intelligent fixture selection
- You're creating new lighting shows

**Use the MIDI-Based DMX System when:**
- You have existing MIDI-based lighting shows
- You need precise channel-level control
- You're integrating with existing MIDI workflows
- You prefer direct DMX channel programming

### Basic DMX Information

DMX is a standard that allows for the controlling of stage devices, primarily lights. Each of these devices will react to data being
fed into one or more DMX channels. Each DMX channel can be set from `0` to `255`. For example, a multicolor stage light might have 3
DMX channels: 1 for red, 1 for green, 1 for blue. In order to set the color of the light, you would have to supply these channels with
data representing the color that you want. DMX data is arranged into universes, where 1 universe consists of 512 channels of DMX data.

### Configuring mtrack for Legacy DMX Playback

In order to use legacy light shows, you'll need to set up OLA on your playback device and map your DMX devices into DMX universes. I recommend
following [this tutorial](https://www.openlighting.org/ola/getting-started/). mtrack assumes that OLA is running on the same device.

mtrack can be configured to stream DMX data to OLA universes. This can be done through the mtrack configuration file when using `mtrack start`
or through the command line when using `mtrack play` using the `--dmx-dimming-speed-modifier` argument and the `dmx-universe-config` arguments.
The `dmx-universe-config` argument format is:

```
universe=1,name=light-show;universe=2,name=another-light-show
```

Legacy light shows can be defined in `Song` files and consist of an array of "universe names" and MIDI files. These universe names correlate to the
names used in the mtrack configuration. For instance, a song with a light show with a universe name of `light-show` will play on the mtrack
universe with the equivalent name.

Additionally, songs can be defined to only recognize specific MIDI channels from the given MIDI file as lighting data. For instance, if you
have a single MIDI file that contains all of your automation, you can restrict light shows to only recognize events from channel 15.

Examples for these configuration options are in the song definition example and mtrack player examples above.

#### Live MIDI to DMX mapping

mtrack is also capable of mapping live MIDI events into the DMX engine. This allows for live control of lighting using a
MIDI controller. Additionally, transformations can be applied to the incoming MIDI events that allow for singular MIDI messages
to be transformed into multiple MIDI messages and then fed into the DMX engine, allowing for the control multiple lights. Right now,
the transformers supported are:

- Note Mapper: This maps one note on a MIDI Channel to multiple notes, all with the same velocity. Works for both note_on and note_off events.
- Control Change Mapper: This maps one control change event on a MIDI Channel to multiple control change events, all with the same value.

Right now collision behavior is undefined. The intention is to provide some sort of composable mechanism here, so it's very possible that
this interface will change in the future.

### MIDI format

The MIDI engine was heavily inspired by the [DecaBox MIDI to DMX converter](https://response-box.com/gear/product/decabox-protocol-converter-basic-firmware/), with the MIDI to DMX conversion mechanism being described
[here](http://67.205.146.177/books/decabox-midi-to-dmx-converter).

Note here that MIDI is an older protocol and doesn't have the same resolution that DMX does. As a result, we have
to do some munging here in order to make this work. Some other notes:

- `u7` is an unsigned 7 bit integer, which ranges from 0-127.
- mtrack only supports 127 DMX channels per universe at present.

MIDI data is converted into DMX data as follows:

| MIDI Event | Outputs | Description |
|------------|---------|-------------|
| key on/off | key (`u7`), velocity(`u7`) | The value of _key_ is interpreted as the DMX channel, and _velocity_ is doubled and assigned to the channel |
| program change | program (`u7`) | The dimming speed. 0 means instantaneous, any other number is multiplied by the dimming speed modifier and used as a duration. |
| continuous controller | controller (`u7`), value (`u7`) | Similar to key on/off, the value of the _controller_ is interpreted as the DMX channel, and _value_ is doubled and assigned to the channel. Ignores dimming. |

The general idea here is to create a MIDI file that generally describes the way you want your lights to display. Much like regular MIDI
automation, you can program some pretty dynamic lights this way.

### Dimming engine

The dimming engine built into mtrack is controlled by program change (PC) commands. The value of the PC command will be multiplied by
the dimming speed modifier and will produce a duration. Subsequent key on/off commands will gradually progress to their new value
over this duration. For example, a dimming speed modifier of `0.25` and a PC command with a `1` will produce a dimming duration of `0.25`.
New key on/off events will take `0.25` seconds to reach the new value. PC0 will ensure color changes are instantaneous.

Dimming of channels is independent of one another. Imagine a lifecycle that looks like this, assuming a dimming speed modifier of 1:

```
PC5 --> key_on(0, 127) --> PC10 --> key_on(1, 127)
```

A PC command instructs the dimmer to dim over 5 seconds. The first `key_on` event will gradually progress channel from 0 to 127 over 5 seconds.
After this, another PC command instructs the dimmer to dim over 10 seconds. The second `key_on` event will gradually progress channel 1 from 0
to 127 over 10 seconds. This will not affect channel 0, which will still only take 5 seconds.

Continuous controller (CC) messages will ignore dimming.

## Known limitations

This has been tested with:

### Audio cards
- MOTU UltraLite-mk5 
- Behringer X32 (through X-Live card)
- Behringer Wing Rack

### MIDI
- MOTU UltraLite-mk5
- Roland UM-ONE
- CME U6MIDI Pro

### DMX

DMX is expected to be well supported through OLA, but the devices that have been explicitly tested:

- Entec DMX USB Pro
- RatPac Satellite (Art-Net and sACN)
- Cinelex Skycast A (sACN)

### General disclaimer

This is my first Rust project, so this is likely cringey, horrible non-idiomatic Rust. Feel free to
submit PRs to make this better.
