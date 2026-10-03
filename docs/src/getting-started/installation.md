# Installation

## Pre-built binaries (recommended)

Download a binary from the
[latest release](https://github.com/mdwn/mtrack/releases/latest) for your
platform — Linux (x86_64, aarch64) or macOS (Intel, Apple Silicon) — extract
it, and put `mtrack` somewhere on your `PATH`:

```
$ tar xzf mtrack-<version>-<target>.tar.gz
$ sudo cp mtrack-<version>-<target>/mtrack /usr/local/bin/mtrack
```

On Linux, the binary needs `libasound2` and `libudev1` at runtime. These are
present by default on most desktop distros; on a minimal server:

```
$ sudo apt install libasound2 libudev1
```

If you use [cargo-binstall](https://github.com/cargo-bins/cargo-binstall), it
will fetch the same release binaries:

```
$ cargo binstall mtrack
```

## Raspberry Pi image

For a Raspberry Pi, the least work is a pre-built image: Raspberry Pi OS Lite
with mtrack installed, running on boot, and reachable at
`http://mtrack.local:8080`, with `avahi` for that name and `olad` for DMX.
Download `mtrack-<version>-raspberrypi-arm64.img.xz` from the
[latest release](https://github.com/mdwn/mtrack/releases/latest) and flash it
with Raspberry Pi Imager, filling in Imager's customisation dialog: the image
ships no default username or password, so that dialog is what makes the Pi
reachable without a keyboard and a monitor.

[Raspberry Pi Image](raspberry-pi.md) walks through the flash, the first boot,
finding the Pi on the network, loading songs by upload, patching `olad`, and
updating. The image is 64-bit, so it needs a Pi 3 or newer; on anything older,
or on 32-bit Raspberry Pi OS, install from source with cargo.

## Debian, Ubuntu and Raspberry Pi OS packages

On a Debian-derived system a `.deb` is the least fiddly option, and the one to
prefer on a Raspberry Pi. It installs the binary, creates the `mtrack` service
account, adds it to the `audio` group, and generates the systemd unit — the
whole of [Running on Startup](../deployment/systemd.md) happens for you.

Download the package matching your architecture (`arm64` for 64-bit Raspberry
Pi OS, `amd64` for a PC) from the
[latest release](https://github.com/mdwn/mtrack/releases/latest), then:

```
$ sudo apt install ./mtrack_<version>_arm64.deb
```

Installing it through `apt` rather than `dpkg -i` matters: it pulls in
`libasound2` and `libudev1` for you. It also installs `ola` — the Open Lighting
daemon mtrack talks to for DMX output — because the package recommends it. If you
do not run lights, skip it with `--no-install-recommends`; if you skipped it and
later want lights, `sudo apt install ola` adds it.

The service is enabled and started automatically, with its library at
`/var/lib/mtrack`. Point it somewhere else — an SD card or USB drive — by
editing `/etc/default/mtrack` and regenerating the unit, which that file
explains inline.

What the package deliberately does *not* do is delete your library. `apt purge
mtrack` leaves `/var/lib/mtrack` and the `mtrack` user alone, because a purge
should not take a band's set with it.

### Upgrading

`apt upgrade` replaces the binary and restarts the service. It also re-renders
the systemd unit against the new binary, so the "regenerate your unit" step in
[Running on Startup](../deployment/systemd.md) is handled — unless you have
edited the unit yourself, in which case your version is kept and the upgrade
tells you what it would have written.

## From source with cargo

Building from source requires a Rust toolchain plus a few system packages:
the protobuf compiler and, on Linux, ALSA and udev development headers.

On Debian/Ubuntu:

```
$ sudo apt install libasound2-dev libudev-dev pkg-config libssl-dev protobuf-compiler
```

On macOS:

```
$ brew install pkg-config protobuf
```

Then:

```
$ cargo install mtrack --locked
```

To run `mtrack` on startup, copy it to `/usr/local/bin`:

```
$ sudo cp ~/.cargo/bin/mtrack /usr/local/bin/mtrack
```
