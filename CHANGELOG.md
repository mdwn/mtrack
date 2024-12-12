# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
