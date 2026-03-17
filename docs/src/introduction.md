# Introduction

[![Actions Status](https://github.com/mdwn/mtrack/actions/workflows/mtrack.yaml/badge.svg)](https://github.com/mdwn/mtrack/actions)
[![codecov](https://codecov.io/gh/mdwn/mtrack/graph/badge.svg?token=XWEK2BIPZL)](https://codecov.io/gh/mdwn/mtrack)
[![Crates.io Version](https://img.shields.io/crates/v/mtrack)](https://crates.io/crates/mtrack)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-2.1-4baaaa.svg)](https://github.com/mdwn/mtrack/blob/main/CODE_OF_CONDUCT.md)

`mtrack` is a multitrack audio, MIDI, and lighting player for live performances. It runs on
small devices like the Raspberry Pi and is designed to be controlled remotely — from your feet,
a phone, or any device with a browser — so you never have to babysit a computer on stage.

![Dashboard](images/dashboard.png)

## Features

- **Multitrack audio** — Play back multiple audio files simultaneously, mapping channels to
  any class-compliant audio interface. Supports WAV, FLAC, MP3, OGG, AAC, M4A, and AIFF.
- **MIDI playback** — Synchronize MIDI file playback with audio for automating on-stage gear.
- **DMX lighting** — Programmable lighting effects with a custom DSL, real-time effects engine,
  and OLA integration for DMX output.
- **Web UI** — Full browser-based interface for playback control, song management, lighting
  show editing, playlist management, and hardware configuration. Includes a DAW-style timeline
  editor with integrated playback preview.
- **Zero-config startup** — Point mtrack at a directory of songs and it works. No config file
  required.
- **Lock mode** — Safety mechanism for live shows. Lock the player to prevent accidental
  configuration changes while keeping playback controls active.
- **Multiple control interfaces** — Web UI, gRPC, OSC, and MIDI control. Use foot controllers,
  tablets, or custom software to drive playback.
- **Hardware profiles** — Define per-machine hardware configurations that auto-select based on
  hostname. Carry the same config across rehearsal and show rigs.
- **Triggered samples** — Audio and MIDI-triggered sample playback with velocity curves,
  release groups, and voice management.

## Quick Start

```
# Install
cargo install mtrack

# Start with a directory of songs
cd /path/to/my/songs
mtrack start

# Or point at a specific directory
mtrack start /path/to/my/songs
```

Open `http://localhost:8080` in a browser to access the web UI.

## How It Works

1. **mtrack starts** on the first song in the active playlist, selected but not playing.
2. **Navigate** songs using next/previous controls (web UI, MIDI, OSC, or gRPC).
3. **Play** the selected song. Audio, MIDI, and lighting play back in sync.
4. **Stop** at any time. When a song finishes naturally, the playlist advances to the next song.
5. **Switch playlists** to access different setlists, or use the all-songs list to find any song.

![Timeline editor](images/timeline-editor.png)

## Documentation

See the full documentation for:

- [Installation](getting-started/installation.md)
- [Song Repository](getting-started/song-repository.md)
- [Playlists](getting-started/playlists.md)
- [Web UI](interfaces/web-ui.md)
- [Lighting](lighting/overview.md)
- [Hardware Profiles](configuration/hardware-profiles.md)
- [gRPC Control](interfaces/grpc.md)
- [OSC Control](interfaces/osc.md)
