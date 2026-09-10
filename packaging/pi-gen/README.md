# Raspberry Pi image

A flashable Raspberry Pi OS Lite image with mtrack installed and running on
boot, built as a [pi-gen](https://github.com/RPi-Distro/pi-gen) stage.

## What the image is

Raspberry Pi OS Lite (arm64), plus:

| Piece | Why |
| --- | --- |
| the mtrack `.deb` | The service account, the systemd unit and the project directory at `/var/lib/mtrack` — see `packaging/README.md`. |
| `avahi-daemon` | `http://mtrack.local:8080` on a machine with no monitor. |
| `ola` | The Open Lighting daemon mtrack talks to for DMX output. |
| hostname `mtrack` | What avahi answers to. |

The image ships **no default username or password**. pi-gen leaves the account
locked and runs the setup wizard on first boot, so an image that leaks onto the
internet is not a device anyone can log into.

The headless path is Raspberry Pi Imager's customisation dialog, which writes
the username, password, wifi credentials, wifi country and ssh keys onto the
boot partition before the card is ever booted. **Flash it with Imager and fill
that in**, or the first boot will want a keyboard and a monitor.

## Why the stage enables things the package does not

The package's `postinst` only enables the service when it is installed under a
booted systemd — it checks for `/run/systemd/system`, which is how it tells a
real machine from a chroot, a container or an image build. An image build is
exactly the case it skips, so a stage that only installed the package would
produce an image where mtrack is present and nothing is running. The stage
enables `mtrack.service` itself, and this is the reason.

`olad` is the opposite case and is deliberately left alone. The `ola` package
ships a SysV init script and no systemd unit, so `systemctl enable olad.service`
has nothing to act on at build time and would fail the build. Its `postinst`
has already run `update-rc.d`, and systemd's sysv generator synthesises the unit
at boot.

## Building

CI builds this on every published release (`.github/workflows/pi-image.yaml`),
on a native arm64 runner so pi-gen's chroot needs no emulation.

It also builds on pull requests that touch `packaging/`, `src/cli.rs` or the
workflow itself, and attaches the image as a run artifact. Those three paths are
the ones that can break an image without touching the stage: the stage leans on
the package's `postinst` behaviour, and `src/cli.rs` holds the unit template.

A pull request has no release to draw a package from, and taking the last
release's would test the previous version rather than the change in hand, so a
pull request builds the package it is about to install. Only a pull request
does: a run that uploads to a release refuses to proceed unless the package came
from that release, so an image on the downloads page always carries a binary you
can trace back to a published artifact. To build by
hand you want an arm64 machine for the same reason; on x86 you additionally need
`qemu-user-static` and binfmt registration, which pi-gen's README covers.

```
$ git clone https://github.com/RPi-Distro/pi-gen
$ cd pi-gen
$ cp /path/to/mtrack/packaging/pi-gen/config ./config
$ touch ./stage3/SKIP ./stage4/SKIP ./stage5/SKIP
$ touch ./stage2/SKIP_IMAGES ./stage4/SKIP_IMAGES ./stage5/SKIP_IMAGES
$ export MTRACK_STAGE_DIR=/path/to/mtrack/packaging/pi-gen/stage-mtrack
$ export MTRACK_DEB=/path/to/mtrack_0.16.0-1_arm64.deb
$ sudo --preserve-env=MTRACK_DEB,MTRACK_STAGE_DIR ./build.sh
```

The image lands in `deploy/`. Expect tens of minutes and roughly 10 GB of
working space — pi-gen keeps a full rootfs per stage.

`MTRACK_DEB` is passed in rather than built here, so the image carries the exact
binary the release published. `STAGE_LIST` in the config points at
`MTRACK_STAGE_DIR` by absolute path; pi-gen supports stages outside its own
tree, so the stage lives in this repo and is never copied into a pi-gen
checkout.

### Targeting an older Raspberry Pi OS

`RELEASE` defaults to `trixie`. pi-gen's package sets differ per release, so
building an older one means checking out the matching pi-gen branch too:

```
$ git checkout bookworm      # in the pi-gen clone
$ RELEASE=bookworm sudo --preserve-env=... ./build.sh
```

The package installs on either. Its dependencies name
`libasound2t64 | libasound2` precisely so that the same `.deb` resolves on both
sides of the time_t rename — verified on both.

## What this does not do yet

- **32-bit (`armhf`)** is not covered; mtrack publishes arm64 and amd64 only.
- **The library lives on the SD card** at `/var/lib/mtrack`. Moving it to a USB
  drive means editing `/etc/default/mtrack` and regenerating the unit, which
  that file explains inline. Auto-mounting a USB library is not wired up.
- **No wifi hotspot fallback.** A venue with no network the Pi already knows
  means connecting by ethernet or reflashing with new Imager settings.
- **No apt archive.** Updates mean flashing a new image or installing a newer
  `.deb` by hand until the archive in `packaging/README.md` is live; once it is,
  this stage should also drop in the sources file and keyring so `apt upgrade`
  works on a running Pi.
