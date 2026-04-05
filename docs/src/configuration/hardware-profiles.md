# Hardware Profiles

If you have multiple devices or run `mtrack` on multiple hosts sharing the same config file,
you can use hardware profiles instead of the flat `audio:` / `midi:` / `dmx:` / `track_mappings:` sections.
Each profile represents one complete host configuration with all subsystems. Profiles are filtered
by hostname; the first match is used.

```yaml
# Unified profiles: each entry defines one complete host configuration
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

Profiles with a `hostname` constraint only apply on hosts whose hostname matches. Profiles
without a hostname constraint match any host. Set the `MTRACK_HOSTNAME` environment variable
to override the system hostname (useful for testing or when the OS hostname differs from
what you want).

The existing flat format (`audio:` + `track_mappings:` + `midi:` + `dmx:`) continues to work
unchanged. At startup, legacy fields are automatically normalized into a single profile,
so all internal code paths use the same profile-based logic.

## External Profiles Directory

Instead of defining all profiles inline, you can load them from individual YAML files in a
directory. Each file defines one profile using the same format as inline profile entries.

```yaml
# Load profiles from a directory (path relative to this config file)
profiles_dir: profiles/

# Inline profiles still work alongside directory profiles.
# Directory profiles are prepended before inline profiles.
profiles:
  # Fallback for any host not matched by a directory profile
  - audio:
      device: "Built-in Audio"
      track_mappings:
        click: [1]
        backing-track-l: [1]
        backing-track-r: [2]
```

```yaml
# profiles/01-pi-a.yaml
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
(e.g., `01-pi-a.yaml`, `02-pi-b.yaml`, `99-fallback.yaml`) to control priority.

## Controllers

Controllers (gRPC, OSC, MIDI) are defined per-profile under the `controllers` key.
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
Off (1 second) → On (250ms, either idling or playing) → Off → On → ...

Status events are configured per-profile:

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

Legacy top-level `status_events` in `mtrack.yaml` are automatically normalized into the
matched profile at startup.

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
