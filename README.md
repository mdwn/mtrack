# mtrack

[![Actions Status](https://github.com/mdwn/mtrack/actions/workflows/mtrack.yaml/badge.svg)](https://github.com/mdwn/mtrack/actions)
[![codecov](https://codecov.io/gh/mdwn/mtrack/graph/badge.svg?token=XWEK2BIPZL)](https://codecov.io/gh/mdwn/mtrack)
[![Crates.io Version](https://img.shields.io/crates/v/mtrack)](https://crates.io/crates/mtrack)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-2.1-4baaaa.svg)](code_of_conduct.md)

`mtrack` is a multitrack audio, MIDI, and lighting player for live performances. It runs on
small devices like the Raspberry Pi and is designed to be controlled remotely — from your feet,
a phone, or any device with a browser — so you never have to babysit a computer on stage.

![Dashboard](docs/src/images/dashboard.png)

## Features

- **Multi-format audio playback** — WAV, FLAC, MP3, OGG, AAC, M4A, AIFF via Symphonia.
  Automatic transcoding to match your audio device.
- **MIDI playback and control** — Play back MIDI files, emit MIDI events on song selection, and
  control the player via MIDI. Beat clock output for syncing external gear.
- **DMX lighting engine** — Tag-based, venue-agnostic lighting system with a custom DSL for
  effects, cues, and sequences. Real-time effects engine with OLA integration for DMX output.
- **Web UI** — Full browser-based interface for playback control, song management, lighting
  show authoring, playlist editing, and hardware configuration. Includes a DAW-style timeline
  editor with integrated playback preview and real-time stage visualization.
- **Zero-config startup** — Point mtrack at a directory of songs and it works. No config file
  required. Bulk import entire song libraries from the filesystem.
- **Lock mode** — Safety mechanism for live shows. Lock the player to prevent accidental
  configuration changes while keeping playback controls active.
- **MIDI-triggered samples** — Low-latency sample playback via MIDI or piezo audio triggers
  with velocity scaling, voice management, and release groups.
- **Terminal UI** — Optional ratatui-based TUI with playlist, now-playing, fixture colors,
  and log panel.
- **Hardware profiles** — Per-machine hardware configurations with hostname-based profile
  selection. Carry the same config across rehearsal and show rigs.
- **Remote control** — gRPC and OSC interfaces for external control and status reporting.
- **Systemd integration** — Generated service file with security hardening for production
  deployments.

## Quick Start

Install via cargo:

```
$ cargo install mtrack --locked
```

Start the player (zero-config):

```
$ cd /path/to/my/songs
$ mtrack start
```

Or point at a specific directory:

```
$ mtrack start /path/to/my/songs
```

Existing config files still work:

```
$ mtrack start /path/to/mtrack.yaml
```

The web UI will be available at `http://localhost:8080`. Use the web UI to import songs,
create playlists, configure hardware, and author lighting shows.

![Timeline editor](docs/src/images/timeline-editor.png)

## Documentation

For full documentation, see the [mtrack book](docs/src/SUMMARY.md).

Topics covered include:

- [Getting started](docs/src/getting-started/installation.md) — installation, device discovery,
  song setup, and player configuration
- [Web UI](docs/src/interfaces/web-ui.md) — dashboard, song browser, timeline editor, playlist
  editor, configuration, and lock mode
- [Interfaces](docs/src/interfaces/tui.md) — terminal UI, gRPC, and OSC control
- [Configuration](docs/src/configuration/hardware-profiles.md) — hardware profiles, samples,
  and triggers
- [Lighting](docs/src/lighting/overview.md) — lighting system overview, fixture configuration,
  effects, and cueing
- [Deployment](docs/src/deployment/systemd.md) — systemd setup and service hardening

## Building

mtrack uses a Makefile for build tasks. First, install system dependencies:

```
$ ./setup.sh          # build dependencies only
$ ./setup.sh --dev    # include development tools (buf, cargo-tarpaulin, licensure, mdbook)
```

Then build:

```
$ make build
```

Other useful targets: `make test`, `make lint`, `make fmt`, `make docs`, `make docs-serve`.

## License

This project is licensed under the [GNU General Public License v3.0](https://www.gnu.org/licenses/gpl-3.0).
