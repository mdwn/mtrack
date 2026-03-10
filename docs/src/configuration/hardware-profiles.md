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
