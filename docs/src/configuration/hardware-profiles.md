# Hardware Profiles

A hardware profile is one complete host configuration: the audio device and its track
mappings, the MIDI device, DMX universes, trigger inputs, controllers, status events and
notification sounds. `mtrack.yaml` can carry several, so one project directory can be shared by
several hosts; each host picks the profile that matches it.

## Choosing a profile

Profiles are matched by **hostname**. A profile with a `hostname` applies only on the host
whose hostname equals it; a profile without one matches any host. Profiles are tried in order
(file order in `profiles_dir`, list order inline) and the first match is used, so put
host-specific profiles before a catch-all.

mtrack reads the hostname from the `MTRACK_HOSTNAME` environment variable when it is set and
non-empty, and from the operating system otherwise. Set the variable to test another host's
profile, or when the OS hostname differs from the one you want to match on.

When no profile matches, mtrack starts with no hardware: no audio, MIDI or DMX is opened, and the
web UI runs so you can add a profile from its Config page. `mtrack verify <config> --hostname
<name>` checks the profiles a given hostname would select and warns when none match.

Inline, the profiles are a list under `profiles:`:

```yaml
profiles:
  # Raspberry Pi A: Full setup with WING audio + MIDI + DMX
  - hostname: raspberry-pi-a
    audio:
      device: "Behringer WING"
      sample_rate: 48000
      sample_format: int
      bits_per_sample: 32
      buffer_size: 1024
      playback_delay: 500ms
      track_mappings:
        click: [1]
        cue: [2]
        backing-track-l: [3]
        backing-track-r: [4]
        keys: [5, 6]
      # Optional per-track gain in dB. Tracks without an entry play at unity.
      track_gains:
        click: -6.0
        keys: 2.5
    midi:
      device: "Behringer WING"
      playback_delay: 500ms
      midi_to_dmx:
        - midi_channel: 15
          universe: light-show
    dmx:
      dim_speed_modifier: 0.25
      universes:
        - universe: 1
          name: light-show

  # Raspberry Pi B: WING with different channels, MIDI required, no DMX
  - hostname: raspberry-pi-b
    audio:
      device: "Behringer WING"
      sample_rate: 48000
      track_mappings:
        click: [11]
        cue: [12]
        backing-track-l: [13]
        backing-track-r: [14]
        keys: [15, 16]
    midi:
      device: "USB MIDI Interface"
      playback_delay: 200ms
    # dmx omitted = not used on this host

  # Lighting-only node: DMX only, no audio or MIDI
  - hostname: lighting-node
    dmx:
      universes:
        - universe: 1
          name: light-show

  # Fallback: minimal audio setup for any host (no MIDI/DMX)
  - audio:
      device: "Built-in Audio"
      track_mappings:
        click: [1]
        backing-track-l: [1]
        backing-track-r: [2]
```

**Subsystem semantics:**
- All three subsystems (**Audio**, **MIDI**, **DMX**) are optional:
  - If present in a profile → required for that host (player waits/retries until device is found)
  - If absent from a profile → skipped for that host (player proceeds without it)
- A profile can define any combination of subsystems, enabling dedicated roles such as
  lighting-only nodes, MIDI-only controllers, or full audio + MIDI + DMX setups.

