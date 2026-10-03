# Hardware Configuration

The web UI's **Config** page lets you configure all of mtrack's hardware settings without
editing YAML files directly.

## Profiles

Hardware profiles let you define per-machine configurations that auto-select based on hostname.
This is useful when you use different audio interfaces at rehearsal and at shows — each machine
picks up the right profile automatically.

To create a profile, click **Add Profile** on the Config page. If mtrack asks for a profile
filename, enter one. Then set the **Hostname** field:

- A profile with a **Hostname** applies only on a machine whose hostname equals it exactly.
- A profile with the field left empty applies on any machine.
- When several profiles match, the first one is used.
- When no profile matches, mtrack starts with no audio, MIDI, or DMX hardware. Playback
  controls and the web UI still work, but there is no sound output until a profile matches.

mtrack reads the machine's hostname from the operating system. Set the `MTRACK_HOSTNAME`
environment variable to override it, for example to test a profile on another machine or when
the OS hostname differs from the name you want to match.

## Configuring a Profile

Click a profile to open its settings. Configuration is organized into tabs:

- **Audio** — Select your audio device, set sample rate, buffer size, bit depth, and playback
  delay. The device list is populated from connected hardware.
- **MIDI** — Select your MIDI device, configure playback delay, beat clock output, and
  MIDI-to-DMX passthrough mappings with transformer editors.
- **Lighting** — Configure DMX universes, map them to olad universe numbers, and set the olad port (`ola_port`). Fixtures,
  venues and groups are set up in the [Lighting area](../interfaces/web-ui.md#lighting); see
  [First Light](../lighting/first-light.md).
- **Triggers** — Set up audio and MIDI-triggered sample playback.
- **Controllers** — Configure gRPC, OSC, and MIDI control interfaces. MIDI controllers support
  full event mapping plus optional Morningstar preset naming and section loop control events.
- **Status Events** — Configure MIDI events emitted on player state changes (off/idling/playing)
  for driving controller LEDs.
- **Notifications** — Configure custom audio files for loop and section transition events,
  with per-section-name overrides.

Enable a section by toggling it on, then fill in the settings. Tooltips on each field explain
what it does. A dirty indicator (`*` in yellow) appears in the profile editor title when there
are unsaved changes.

## Track Mappings

A **track** is one channel of audio in a song: a mono file is one track, a stereo file
imported through the web UI becomes two (`name-l` and `name-r`), and each track has a name.
Track mappings route those named tracks to output channels on your audio interface. They are
configured under **Track Mappings** on the **Audio** tab of a profile: click **Add**, enter the
track name, and enter the output channel numbers, separated by commas. Channels are numbered
from 1.

For example, on an interface with four outputs where outputs 1 and 2 go to the in-ear mix and
outputs 3 and 4 go to the PA:

| Track name  | Channels |
|-------------|----------|
| `click`     | `1`      |
| `backing-l` | `3`      |
| `backing-r` | `4`      |

which is stored in the profile as:

```yaml
audio:
  device: UltraLite-mk5
  track_mappings:
    click: [1]
    backing-l: [3]
    backing-r: [4]
```

A track can list more than one channel (`click: [1, 2]` plays it on both outputs).

Two rules matter:

- **Names must match exactly.** A mapping applies to the song tracks whose names equal the
  mapping key. See [Importing Songs](importing-songs.md#naming-your-tracks) for how track names
  are chosen.
- **Unmapped tracks are silent.** A song track with no entry in the profile's track mappings
  is not sent to any output, and a mapping whose name matches no track in the song does
  nothing. If a song plays silently, compare the song's track names against the mapping names
  first.

mtrack reloads the hardware when you save a profile; this is refused while a song is playing.
Before changing mappings, find your interface's name and channel count with
[Discovering Devices](devices.md).

## How Configuration Is Stored

The Config page writes to `mtrack.yaml` in your project root. For full details on the YAML
format, see [Player Configuration (YAML)](../configuration/player-config.md).
