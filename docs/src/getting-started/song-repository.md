# Song Repository

## Song repository

The song repository is a location on disk that houses both your backing tracks, MIDI files, and song
definitions. The song repository does not have to be in any particular layout, as `mtrack` will attempt
to parse any/all config files supported by `config-rs` it finds to look for song definitions.

## Songs

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

## Generating default song configurations

Song configurations can be generated using the `songs` command as follows:


```
$ mtrack songs --init /mnt/song-storage
```

This will create a file called `song.yaml` in each subfolder of `/mnt/storage`. The name of the
subfolder determines the song's name. Audio files (WAV, MP3, FLAC, OGG, AAC, ALAC, etc.) are used as tracks. The track's name is
determined using the file name and the number of channels within the file. MIDI files are used as
MIDI playback, MIDI files that start with `dmx_` will be used as light shows. You can edit the generated files to refine the settings to your needs.
