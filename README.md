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

## Features

- **Multi-format audio playback** — WAV, FLAC, MP3, OGG, AAC, ALAC, and more via Symphonia. Automatic
  transcoding to match your audio device.
- **MIDI playback and control** — Play back MIDI files, emit MIDI events on song selection, and control
  the player via MIDI. Beat clock output for syncing external gear.
- **Lighting engine** — Tag-based, venue-agnostic lighting system with a DSL for defining effects, cues,
  and sequences. Legacy MIDI-to-DMX conversion still supported.
- **MIDI-triggered samples** — Low-latency sample playback via MIDI or piezo audio triggers with velocity
  scaling, voice management, and release groups.
- **Web UI** — Browser-based interface for playback control, waveform visualization, stage view with
  real-time DMX state, and lighting simulation.
- **Terminal UI** — Optional ratatui-based TUI with playlist, now-playing, fixture colors, and log panel.
- **Hardware profiles** — Multiple host configurations in a single config file with hostname-based
  profile selection.
- **Remote control** — gRPC and OSC interfaces for external control and status reporting.
- **Systemd integration** — Generated service file with security hardening for production deployments.

## Quick Start

Install via cargo:

```
$ cargo install mtrack --locked
```

Discover your devices:

```
$ mtrack devices
$ mtrack midi-devices
```

Start the player:

```
$ mtrack start /path/to/player.yaml
```

The web UI will be available at `http://localhost:8080`.

## Documentation

For full documentation, see the [mtrack book](docs/src/SUMMARY.md).

Topics covered include:

- [Getting started](docs/src/getting-started/installation.md) — installation, device discovery, song
  setup, and player configuration
- [Interfaces](docs/src/interfaces/web-ui.md) — web UI, terminal UI, gRPC, and OSC control
- [Configuration](docs/src/configuration/hardware-profiles.md) — hardware profiles, samples, and triggers
- [Lighting](docs/src/lighting/overview.md) — lighting system overview, fixture configuration, effects,
  and cueing
- [Deployment](docs/src/deployment/systemd.md) — systemd setup and service hardening

## Building

mtrack uses a Makefile for build tasks. With [devbox](https://www.jetify.com/devbox) installed:

```
$ devbox shell
$ make build
```

Other useful targets: `make test`, `make lint`, `make fmt`, `make docs`, `make docs-serve`.

## License

This project is licensed under the [GNU General Public License v3.0](https://www.gnu.org/licenses/gpl-3.0).
