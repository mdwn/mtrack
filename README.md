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

`mtrack` now uses [config-rs](https://github.com/rust-cli/config-rs) for configuration parsing, which
means we should support any of the configuration file formats that it supports. Testing for anything
other than YAML is limited at the moment.

## Structure of an mtrack repository and supporting files

### Song repository

The song repository is a location on disk that houses both your backing tracks, MIDI files, and song
definitions. The song repository does not have to be in any particular layout, as `mtrack` will attempt
to parse any/all config files supported by `config-rs` it finds to look for song definitions.

### Songs

A song comprises of:

- One or more audio files.
- An optional MIDI file.
- One or more light shows (MIDI files interpreted as DMX).
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

# Light shows are MIDI files that are interpreted as DMX and sent to
# OLA (Open Lighting Architecture).
light_shows:
# The universe name is used by the player to determine which OLA universe
# to send the DMX information to.
- universe_name: light-show
  dmx_file: DMX Light Show.mid
  # You can instruct the DMX engine to only recognize specific MIDI channels as
  # having lighting data. If this is not supplied, all MIDI channels will be used
  # as lighting data.
  midi_channels:
  - 15
- universe_name: a-second-light-show
  dmx_file: DMX Light Show 2.mid

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
subfolder determines the song's name. WAV files are used as tracks. The track's name is
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
# - keyboard
# - midi
# - osc
# Keyboard is largely for testing.
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

### mtrack on startup

To have `mtrack` start when the system starts, you can run:

```
$ sudo mtrack systemd > /etc/systemd/system/mtrack.service
```

Note that the service expects that `mtrack` is available at the location `/usr/local/bin/mtrack`. It also
expects you to define your player configuration in `/etc/default/mtrack`. This file
should contain one variable: `MTRACK_CONFIG`:

```
# The configuration for the mtrack player.
MTRACK_CONFIG=/mnt/storage/mtrack.yaml
```

Once that's defined, you can start it with:

```
$ systemctl start mtrack
```

It will now be running and will restart when you reboot your machine. You'll be able to view the logs
for `mtrack` by running:

```
$ journalctl -u mtrack 
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
$ mtrack previous
$ mtrack next
$ mtrack stop
$ mtrack switch-to-playlist all_songs|playlist
$ mtrack status
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

## Light shows

Light shows and DMX playback are now supported through the use of the [Open Lighting Architecture](https://www.openlighting.org/).
Before delving too far into this, let's define some basic DMX information.

### Basic DMX information

DMX is a standard that allows for the controlling of stage devices, primarily lights. Each of these devices will react to data being
fed into one or more DMX channels. Each DMX channel can be set from `0` to `255`. For example, a multicolor stage light might have 3
DMX channels: 1 for red, 1 for green, 1 for blue. In order to set the color of the light, you would have to supply these channels with
data representing the color that you want. DMX data is arranged into universes, where 1 universe consists of 512 channels of DMX data.

### Configuring mtrack for DMX playback

In order to use light shows, you'll need to set up OLA on your playback device and map your DMX devices into DMX universes. I recommend
following [this tutorial](https://www.openlighting.org/ola/getting-started/). mtrack assumes that OLA is running on the same device.

mtrack can be configured to stream DMX data to OLA universes. This can be done through the mtrack configuration file when using `mtrack start`
or through the command line when using `mtrack play` using the `--dmx-dimming-speed-modifier` argument and the `dmx-universe-config` arguments.
The `dmx-universe-config` argument format is:

```
universe=1,name=light-show;universe=2,name=another-light-show
```

Light shows can be defined in `Song` files and consist of an array of "universe names" and MIDI files. These universe names correlate to the
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

### General disclaimer

This is my first Rust project, so this is likely cringey, horrible non-idiomatic Rust. Feel free to
submit PRs to make this better.
