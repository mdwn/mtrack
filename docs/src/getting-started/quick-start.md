# Quick Start

This page takes you from an installed `mtrack` to a song playing out of your audio interface,
using only the web UI. It assumes you have [installed mtrack](installation.md), have an audio
interface connected, and have a folder of audio files for one song (for example a mono
`click.wav` and a stereo `backing.wav`).

## 1. Start mtrack

```
# Start in a directory that will hold your project (an empty directory is fine)
cd /path/to/my/project
mtrack start
```

mtrack creates an `mtrack.yaml` configuration file in that directory if one does not exist,
and uses the directory as the project root. Songs, playlists, and lighting files all live
under it.

> **Note:** mtrack needs **write access** to the project directory in order to manage
> configuration, songs, playlists, and lighting files through the web UI. If the directory
> is read-only, playback works but editing features are disabled.

On the Raspberry Pi image, mtrack is already running and its project directory is
`/var/lib/mtrack`.

## 2. Open the web UI

Open the address that matches where mtrack runs:

- Same computer: **<http://localhost:8080>**
- Another computer or a phone: `http://<pi-ip>:8080`, using the Pi's IP address
- Raspberry Pi image: `http://mtrack.local:8080` (if the name does not resolve, use the IP
  address)

## 3. Unlock the web UI

mtrack starts **locked**. While locked, saving configuration, importing songs, and uploading
files are disabled; playback controls still work. An amber **LIVE — locked** stripe under the
top nav tells you the state.

Click the lock icon in the top nav and confirm the "Unlock during a live session?" prompt.
The stripe disappears. See [Lock Mode](../interfaces/web-ui.md#lock-mode).

## 4. Add a hardware profile

mtrack does not touch any hardware until a profile describes it.

1. Open **Config** in the nav bar.
2. Click **Add Profile**. If mtrack asks for a profile filename, enter any name. Leave the
   **Hostname** field empty so the profile applies on any machine.
3. On the **Audio** tab, click **Enable Audio**, then choose your interface in **Device**. If it
   does not appear in the list, click **Refresh**; see [Discovering Devices](devices.md).
4. Click **Test**. The test opens the device and checks that it is streaming; it plays no
   sound, so it is safe with the PA on.
5. Click **Save**.

## 5. Import a song

1. Copy your song's folder into the project directory. Name the files after the tracks you
   want, for example `click.wav` (mono) and `backing.wav` (stereo).
2. Open **Songs** and click **Import from Filesystem**.
3. Navigate to the folder and click **Use This Directory**, then **Create Song**.

mtrack names each track after its file, and splits a stereo file into `backing-l` and
`backing-r`. See [Importing Songs](importing-songs.md) for the details, including the special
role of a track named `click`.

## 6. Send the tracks to your outputs

A track is silent until the profile says which output channels it plays on.

1. Back on **Config**, open your profile and go to the **Audio** tab.
2. Under **Track Mappings**, click **Add** and enter the track name and channel numbers:
   `click` to `1`, `backing-l` to `3`, and `backing-r` to `4` (or whichever outputs feed your
   in-ears and the PA).
3. Click **Save**. mtrack reloads the hardware with the new mappings.

[Hardware Configuration](hardware-config.md#track-mappings) explains track mappings in full.

## 7. Play

Open the dashboard, make sure your song is the selected one, and press **Play**.

You should now hear the click on output 1 and the backing track on outputs 3 and 4, and the
playhead on the dashboard should advance. If it is silent, check that the track names in the
mappings match the track names on the song's detail page exactly, and that the **Status**
page shows the audio device as connected.

## Where to go next

The nav bar links to:

- **Songs** — Browse, create, import, and edit songs. See [Importing Songs](importing-songs.md).
- **Playlists** — Create and manage setlists. See [Playlists](playlists.md).
- **Lighting** — Fixtures, venues, groups, and readiness checks for light shows. Skip this
  unless you run lights; if you do, start with [First Light](../lighting/first-light.md).
- **Config** — Audio, MIDI, lighting, and controller settings. See
  [Hardware Configuration](hardware-config.md).
- **Status** — Connected devices, controller status, and system health.

Before a show, lock the web UI again from the same lock icon so a stray tap cannot change
your configuration.
