# Trigger Configuration

The trigger system provides a unified way to trigger sample playback from both audio inputs (piezo drum triggers) and MIDI events. Audio and MIDI inputs coexist in a single `inputs` list, discriminated by a required `kind` field.

Audio triggers use the same sample engine as MIDI triggers, so all sample features (velocity scaling, voice management, release groups, retrigger behavior) work identically regardless of input source.

## Configuration

Trigger configuration can be placed at the top level (legacy) or inside a hardware profile. Each input requires a `kind` field: `audio` or `midi`. The `device` field is only required when audio inputs are present.

```yaml
# Inside a hardware profile (recommended):
profiles:
  - hostname: drum-pi
    audio:
      device: "UltraLite-mk5"
      track_mappings:
        kick: [3, 4]
    trigger:
      device: "UltraLite-mk5"    # Required for audio inputs
      sample_rate: 44100
      # sample_format: int       # "int" or "float" (default: device native)
      # bits_per_sample: 16      # 16 or 32 (default: device native)
      # buffer_size: 256         # stream buffer size in frames (default: device default)
      # crosstalk_window_ms: 4   # suppression window (ms) after any channel fires
      # crosstalk_threshold: 3.0 # threshold multiplier during suppression
      inputs:
        # Audio trigger input (piezo drum trigger)
        - kind: audio
          channel: 1
          sample: "kick"
          threshold: 0.1
          retrigger_time_ms: 30
          scan_time_ms: 5
          gain: 1.0
          velocity_curve: linear
          release_group: "kick"
          # highpass_freq: 80.0               # high-pass filter cutoff in Hz
          # dynamic_threshold_decay_ms: 50    # adaptive threshold decay in ms
        - kind: audio
          channel: 3
          sample: "cymbal"
          threshold: 0.08
          release_group: "cymbal"
        - kind: audio
          channel: 4
          action: release
          release_group: "cymbal"
          threshold: 0.05
        # MIDI trigger input (alternative to top-level sample_triggers)
        - kind: midi
          event:
            type: note_on
            channel: 10
            key: 60
          sample: kick
```

Or as a top-level field (legacy, normalized into a profile at startup):

```yaml
trigger:
  device: "UltraLite-mk5"
  sample_rate: 44100
  inputs:
    - kind: audio
      channel: 1
      sample: "kick"
      threshold: 0.1
      retrigger_time_ms: 30
      scan_time_ms: 5
      gain: 1.0
      velocity_curve: linear
      release_group: "kick"
```

MIDI-only trigger configs don't need a device:

```yaml
trigger:
  inputs:
    - kind: midi
      event:
        type: note_on
        channel: 10
        key: 60
      sample: kick
    - kind: midi
      event:
        type: note_on
        channel: 10
        key: 62
      sample: snare
```

> **Note:** Top-level `sample_triggers` are still supported for backwards compatibility. At startup they are automatically converted to `kind: midi` inputs in the trigger config. When using profiles, top-level `sample_triggers` are ignored with a warning.

## Stream Configuration

| Parameter | Default | Description |
|-----------|---------|-------------|
| `sample_format` | device native | Sample format for the input stream: `int` or `float` |
| `bits_per_sample` | device native | Bits per sample: `16` or `32` |
| `buffer_size` | device default | Stream buffer size in frames; smaller values reduce latency |

## Detection Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `threshold` | 0.1 | Minimum amplitude to trigger (0.0–1.0) |
| `scan_time_ms` | 5 | Window (ms) after threshold crossing to find the peak amplitude |
| `retrigger_time_ms` | 30 | Lockout period (ms) after a trigger fires to prevent double-triggering |
| `gain` | 1.0 | Input gain multiplier applied before threshold comparison |
| `velocity_curve` | linear | How peak amplitude maps to velocity: `linear`, `logarithmic`, or `fixed` |
| `fixed_velocity` | 127 | Velocity value when `velocity_curve` is `fixed` |

## Signal Conditioning

Optional signal conditioning features for rejecting false triggers in live stage environments. All default to off for backward compatibility.

