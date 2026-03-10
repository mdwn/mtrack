# MIDI-Triggered Samples

`mtrack` supports triggering audio samples via MIDI events. This is useful for playing one-shot sounds like clicks, cues, sound effects, or drum samples during a performance. Samples are preloaded into memory and transcoded at startup for low-latency playback. Trigger latency is approximately 2x the audio buffer size (e.g., ~11.6ms at 256 samples/44.1kHz).

## Global vs Per-Song Samples

Samples can be configured at two levels:

1. **Global samples** - Defined in the main `mtrack.yaml` configuration file. These are available throughout the entire session.
2. **Per-song samples** - Defined in individual song configuration files. These override or extend the global configuration when that song is selected.

## Sample Configuration

Samples are defined in two parts: **sample definitions** (the audio files and their behavior) and **sample triggers** (the MIDI events that play them).

### Sample Definitions

```yaml
samples:
  # Each sample has a name that can be referenced by triggers.
  kick:
    # The audio file to play. Path is relative to the config file.
    file: samples/kick.wav

    # Output channels to route this sample to (1-indexed).
    # Use output_channels for fixed channel numbers, or output_track to reference
    # a track mapping name from the active hardware profile.
    output_channels: [3, 4]

    # Alternatively, use output_track to reference a track mapping by name.
    # This lets the same sample definition work across different hardware profiles
    # with different channel assignments. If both are set, output_track takes precedence.
    # output_track: "kick-out"

    # Velocity handling configuration.
    velocity:
      # Mode can be: ignore, scale, or layers.
      mode: scale

    # Behavior when released: play_to_completion, stop, or fade.
    # (Also accepts "note_off" as a key name for backwards compatibility.)
    release_behavior: play_to_completion

    # Behavior when retriggered while playing: cut or polyphonic.
    retrigger: cut

    # Maximum concurrent voices for this sample (optional).
    max_voices: 4

    # Fade time in milliseconds when release_behavior is "fade" (default: 50).
    fade_time_ms: 100
```

When using `output_track`, the sample's output channels are resolved through the active profile's `track_mappings`. This is useful when multiple hosts share a config file but have different channel assignments:

```yaml
samples:
  kick:
    file: samples/kick.wav
    output_track: "kick-out"     # resolved via profile's track_mappings

profiles:
  - hostname: pi-a
    audio:
      device: "UltraLite-mk5"
      track_mappings:
        kick-out: [3, 4]         # pi-a routes kick to channels 3-4
  - hostname: pi-b
    audio:
      device: "UltraLite-mk5"
      track_mappings:
        kick-out: [13, 14]       # pi-b routes kick to channels 13-14
```

### Sample Triggers

Triggers map MIDI events to samples. For Note On/Off events, only the channel and key are matched — the velocity from the incoming MIDI event is used for volume scaling or layer selection.

The preferred way to define MIDI triggers is as `kind: midi` inputs in the [trigger configuration](triggers.md):

```yaml
trigger:
  inputs:
    - kind: midi
      event:
        type: note_on
        channel: 10
        key: 60  # C3
      sample: kick
    - kind: midi
      event:
        type: note_on
        channel: 10
        key: 62  # D3
      sample: snare
```

The legacy top-level `sample_triggers` format is still supported and automatically converted at startup:

```yaml
sample_triggers:
- trigger:
    type: note_on
    channel: 10
    key: 60  # C3
  sample: kick
- trigger:
    type: note_on
    channel: 10
    key: 62  # D3
  sample: snare
```

## Velocity Handling Modes

### Ignore Mode

Ignores the MIDI velocity and plays at a fixed volume:

```yaml
velocity:
  mode: ignore
  default: 100  # Fixed velocity (0-127), defaults to 100
```

### Scale Mode

Scales the playback volume based on MIDI velocity (velocity/127):

```yaml
velocity:
  mode: scale
```

### Layers Mode

Selects different audio files based on velocity ranges. Useful for realistic drum sounds:

```yaml
velocity:
  mode: layers
  # Optional: also scale volume within each layer.
  scale: true
  layers:
  - range: [1, 60]      # Soft hits
    file: samples/snare_soft.wav
  - range: [61, 100]    # Medium hits
    file: samples/snare_medium.wav
  - range: [101, 127]   # Hard hits
    file: samples/snare_hard.wav
```

## Release Behavior

Controls what happens when a voice is released (via MIDI Note Off or audio trigger release):

- **`play_to_completion`** (default) - Ignores the release, lets the sample play to the end.
- **`stop`** - Immediately stops the sample.
- **`fade`** - Fades out the sample over the configured `fade_time_ms`.

Note: In configuration files, both `release_behavior` and `note_off` are accepted as key names.

## Retrigger Behavior

Controls what happens when a sample is triggered while it's already playing:

- **`cut`** (default) - Stops the previous instance and starts a new one.
- **`polyphonic`** - Allows multiple instances to play simultaneously.

## Voice Limits

To prevent resource exhaustion, you can limit concurrent voices:

```yaml
# Global limit for all samples.
max_sample_voices: 32

samples:
  hihat:
    # Per-sample limit (in addition to global limit).
    max_voices: 8
```

When limits are exceeded, the oldest voice is stopped to make room for new ones.

## Stopping All Samples

All triggered samples can be stopped via:

- **OSC**: Send a message to `/mtrack/samples/stop` (configurable via `stop_samples` in OSC controller config)
- **gRPC**: Call the `StopSamples` RPC method

## Per-Song Sample Overrides

Individual songs can override or extend the global sample configuration:

```yaml
# In a song's configuration file (e.g., songs/my-song/song.yaml)
name: My Song

tracks:
- file: click.wav
  name: click
- file: backing.wav
  name: backing

# Override global samples for this song.
samples:
  kick:
    file: custom_kick.wav  # Use a different kick for this song
    output_channels: [5, 6]

# Add song-specific triggers.
sample_triggers:
- trigger:
    type: note_on
    channel: 10
    key: 64
  sample: kick
```
