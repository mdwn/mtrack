# Lighting Show Examples

This directory contains example lighting shows that demonstrate various features of the mtrack lighting system.

## Show Files

### `layering_show.light`
Demonstrates the effect layering system with crossfades:
- Background, midground, and foreground layers
- Different blend modes (replace, multiply, overlay, add)
- Crossfade in/out timing
- Complex effect combinations

### `comprehensive_show.light`
A comprehensive example showing all effect types with crossfades:
- Static colors with crossfades
- Color cycling with smooth transitions
- Strobe effects with fade in/out
- Chase patterns with crossfades
- Dimmer effects with smooth transitions
- Rainbow effects with crossfades
- Pulse effects with fade timing

### `crossfade_show.light`
A dedicated crossfade demonstration:
- Various crossfade timing examples
- Complex layering with crossfades
- All effect types with crossfade support
- Professional lighting console-style transitions

### `dimmer_curves_demo.light`
Demonstrates different dimmer curve types:
- Linear curves (constant rate)
- Exponential curves (slow start, fast end)
- Logarithmic curves (fast start, slow end)
- Sine/cosine curves (smooth ease in/out)

### `measure_timing_demo.light`
Demonstrates measure-based timing and tempo control:
- Tempo section with BPM and time signature
- Measure/beat notation for cue timing
- Tempo changes (instant and gradual)
- Time signature changes
- Transition durations in beats or measures

## Crossfade Syntax

The lighting system supports professional-grade crossfades for smooth transitions:

```light
# Basic crossfade syntax
effect_name: effect_type parameters..., fade_in: 2s, fade_out: 1s, duration: 5s

# Examples
front_wash: static color: "blue", dimmer: 100%, fade_in: 2s
back_wash: cycle color: "red", color: "green", speed: 1.0, fade_in: 1s, fade_out: 1s, duration: 8s
strobe_lights: strobe frequency: 4, fade_in: 0.5s, fade_out: 0.5s, duration: 3s
```

### Crossfade Parameters

- `fade_in: <duration>` - Time to fade in from 0% to 100% intensity
- `fade_out: <duration>` - Time to fade out from 100% to 0% intensity  
- `duration: <duration>` - Total effect duration (optional)
- Both fade parameters are optional
- Duration can be specified in seconds (s), milliseconds (ms), or as time values (e.g., 2.5s)

### Crossfade Behavior

- **Fade In**: Linear ramp from 0% to 100% over specified duration
- **Fade Out**: Linear ramp from 100% to 0% over specified duration
- **Combined**: Takes minimum of both multipliers for smooth transitions
- **Layering**: Crossfades work with all blend modes and layers
- **Professional**: Follows industry standards used by GrandMA, Hog, ETC Eos

### Example Timeline

```
@00:00.000  # Effect starts, begins fade in
@00:02.000  # Fade in complete (100% intensity)
@00:08.000  # Effect at full intensity
@00:09.000  # Fade out begins
@00:10.000  # Fade out complete (0% intensity)
```

This creates smooth, professional lighting transitions that are standard in the lighting industry.

## Tempo-Based Timing

The lighting system supports two timing methods: absolute time and measure-based timing.

### Absolute Time (Traditional)

```light
@00:00.000  # Minutes:Seconds.Milliseconds
@00:12.500  # 12.5 seconds
@01:30.000  # 1 minute 30 seconds
```

### Measure-Based Timing

For music-synchronized shows, use measure/beat notation:

```light
@1/1        # Measure 1, beat 1
@1/2        # Measure 1, beat 2
@4/1        # Measure 4, beat 1
@8/2.5      # Measure 8, halfway between beat 2 and 3
```

**Note**: Measures are 1-indexed (first measure is measure 1, not 0).

### Tempo Section

When using measure-based timing, you **must** define a tempo section:

```light
tempo {
    start: 0.0s              # Offset where music starts (seconds)
    bpm: 120                 # Beats per minute
    time_signature: 4/4      # Time signature
    changes: [
        # Tempo and time signature changes go here
    ]
}
```

#### Tempo Changes

Tempo and time signature changes must be specified at measure/beat positions:

```light
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
        # Instant tempo change (snap is default)
        @8/1 { bpm: 140 },
        @8/1 { bpm: 140, transition: snap },
        
        # Gradual change over 4 beats
        @16/1 { bpm: 160, transition: 4 },
        
        # Gradual change over 2 measures
        @24/1 { bpm: 180, transition: 2m },
        
        # Time signature change
        @32/1 { time_signature: 3/4 }
    ]
}
```

#### Transition Duration Formats

- `snap` - Instant change (default)
- `<number>` - Gradual change over N beats (e.g., `4`)
- `<number>m` - Gradual change over N measures (e.g., `2m`)

### Complete Example

```light
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
        @8/1 { bpm: 140 },
        @16/1 { bpm: 160, transition: 4 }
    ]
}

show "My Show" {
    @1/1
    front_wash: static color: "blue", dimmer: 100%
    
    @2/1
    front_wash: pulse color: "blue", duration: 500ms
    
    @8/1  # When tempo changes to 140 BPM
    back_lights: chase colors: ["red", "green"], duration: 1s
}
```

### Mixing Time Formats

You can mix absolute and measure-based timing in the same show:

```light
@00:00.000  # Absolute time
front_wash: static color: "blue"

@1/1        # Measure-based
back_lights: chase colors: ["red", "green"]
```

However, tempo changes can only be specified at measure/beat positions.
