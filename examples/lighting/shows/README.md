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
