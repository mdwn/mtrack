# Raspberry Pi Image

This page walks a Raspberry Pi from an empty card to a song playing, using the pre-built
image: Raspberry Pi OS Lite with mtrack installed and running on boot, reachable at
`http://mtrack.local:8080`. Nothing on this page needs a keyboard or a monitor on the Pi.

## What you need

- A Raspberry Pi 3 or newer, or a Pi Zero 2 W (the image is 64-bit), with its power supply.
- A microSD card, 8 GB or larger, and a way to write it from your computer.
- A network the Pi and your computer share: wifi, or an Ethernet cable to the same router.
- A USB audio interface. For lights, a USB DMX interface that `olad` supports.
- [Raspberry Pi Imager](https://www.raspberrypi.com/software/) on your computer.
- `mtrack-<version>-raspberrypi-arm64.img.xz` from the
  [latest release](https://github.com/mdwn/mtrack/releases/latest). Imager reads the `.xz`
  directly; do not unpack it.

## 1. Flash the card

1. Start Imager. Choose your Pi model, then for the operating system scroll to **Use custom**
   and pick the downloaded `.img.xz`. Choose the card.
2. When Imager asks whether to apply **OS customisation**, choose to edit the settings. The
   image ships **no default username or password**; this dialog is where the Pi gets them.
   Fill in:
   - a **username and password** — your login over ssh, and nothing else on the Pi;
   - **wifi** network, password and country, unless the Pi will be on Ethernet;
   - under services, **enable ssh** (password or your public key).
   - A **hostname** is optional. The image's own is `mtrack`, which is what makes
     `mtrack.local` work; if you set one, the address becomes `http://<hostname>.local:8080`.
3. Save, confirm, and let Imager write and verify the card.

If you skip the customisation, the first boot runs Raspberry Pi OS's setup wizard on the Pi's
own console and waits there, so the Pi is not reachable until someone with a keyboard and a
monitor finishes it.

## 2. First boot

Put the card in, connect the audio interface and any DMX interface, connect Ethernet if you
are using it, and power the Pi. The first boot grows the filesystem to the card and applies
your settings; give it two or three minutes.

Then, from a computer on the same network, open **<http://mtrack.local:8080>**. You should see
mtrack's web UI, locked, with no profile and no songs.

If the name does not resolve: `mtrack.local` is found by multicast, which reaches the devices
on the same network segment as the Pi and no further. A Pi on wifi is found by other wifi
devices, and by wired ones only if your router passes multicast between the two. Find the
Pi's IP address instead — your router's list of connected devices usually shows it under the
name `mtrack` — and use `http://<ip>:8080`. The address always works, with or without the name.

If nothing answers on either, the Pi is probably not on the network: check the wifi password
and country in Imager, or try an Ethernet cable.

## 3. Set up the player

The Pi is now an ordinary mtrack install, and [Quick Start](quick-start.md) takes it from here:
unlock the web UI, add a hardware profile, pick the audio interface, import a song, map its
tracks to outputs, play. Two things are different on a Pi:

- **Songs are uploaded, not copied.** Your audio files are on your computer and the project is
  on the card. Click **New Song** and name it; on the song's **Tracks** tab, drop the audio
  files on **Drop audio files here or click to upload**, then **+ Add Track** for each part:
  a name (`click`, `backing-l`), the file from the list, and for a stereo file the **Channel**
  (one track for channel 1, another for channel 2). Save. There is no need to log in to the Pi
  to add songs. ([Importing Songs](importing-songs.md) has the details, including the role of a
  track named `click`.)
- **The project is `/var/lib/mtrack` on the card**: `mtrack.yaml`, the songs you upload, your
  profiles, playlists, and the `lighting/` directory with imported GDTFs, venues and fixture
  records. The web UI writes there, so everything you set up in the browser is on the card. To
  keep it on a USB drive instead, edit `/etc/default/mtrack` and regenerate the unit; that file
  explains how inline.

## 4. Lights

The image carries `olad`, the Open Lighting daemon, started on boot. It starts with **no
universe patched to any output**, and frames sent to an unpatched universe go nowhere. Open
`http://mtrack.local:9090`, patch your DMX interface's port to the universe number your profile
names, and the Lighting Overview's output check turns green. mtrack warns when a universe it
drives has no port patched. From there,
[First Light](../lighting/first-light.md) goes from a fixture's GDTF to a lit show.

## Logging in

You only need ssh for things the web UI does not do: moving the project to a USB drive,
reading logs, or installing an update.

```
ssh <username>@mtrack.local
sudo journalctl -u mtrack -f       # the player's log
sudo systemctl status mtrack       # is it running
```

mtrack runs as its own `mtrack` user inside a systemd sandbox that may write only to the
project directory; see [Service Hardening](../deployment/security.md). Files you copy into
`/var/lib/mtrack` by hand need to be readable by that user (`sudo chown -R mtrack:mtrack`).

## Updating

A newer mtrack is installed as a package, without reflashing. Download
`mtrack_<version>_arm64.deb` from the release and, on the Pi:

```
sudo apt install ./mtrack_<version>_arm64.deb
```

The service restarts on the new version and the project directory is untouched. See
[Upgrading](installation.md#upgrading) for what the package does and does not do.
