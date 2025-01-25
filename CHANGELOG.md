# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Configurable playback delays have been added for audio, MIDI, and DMX playback.

A fairly large refactor has been done to the config logic. The motivation is to
keep (most) of the instantiation of the various pieces of business logic out of
the config package while simultaneously trying to simplify the configuration of
the player and its various components.

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
