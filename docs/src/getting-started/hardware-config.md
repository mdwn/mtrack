# Hardware Configuration

The web UI's **Config** page lets you configure all of mtrack's hardware settings without
editing YAML files directly.

## Profiles

Hardware profiles let you define per-machine configurations that auto-select based on hostname.
This is useful when you use different audio interfaces at rehearsal and at shows — each machine
picks up the right profile automatically.

To create a profile, click **New Profile** on the Config page and enter a name (typically your
machine's hostname).

## Configuring a Profile

Click a profile to open its settings. Configuration is organized into tabs:

- **Audio** — Select your audio device, set sample rate, buffer size, bit depth, and playback
  delay. The device list is populated from connected hardware.
- **MIDI** — Select your MIDI device, configure playback delay and beat clock output.
- **Lighting** — Configure DMX universes and map them to OLA universe numbers.
- **Triggers** — Set up audio and MIDI-triggered sample playback.
- **Controllers** — Configure gRPC, OSC, and MIDI control interfaces.

Enable a section by toggling it on, then fill in the settings. Tooltips on each field explain
what it does.

## Discovering Devices

The **Status** page shows currently connected audio and MIDI devices. You can also use CLI
commands — see [Discovering Devices](devices.md).

## Track Mappings

Track mappings route named tracks (e.g. "click", "backing-l") to specific output channels
on your audio interface. Configure these in the Audio section of your profile.

## How Configuration Is Stored

The Config page writes to `mtrack.yaml` in your project root. For full details on the YAML
format, see [Player Configuration (YAML)](../configuration/player-config.md).
