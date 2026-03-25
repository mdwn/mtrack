# Lighting Effects Reference

The lighting system supports a variety of effect types, each with specific parameters and use cases.

## Effect Types

### Static Effect

Sets fixed parameter values for fixtures. Useful for solid colors, fixed dimmer levels, or any unchanging state.

**Parameters:**
- `color`: Color name (e.g., `"red"`, `"blue"`), hex (`#FF0000`), or RGB (`rgb(255,0,0)`)
- `dimmer`: Dimmer level (0-100% or 0.0-1.0)
- `red`, `green`, `blue`, `white`: Individual color channel levels (0-100% or 0.0-1.0)
- `duration`: **Required.** Duration after which effect stops (e.g., `5s`, `2measures`)

**Example:**
```light
@00:05.000
front_wash: static color: "red", dimmer: 80%, duration: 10s

@00:10.000
back_wash: static red: 100%, green: 50%, blue: 0%, dimmer: 60%, duration: 5s
```

### Color Cycle Effect

Cycles through a list of colors continuously. Colors transition smoothly or instantly based on transition mode.

**Parameters:**
- `color`: Multiple color values (e.g., `color: "red", color: "green", color: "blue"`)
- `speed`: Cycles per second, or tempo-aware (e.g., `1.5`, `1measure`, `2beats`)
- `direction`: `forward`, `backward`, or `pingpong`
- `transition`: `snap` (instant) or `fade` (smooth)
- `duration`: **Required.** Total duration of the effect (e.g., `10s`, `4measures`)

**Example:**
```light
@00:10.000
movers: cycle color: "red", color: "blue", color: "green", speed: 2.0, direction: forward, transition: fade, duration: 10s
```

### Strobe Effect

Rapidly flashes fixtures on and off at a specified frequency.

**Parameters:**
- `frequency`: Flashes per second (Hz), or tempo-aware (e.g., `8`, `1beat`, `0.5measures`)
- `duration`: **Required.** Duration of the strobe effect (e.g., `3s`, `4measures`)

**Example:**
```light
@00:15.000
strobes: strobe frequency: 8, duration: 2s

@01:00.000
all_lights: strobe frequency: 1beat, duration: 4measures
```

### Pulse Effect

Smoothly pulses the dimmer level up and down, creating a breathing effect.

**Parameters:**
- `base_level`: Base dimmer level (0-100% or 0.0-1.0)
- `pulse_amplitude` or `intensity`: Amplitude of the pulse (0-100% or 0.0-1.0)
- `frequency`: Pulses per second (Hz), or tempo-aware (e.g., `2`, `1beat`)
- `duration`: **Required.** Duration of the pulse effect

**Example:**
```light
@00:20.000
front_wash: pulse base_level: 50%, pulse_amplitude: 50%, frequency: 1.5, duration: 5s
```

### Chase Effect

Moves an effect pattern across multiple fixtures in a spatial pattern.

**Parameters:**
- `pattern`: `linear`, `snake`, or `random`
- `speed`: Steps per second, or tempo-aware (e.g., `2.0`, `1measure`)
- `direction`: `left_to_right`, `right_to_left`, `top_to_bottom`, `bottom_to_top`, `clockwise`, `counter_clockwise`
- `transition`: `snap` or `fade` for transitions between fixtures
- `duration`: **Required.** Duration of the chase effect (e.g., `10s`, `8measures`)

**Example:**
```light
@00:25.000
movers: chase pattern: linear, speed: 2.0, direction: left_to_right, transition: fade, duration: 10s
```

### Dimmer Effect

Smoothly transitions dimmer level from start to end over a duration.

**Parameters:**
- `start_level` or `start`: Starting dimmer level (0-100% or 0.0-1.0)
- `end_level` or `end`: Ending dimmer level (0-100% or 0.0-1.0)
- `duration`: Transition duration (e.g., `3s`, `2measures`)
- `curve`: Transition curve - `linear`, `exponential`, `logarithmic`, `sine`, `cosine`

**Example:**
```light
@00:30.000
all_lights: dimmer start_level: 100%, end_level: 0%, duration: 3s, curve: sine
```

### Rainbow Effect

Generates a continuous rainbow color cycle across the color spectrum.

**Parameters:**
- `speed`: Cycles per second, or tempo-aware (e.g., `1.0`, `1measure`)
- `saturation`: Color saturation (0-100% or 0.0-1.0)
- `brightness`: Overall brightness (0-100% or 0.0-1.0)
- `duration`: **Required.** Duration of the rainbow effect (e.g., `10s`, `8measures`)

**Example:**
```light
@00:35.000
all_lights: rainbow speed: 1.0, saturation: 100%, brightness: 80%, duration: 10s
```

## Common Effect Parameters

All effects support these optional parameters for advanced control:

- `layer`: Effect layer - `background`, `midground`, or `foreground` (for layering)
- `blend_mode`: How effect blends with lower layers - `replace`, `multiply`, `add`, `overlay`, `screen`
- `up_time`: Fade-in duration (e.g., `2s`, `1beat`)
- `hold_time`: Duration to hold at full intensity (e.g., `5s`, `4measures`)
- `down_time`: Fade-out duration (e.g., `1s`, `2beats`)

**Example with crossfades:**
```light
@00:05.000
front_wash: static color: "blue", dimmer: 100%, up_time: 2s, hold_time: 5s, down_time: 1s
```
