# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

[0.7.0]

A major resampling bug discovered in 0.6.0 was fixed, apologies to everyone who encountered
it. 0.6.0 was pulled for this.

A new lighting engine has been added that mimics grandMA style lighting consoles. There are
multiple layers and multiple effects. It exists entirely differently from the MIDI to DMX
conversion that was written before, but it lives along side the previous functionality.
In general, I found the MIDI to DMX engine to be too cumbersome so I'm hoping this new
engine is easier to deal with. It has its own DSL as well. Feedback on all of this would be
appreciated.

The play-direct command was removed, as I think it's outlived its purpose, which was largely
for testing.

OSC commands now broadcast back to any connecting clients. Right now these clients are never
forgotten, so it's possible to DDoS mtrack, so be responsible with your network security!

[0.6.0]

**Note**: You can now explicitly specify the sample rate, sample format, and bits per
sample for your audio device. This may be a breaking change depending on the audio files
you have been using. Please test before playing live!

Transcoding has been added to mtrack. This allows audio files and audio devices to have
mismatched sample rates and formats, making it easier to deal with files from multiple
sources. This also adds the ability to configure the target output format for an audio
device.

A new way of reading files has been added: SampleSources, which will hopefully allow us
to build different types of input sources in the future. This will make it easier to
introduce other input file types like FLAC, Ogg, MP3, etc. Along with this was a good
bit of performance tuning. The transcoding introduces increased performance requirements,
but this was reduced as much as possible.

Finally, audio is now played in a constant stream as opposed to opening up a new stream
per file. Combined with the transcoding work, this should make CPU usage very consistent.
In my testing, it seems that the biggest performance hit is from libasound's
speex_resampler_process. The rubato method is extremely efficient, so I recommend targeting
your audio device's native sample rate/format.

## [0.5.0]

Initialization of a songs directory is now easier, as mtrack can be given an `--init` flag
when using the `songs` subcommand. This will establish a baseline YAML file in each song
directory.

Live MIDI can be routed to the DMX engine. This will allow live controlling of lights through
mtrack.

The playlist can now be specified through the mtrack configuration YAML rather than specified
separately when using the start command.

Simple MIDI to DMX transformations are now supported. This allows for a simple MIDI event to
be expanded into multiple, which can then be fed into the DMX engine. This allows for things
like simple MIDI controllers to control multiple lights.

## [0.4.1]

Fix: Audio interfaces with spaces at the end can now be selected.

## [0.4.0] - Refactoring of config parsing, gRPC/OSC.

A gRPC server has been added to mtrack along with several utility subcommands that allow
for control of the player from the command line. This should be useful for creating
external player clients.

An OSC server has been added to mtrack. This will allow communication with mtrack over any
OSC protocol (UDP only at the moment). This is handy for using clients like TouchOSC to
control mtrack. This includes reporting, so that OSC clients can display information about
currently playing songs, track durations, etc.

The keyboard controller has been fixed -- it wasn't trimming off the newlines at the end
of keyboard input.

(Breaking Change) The `play` subcommand has been renamed `play-direct` so that the `play`
subcommand could be used to control the gRPC server.

(Breaking Change) mtrack no longer supports multiple song definitions in one file. This
is because mtrack has shed `serde_yaml`, which has been deprecated, and now uses `config-rs`
to parse config files, and config-rs does not support YAML documents in one file.
Other than lessening the maintenance burden, one advantage to doing this is that mtrack
can now support multiple file types. As of the time of writing, this includes:

- JSON
- TOML
- YAML
- INI
- RON
- JSON5

Note that I still personally test with YAML, so I haven't had an opportunity to exercise
all of the different file types.

## [0.3.0] - Configurable playback delays and refactoring.

Configurable playback delays have been added for audio, MIDI, and DMX playback.

DMX will now only use one OLA client instead of one per universe.

A fairly large refactor has been done to the config logic. The motivation is to
keep (most) of the instantiation of the various pieces of business logic out of
the config package while simultaneously trying to simplify the configuration of
the player and its various components.

Finally, the nodi dependency was updated. This is of interested as it has a corrected
timer, so mtrack has been updated to use it. Initial testing seems to indicate
that it works well.

## [0.2.1] - Repeatedly attempt to connect to OLA on startup.

Repeatedly attempt to connect to OLA, which should make connecting to DMX on startup
more reliable.

## [0.2.0] - Fix hard coded DMX universe.

The DMX universe was inadvertently hard coded to OLA universe 1. This
has been corrected.

## [0.1.9] - Better cancellation.

The expiration mechanism for cancellation had an unintended side effect
of preventing cancellation if one component of the song completed ahead
of time. In other words, if a MIDI file finished playing but there is
still audio to play, the song is no longer cancellable. Additionally, it
would be possible, in some circumstances, for the completion of one
aspect of a song to cancel others unexpectedly.

The "expiration" concept was introduced to allow cancellation while
still allowing a song to finish normally. This has been replaced with a
simple concept of an atomic bool that indicates whether a song component
(MIDI, DMX, or audio) has finished and, when used in combination with
the new "notify" function, will allow a cancel\_handle.wait() call to
return without an actual cancellation happening.

## [0.1.8] - Better MacOS support.

MacOS support is improved. It's not super thoroughly tested, but has been tested
against several audio interfaces.

`mtrack -V` will report the correct mtrack version.

## [0.1.7] - Fix dependencies.

Dependencies for mtrack have all been updated. This should hopefully resolve the issue
with `cargo install` not working properly.

## [0.1.6] - DMX engine with dimming support, MIDI channel exclusions.

DMX engine using OLA has been added. Contains a built in dimming engine.

Added the ability to exclude MIDI channels from MIDI playback.

## [0.1.5] - Initial DMX engine, fixed MIDI clock timing.

Initial DMX engine implemented. This is not quite ready for prime time.

Fixed the MIDI cancelable clock. Not sure what I was thinking when I implemented that.

## [0.1.4] - Track mapping update, status reporting, stopping fix.

(Breaking Change) Track mappings now support mapping to multiple channels.

Status reporting is now configurable.

Address stopping not working for songs with sparse MIDI files.

## [0.1.3] - MIDI tuning, accuracy.

MIDI playback is now more accurate and has been tuned to be more in time with audio
playback.

## [0.1.2] - Player level channel mapping, merging of channels.

Channel mappings have been removed from individual song files and will now be
maintained as part of the player configuration.

Channels can now be merged. That is, tracks can target the same output channel.

## [0.1.1] - Minor dependency update.

Removal of unneeded ringbuffer dependency.

## [0.1.0] - Initial release.

### Added

- Initial release.