### Per-Input Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `highpass_freq` | *(disabled)* | High-pass filter cutoff in Hz. Rejects low-frequency stage rumble and bass cab vibration using a 2nd-order Butterworth filter. Typical values: 60–120 Hz. |
| `dynamic_threshold_decay_ms` | *(disabled)* | Decay time (ms) for an adaptive threshold that rises after each hit and exponentially decays back to the base threshold. Prevents piezo ringing from causing false re-triggers after lockout expires. Typical values: 20–80 ms. |
| `noise_floor_sensitivity` | *(disabled)* | Sensitivity multiplier for adaptive noise floor tracking. When set, the detector continuously estimates ambient noise via an EMA (updated only during Idle state) and raises the effective threshold to `noise_ema * sensitivity` if that exceeds the configured threshold. Can only raise the threshold, never lower it. Typical value: 5.0. |
| `noise_floor_decay_ms` | 200 | Time constant (ms) for the noise floor EMA. Controls how quickly the estimate tracks changing ambient levels. Smaller values react faster but are noisier; larger values are more stable. Only used when `noise_floor_sensitivity` is set. |

### Adaptive Noise Floor

If ambient noise on a channel rises during a performance (monitor bleed, rack vibration, stage volume changes), a static threshold can end up below the noise floor and cause phantom triggers. The adaptive noise floor tracker solves this by continuously estimating ambient noise during idle periods and raising the effective threshold when the environment gets louder.

Enable it by setting `noise_floor_sensitivity` on any input. The tracker computes an exponential moving average (EMA) of sample amplitude, updated **only during Idle state** (Scanning and Lockout freeze the estimate so hit transients don't pollute it). The effective threshold becomes `max(configured_threshold, noise_ema * sensitivity)`, so it can only raise the threshold above the configured value, never lower it.

```yaml
inputs:
  - kind: audio
    channel: 1
    sample: "kick"
    threshold: 0.1
    noise_floor_sensitivity: 5.0    # enable adaptive tracking
    noise_floor_decay_ms: 200       # default; 200ms time constant
```

### Device-Level Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `crosstalk_window_ms` | *(disabled)* | Suppression window (ms) after any channel fires during which all other channels have their thresholds temporarily elevated. Rejects vibration bleed between pads on a shared rack. Typical values: 2–6 ms. |
| `crosstalk_threshold` | *(disabled)* | Threshold multiplier applied to other channels during the crosstalk suppression window (e.g., `3.0` = 3x normal threshold). Both `crosstalk_window_ms` and `crosstalk_threshold` must be set to enable crosstalk suppression. |

## Velocity Curves

- **`linear`**: `velocity = peak * 127` (clamped to 0–127)
- **`logarithmic`**: Maps the threshold–1.0 range logarithmically to 1–127, giving more sensitivity at lower amplitudes
- **`fixed`**: Always uses the configured `fixed_velocity` value regardless of amplitude

## Release Groups and Choke

Trigger inputs can specify a `release_group` to enable voice management across inputs:

- When a trigger input fires, the new voice joins the named release group.
- A separate input with `action: release` can release (stop) all voices in that group.
- This enables cymbal choke behavior: one piezo triggers the cymbal, another chokes it.

## Latency

Total trigger-to-sound latency is approximately:
- Scan window: ~5ms (default `scan_time_ms`)
- Sample engine scheduling delay: ~buffer_size/sample_rate (~5.8ms at 256/44100)
- **Total: ~11ms**, well under the 20ms threshold for acceptable drum trigger response.

## Calibration

Manually tuning trigger parameters (threshold, gain, scan time, retrigger time, etc.) can be tedious. The `calibrate-triggers` command measures your actual hardware and generates a ready-to-paste YAML trigger configuration:

```
$ mtrack calibrate-triggers "UltraLite-mk5"
```

The calibration runs in three phases:

1. **Noise floor measurement** — Keep all pads silent while the tool captures ambient noise for a few seconds.
2. **Hit capture** — Hit each pad several times at varying velocities, then press Enter.
3. **Analysis** — The tool analyzes the captured data and prints a YAML trigger config to stdout.

Progress and diagnostics are printed to stderr, so the YAML output can be piped directly to a file:

```
$ mtrack calibrate-triggers "UltraLite-mk5" > trigger.yaml
```

Optional flags:

| Flag | Description |
|------|-------------|
| `--sample-rate <Hz>` | Override the input sample rate |
| `--duration <seconds>` | Noise floor measurement duration (default: 3) |
| `--sample-format <int\|float>` | Override the sample format |
| `--bits-per-sample <16\|32>` | Override bits per sample |

The generated config includes per-channel `threshold`, `gain`, `scan_time_ms`, `retrigger_time_ms`, and optional `highpass_freq`, `dynamic_threshold_decay_ms`, and device-level `crosstalk_window_ms`/`crosstalk_threshold` — all derived from measured data. Only channels with detected hits are included. Each channel has diagnostic comments showing the number of hits detected, noise floor peak, and max hit amplitude.
