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
$ cargo install mtrack
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

## Structure of an mtrack repository and supporting files

### Song repository

The song repository is a location on disk that houses both your backing tracks, MIDI files, and song
definitions. The song repository does not have to be in any particular layout, as `mtrack` will attempt
to parse any/all `yaml` files it finds to look for song definitions.

### Songs

A song comprises of:

- One or more audio files.
- An optional MIDI file.
- A song definition.

The audio files must all be the same bitrate. They do not need to be the same length. mtrack player will
play until the last audio (or MIDI) file is complete.

A song is defined in a `yaml` file:

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

# An optional MIDI file to play along with the audio.
midi_file: Song Automation.mid

# The tracks associated with this song.
tracks:
# The click track only has one channel, so we can just indicate which output channel
# we want directly.
- name: click
  file: click.wav # File paths are relative to the song.yaml file.
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
---
# We can define multiple songs in one file.
name: The Song Name (alternate version)
...
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

You can play individual songs by using `mtrack play`:

```
$ mtrack play -m my-midi-device my-audio-device click=1,cue=2 /mnt/song-storage "My cool song"
2024-03-22T21:24:25.588828Z  INFO emit (midir): mtrack::midi::midir: Emitting event. device="my-midi-device:my-midi-device MIDI 1 28:0" event="Midi { channel: u4(15), message: ProgramChange { program: u7(3) } }"
2024-03-22T21:24:25.589420Z  INFO player: mtrack::player: Waiting for song to finish. song="My cool song"
2024-03-22T21:24:25.589992Z  INFO play song (rodio): mtrack::audio::rodio: Playing song. device="my-audio-device" song="My cool song" duration="4:14"
2024-03-22T21:24:25.676452Z  INFO play song (midir): mtrack::midi::midir: Playing song MIDI. device="my-midi-device:my-midi-device MIDI 1 28:0" song="My cool song" duration="4:14"
```

### Playlists

The playlist definition is a pretty simple `yaml` file:

```yaml
# This is a simple file that contains, in order, the names of all songs
# that mtrack should play. The names of the songs are defined in the
# song repository, which can be found in mtrack.yaml.
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
# This audio device will be matched as best as possible against the devices on your system.
# Run `mtrack devices` to see a list of the devices that mtrack recognizes.
audio_device: UltraLite-mk5

# This MIDI device will be matched as best as possible against the devices on your system.
# Run `mtrack midi-devices` to see a list of the devices that mtrack recognizes.
midi_device: UltraLite-mk5

# The directory where all of your songs are located, frequently referred to as the song repository.
# If the path is not absolute, it will be relative to the location of this file.
songs: /mnt/song-storage

# The controller definition. As of now, the valid kinds of controllers are:
# - keyboard
# - midi
# Keyboard is largely for testing and MIDI is intended for actual live usage.
controller:
  kind: midi

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
  click: 1
  cue: 2
  backing-track-l: 3
  backing-track-r: 4
  keys: 5
```

You can start `mtrack` as a process with `mtrack start /path/to/player.yaml /path/to/playlist.yaml`.

### mtrack on startup

To have `mtrack` start when the system starts, you can run:

```
$ sudo mtrack systemd > /etc/systemd/system/mtrack.service
```

Note that the service expects that `mtrack` is available at the location `/usr/local/bin/mtrack`. It also
expects you to define your player configuration and playlist in `/etc/default/mtrack`. This file
should contain two variables: `MTRACK_CONFIG` and `PLAYLIST`:

```
# The configuration for the mtrack player.
MTRACK_CONFIG=/mnt/storage/mtrack.yaml

# The playlist to use.
PLAYLIST=/mnt/storage/playlist.yaml
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

## Known limitations

This has been tested with:

### Audio cards
- MOTU UltraLite-mk5 
- Behringer X32 (through X-Live card)
- Behringer Wing Rack

### MIDI
- MOTU UltraLite-mk5
- Roland UM-ONE

### General disclaimer

This is my first Rust project, so this is likely cringey, horrible non-idiomatic Rust. Feel free to
submit PRs to make this better.
