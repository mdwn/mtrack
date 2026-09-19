# Lighting Effects Reference

The lighting system supports a variety of effect types, each with specific parameters and use cases.

## Effect Types

### Static Effect

Sets fixed parameter values for fixtures. Useful for solid colors, fixed dimmer levels, or any unchanging state.

**Parameters:**
- `color`: Color name (e.g., `"red"`, `"blue"`), hex (`#FF0000`), or RGB (`rgb(255,0,0)`)
- `dimmer` or `intensity`: Dimmer level (0-100% or 0.0-1.0)
- `red`, `green`, `blue`, `white`: Individual color channel levels (0-100% or 0.0-1.0)
- `duration`: **Required.** Duration after which effect stops (e.g., `5s`, `2measures`)

The level applies on every fixture, not just those with a dimmer channel. On a
fixture with a dedicated dimmer it drives that channel; on an RGB-only fixture it
scales the color instead, so the same show dims the same way in either venue.

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

With `spread` set, the cycle's targets each run a phase ahead of the last instead of in
lockstep, so the palette paints across the group (or, with `per: cell`, across a fixture's
cells) instead of every target showing the same color at once. See `spread` and `per` under
[Common Effect Parameters](#common-effect-parameters).

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
- `base_level`: Base dimmer level (0-100% or 0.0-1.0). Defaults to 50%
- `pulse_amplitude` or `intensity`: Amplitude of the pulse (0-100% or 0.0-1.0)
- `frequency`: Pulses per second (Hz), or tempo-aware (e.g., `2`, `1beat`)
- `duration`: **Required.** Duration of the pulse effect

The amplitude is added *on top of* the base level: the pulse sweeps
`base_level` to `base_level + amplitude`, it does not modulate around the level
underneath. So `pulse intensity: 16%` with the default base level sweeps 50% to 66%,
not ±16%. Set `base_level` explicitly to place the pulse where you want it.

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
- `direction`: `left_to_right`, `right_to_left`, `top_to_bottom`, `bottom_to_top`, `clockwise`, `counter_clockwise`.
  When the venue places every fixture in the group (`position` in a `.venue` file), these mean
  what they say on the stage plot, seen from the audience: `left_to_right` runs from stage-right
  to stage-left, `top_to_bottom` from upstage to downstage, `clockwise` around the group's
  center starting upstage. Without positions — or if any fixture in the group lacks one — the
  chase runs in the venue's list order, as before.
- `transition`: `snap` or `fade` for transitions between fixtures
- `duration`: **Required.** Duration of the chase effect (e.g., `10s`, `8measures`)

A chase is a moving brightness mask, not a color. It dims the fixtures it is not
currently on and passes through the ones it is, so the color comes from a lower
layer — put a bed underneath it rather than expecting the chase to light the rig by
itself.

With `per: cell`, a chase's spatial order runs through a fixture's cells as well as through
the group's fixtures, so it can step along the pixels of a single bar, or cross from one pixel
bar's cells into the next bar's. See `per` under
[Common Effect Parameters](#common-effect-parameters).

**Example:**
```light
@00:25.000
movers: static color: "red", duration: 10s, layer: background
movers: chase pattern: linear, speed: 2.0, direction: left_to_right, transition: fade, duration: 10s, layer: midground
bars: chase pattern: linear, direction: left_to_right, per: cell, duration: 10s, layer: midground
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

`spread` is where a rainbow becomes worth having across a pixel fixture or a row of them:
`spread: 360deg, per: cell, duration: 30s` paints one full rainbow across the cells instead of
every cell showing the same hue. See `spread` and `per` under
[Common Effect Parameters](#common-effect-parameters).

**Example:**
```light
@00:35.000
all_lights: rainbow speed: 1.0, saturation: 100%, brightness: 80%, duration: 10s

@00:45.000
bars: rainbow speed: 0.5, spread: 360deg, per: cell, duration: 30s
```

### Move Effect

Aims moving heads at a point on stage, or at explicit angles, travelling there in physical
space over the duration. Targets are the venue's **focus points** (`focus "drummer" (x, y, z)`
in a `.venue` file), so the same cue aims correctly in every venue that binds the name.

**Parameters:**
- `focus` (or `to`): The focus point to aim at, e.g. `"drummer"`
- `pan`, `tilt`: Explicit angles in degrees, written with the unit (`45deg`, `-20deg`). Give
  one or both; an omitted axis stays where it is. Not combined with `focus`.
- `from`: The focus point to start from (`from_pan`/`from_tilt` for angles). Without it the
  move starts from wherever each fixture is.
- `easing`: `smooth` (ease in and out, the default) or `linear`
- `duration`: **Required.** Travel time (e.g., `2s`, `1measure`)

A mover that has arrived **holds its pose** until the next `move` on it or a `clear`; the
effect's duration is the travel, not how long the fixture stays there. Seeking into a song
past a `move` lands the head where that move ended, every earlier move replayed in order so
the turn a head takes is the one it would have taken live. Movement is
interpolated in degrees and resolved per fixture through its pan/tilt ranges (16-bit where the
fixture has it), so a slow sweep is smooth. A fixture type that declares
`movement { max_pan_speed: 240deg/s }` is never driven faster than that; it arrives late
instead. Pan zero, tilt zero is the fixture's rest direction: straight down out of the
mounting frame, as GDTF models it. Positive pan turns counter-clockwise seen from above,
positive tilt swings the beam from straight down toward upstage, and the venue's
`rotation` is the mounting those turns happen inside — `(0, 0, 180)` for a hung head
facing downstage, so `pan: 0deg, tilt: 90deg` throws at the audience. See
[Mounting and pose convention](configuration.md#mounting-and-pose-convention).

`validate_lighting` warns about a focus point the current venue does not bind
(`unbound-focus-point`), a `move` on movers the venue has not placed
(`move-without-positions`), a group with no pan/tilt channels (`capability-gap`), and movers
whose fixture type carries no pan/tilt range so degrees resolve over an assumed travel
(`move-imprecise`, fixed by importing the GDTF or adding `range` to the channels).

To check where a show points without a rig, `evaluate_show` (MCP) reports every mover's
`pan`, `tilt` and their `_fine` bytes at any instant, resolved through the same code as the
wire; `get_fixture_state` reports the same for a running song. The example show
`movers_demo.light` aims the example venue's movers at its focus points.

**Example:**
```light
@00:12.000
spots: move focus: "drummer", duration: 2s, easing: smooth

@00:20.000
spots: move from: "center-stage", to: "drummer", duration: 2measures

@00:30.000
spots: move pan: 45deg, tilt: -20deg, duration: 1s, easing: linear
```

## Common Effect Parameters

All effects support these optional parameters for advanced control:

- `layer`: Effect layer - `background`, `midground`, or `foreground` (for layering)
- `blend_mode`: How effect blends with lower layers - `replace`, `multiply`, `add`, `overlay`, `screen`
- `up_time`: Fade-in duration (e.g., `2s`, `1beat`)
- `hold_time`: Duration to hold at full intensity (e.g., `5s`, `4measures`)
- `down_time`: Fade-out duration (e.g., `1s`, `2beats`)
- `per`: `fixture` (the default) or `cell`. A pixel fixture — an LED batten, a mover's pixel
  ring — normally shows one color across all its cells, ganged together. `per: cell` expands
  the effect's targets from fixtures to their cells, so a chase or a rainbow can run *across*
  the pixels of a single fixture, not just across fixtures in a group. Expansion follows
  spatial order: the group's fixtures in stage order (as `direction` on a chase already sorts
  them), and within each fixture its cells in stage order along the same axis — so a
  left-to-right chase over a row of pixel bars runs through every bar's cells in turn, crossing
  from one bar into the next. A fixture with no cells expands to itself, so a mixed group of
  plain and pixel fixtures still works. `per: cell` is a no-op on effects that give every
  target the same value (`static`, `strobe`, `pulse`, `dimmer`) — the lint's
  `per-cell-no-effect` warning says so. It also does nothing, with the lint's `cells-absent`
  warning, on a group whose fixtures have no cells in the current venue. The dashboard's
  stage plot and the Stage 3D view both show the cells: the plot as a segmented disc, the 3D
  view lighting each cell's own lens and beam.
- `spread`: An angle in degrees (e.g. `spread: 180deg`), for `rainbow` and `cycle` only — the
  lint's `spread-unused` warning fires on any other effect. It offsets each of the effect's
  ordered targets by its share of the spread: `spread: 360deg` paints one full rainbow, or one
  full pass of a color cycle's palette, across the targets at once; `spread: 0deg` (the
  default) keeps every target in phase, today's behavior. It works with `per: fixture` (spread
  across a group) or `per: cell` (spread across the cells within a fixture, or across cells and
  fixtures together).

**Example with crossfades:**
```light
@00:05.000
front_wash: static color: "blue", dimmer: 100%, up_time: 2s, hold_time: 5s, down_time: 1s
```