Every key a profile accepts, with its default, is listed under
[Profile keys](player-config.md#profile-keys) in the player configuration reference.

**Per-track gain (`track_gains`):** an optional map of output-track names (the keys of
`track_mappings`) to gain in dB, from -60 to +12 (values at or below -60 mute the track;
tracks without an entry play at unity). Gains are adjustable live from the web UI Tracks
card, gRPC (`SetTrackGain` / `GetTrackGains`), and OSC (`/mtrack/track/*/gain`). Runtime
changes are written back to the owning profile after a short debounce, so they survive
restarts.

### Reserved track names

Three track names in `track_mappings` route audio that mtrack generates rather than a file
from `song.yaml`:

| Name | Routes |
|------|--------|
| `metronome` | the generated click of songs with a `metronome:` block (the song can rename it with `metronome.track`) |
| `pilot` | pilot hint samples (renamed with `pilot.track`) |
| `mtrack:looping` | the [notification sounds](#notification-audio) for section looping |

A song refuses to load when one of its audio tracks shares a name with its metronome or pilot
track, or when the two virtual tracks share one. Without a mapping a virtual track is silent.
The web UI's track-mapping editor offers `mtrack:looping` alongside the track names found in
your songs.

## External Profiles Directory

Instead of defining profiles inline, you can load them from individual YAML files in a
directory. Each file defines one profile using the same keys as an inline entry, plus
`kind: hardware_profile`.

```yaml
# mtrack.yaml
# Load profiles from a directory (path relative to this config file)
profiles_dir: profiles/
```

```yaml
# profiles/01-pi-a.yaml
kind: hardware_profile
hostname: raspberry-pi-a
audio:
  device: "Behringer WING"
  sample_rate: 48000
  track_mappings:
    click: [1]
    cue: [2]
    backing-track-l: [3]
    backing-track-r: [4]
    keys: [5, 6]
midi:
  device: "Behringer WING"
dmx:
  universes:
    - universe: 1
      name: light-show
```

Files are sorted by filename for deterministic ordering. Use numeric prefixes
(e.g., `01-pi-a.yaml`, `02-pi-b.yaml`, `99-fallback.yaml`) to control priority. Files with an
extension other than `.yaml`/`.yml` are ignored; a file that fails to parse is an error at
startup.

The directory **replaces** an inline `profiles:` list: when it holds at least one profile, the
inline list is ignored with a warning in the log. Only when the directory exists but holds no
profile files does mtrack fall back to the inline list. A missing directory is an error. Keep a
catch-all profile as a file in the directory (`99-fallback.yaml`) rather than inline.

The web UI's Config page edits profile files in place, and runtime gain changes are written to
the file the active profile came from.

## Controllers

Controllers (gRPC, OSC, MIDI, MCP) are defined per-profile under the `controllers` key.
They are initialized after all hardware devices are ready.

```yaml
profiles:
  - hostname: my-host
    audio:
      device: "Behringer WING"
      track_mappings:
        click: [1]
    midi:
      device: "Behringer WING"
    controllers:
      - kind: grpc
      - kind: osc
        port: 43235
      - kind: mcp
        port: 43237
      - kind: midi
        play:
          type: control_change
          channel: 16
          controller: 100
          value: 0
        prev:
          type: control_change
          channel: 16
          controller: 100
          value: 1
        next:
          type: control_change
          channel: 16
          controller: 100
          value: 2
        stop:
          type: control_change
          channel: 16
          controller: 100
          value: 3
        all_songs:
          type: control_change
          channel: 16
          controller: 100
          value: 4
        playlist:
          type: control_change
          channel: 16
          controller: 100
          value: 5
```

The keys of each controller kind are documented in the
[player configuration reference](player-config.md#a-profile-file); the interfaces themselves in
[gRPC Control](../interfaces/grpc.md), [OSC Control](../interfaces/osc.md) and
[MCP Control](../interfaces/mcp.md).

### Morningstar Integration

If you use a Morningstar MIDI controller (MC3, MC6, MC8, MC6 Pro, MC8 Pro, MC4 Pro),
mtrack can automatically push the current song name to the controller's display via
SysEx whenever the song changes. Add a `morningstar` block to your MIDI controller
configuration:

```yaml
controllers:
  - kind: midi
    play: { type: control_change, channel: 16, controller: 100, value: 0 }
    # ... other MIDI events ...
    morningstar:
      model: mc4pro     # Controller model (mc3, mc6, mc8, mc6pro, mc8pro, mc4pro)
      # save: false     # Save to flash (default: false = temporary, resets on power cycle)
```

The `model` field determines the SysEx device ID and the bank name length
(16 chars for MC3, 24 for MC6/MC8, 32 for Pro models). Names are automatically
truncated or padded to fit.

For unlisted models, use a custom device ID:

```yaml
    morningstar:
      model:
        custom:
          model_id: 15   # SysEx device ID byte (0-127)
```

### Section Loop Control

MIDI controllers can include events for acknowledging section loops and stopping them:

```yaml
controllers:
  - kind: midi
    play: { type: control_change, channel: 16, controller: 100, value: 0 }
    # ... other events ...
    section_ack:
      type: control_change
      channel: 16
      controller: 100
      value: 6
    stop_section_loop:
      type: control_change
      channel: 16
      controller: 100
      value: 7
```

## Status Events

Status events are MIDI events emitted periodically to indicate the player's state. This is
useful for driving LEDs on MIDI controllers. The statuses are emitted in a repeating cycle:
the **on** events (`idling_events` while stopped, `playing_events` while a song plays), held for
1 second; then the `off_events`, held for 250 ms; then on again. They need a `midi` device in
the same profile.

```yaml
profiles:
  - hostname: my-host
    audio:
      device: "UltraLite-mk5"
      track_mappings:
        click: [1]
    midi:
      device: "UltraLite-mk5"
    status_events:
      off_events:
        - type: control_change
          channel: 16
          controller: 3
          value: 2
      idling_events:
        - type: control_change
          channel: 16
          controller: 2
          value: 2
      playing_events:
        - type: control_change
          channel: 16
          controller: 2
          value: 2
```

A top-level `status_events` block in `mtrack.yaml` is the [legacy
layout](player-config.md#legacy-layout): it is used only when the matched profile has no
`status_events` of its own, and mtrack logs a warning asking you to move it into the profile.

## Notification Audio

Profiles can configure custom audio files for loop and section events. These notifications
play through the `mtrack:looping` track mapping.

```yaml
profiles:
  - hostname: my-host
    audio:
      device: "UltraLite-mk5"
      track_mappings:
        click: [1]
        mtrack:looping: [1, 2]
    notifications:
      # Audio file to play when a section loop is armed
      loop_armed: notifications/loop-armed.wav
      # Audio file to play when a break is requested during looping
      break_requested: notifications/break.wav
      # Audio file to play when exiting a loop
      loop_exited: notifications/exit.wav
      # Audio file to play when entering any section
      section_entering: notifications/section.wav
      # Per-section-name overrides
      sections:
        chorus: notifications/chorus.wav
        bridge: notifications/bridge.wav
```

Per-song overrides can be set in `song.yaml` via the `notification_audio` field. See the
[Song Configuration](../configuration/song-config.md) documentation.
