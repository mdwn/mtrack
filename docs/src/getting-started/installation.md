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
`http://mtrack.local:8080`. It also carries `avahi` for that name to resolve and
`olad` for DMX output.

Download `mtrack-<version>-raspberrypi-arm64.img.xz` from the
[latest release](https://github.com/mdwn/mtrack/releases/latest) and flash it
with [Raspberry Pi Imager](https://www.raspberrypi.com/software/).

**Use Imager's customisation dialog.** The image ships no default username or
password, so the settings you enter there — username, password, wifi and its
country, ssh keys — are what make the Pi reachable. Skip it and the first boot
wants a keyboard and a monitor to run the setup wizard.

`mtrack.local` is found by multicast, which reaches the devices on the same
network segment as the Pi and no further: a Pi on wifi is found by other wifi
devices, and by wired ones only if your router passes multicast between the
two. If the name does not resolve, the Pi's IP address always works. A
hostname set in Imager replaces `mtrack` in that name.

DMX goes through `olad`, which starts with no universe patched to an output:
open `http://<the Pi>:9090` and patch your DMX interface to the universe your
profile names, or frames go nowhere (mtrack warns when a universe it drives has
no port patched; see
[The universe must exist in olad](../lighting/configuration.md#the-universe-must-exist-in-olad)).

`/var/lib/mtrack` on the card is the project directory: `mtrack.yaml`, your
songs, and the `lighting/` directory that holds imported GDTFs
(`lighting/library/`), venues and fixture type records. The web UI writes there,
so everything you set up in the browser is on the card. To keep it on a USB
drive instead, edit `/etc/default/mtrack` and regenerate the unit; that file
explains how inline.

With the Pi up, [Quick Start](quick-start.md) takes it from the web UI, and
[First Light](../lighting/first-light.md) from a fixture's GDTF to a lit show.

The image is 64-bit, so it needs a Pi 3 or newer. On anything older, or on
32-bit Raspberry Pi OS, install from source with cargo.

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

If you want to use `mtrack` on startup, I recommend copying it to
`/usr/local/bin`:

```
$ sudo cp ~/.cargo/bin/mtrack /usr/local/bin/mtrack
```
