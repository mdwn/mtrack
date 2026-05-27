# mtrack lighting DSL — quick reference

`.light` files describe four kinds of content: **fixture types**, **venues**,
**light shows**, and **sequences**. A single file can contain any combination.
Comments use `#` or `//` to end of line. Whitespace is insignificant.

When generating a show for a song, you usually only write a `show "..." { … }`
block. Fixture types and venues are defined once for the whole rig and live in
the global lighting directory.

## Light show

```
show "Optional Name" {
    @00:00.000
    front_wash: static color: "blue", intensity: 0.6, duration: 5s

    @00:05.000
    movers, beams: cycle color: "red", color: "green", color: "blue", \
        duration: 8s, direction: forward, dimmer: 50%
}
```

- Each cue starts with a **timestamp** (`@mm:ss.mmm` or `@ss.mmm`) or a
  **measure** (`@bar/beat[.frac]`, e.g. `@4/1`).
- A cue applies one or more effects to one or more **groups** (`group_a, group_b: effect_name param: value, …`).
- Parameters are comma-separated `name: value` pairs. Parameter names use
  snake_case identifiers.
- Multiple effects under the same `@time` can be stacked by repeating
  `group: effect …` on the next line under the same timestamp.

### Effects

| Effect    | Required params                                        | Notes |
|-----------|--------------------------------------------------------|-------|
| `static`  | `duration`                                             | Hold a color/intensity. Use `color`, `dimmer`. |
| `cycle`   | one or more `color:`, `duration`                       | Iterates colors. Optional `speed`, `direction`. |
| `strobe`  | `frequency`, `duration`                                | Hz strobe. Optional `intensity`. |
| `pulse`   | `frequency`, `duration`                                | Sinusoidal pulse. Optional `intensity`. |
| `chase`   | `speed`, `duration`                                    | Optional `direction`, `pattern: linear|snake|random`. |
| `dimmer`  | `start_level`, `end_level`, `duration`                 | Linear ramp; `curve: linear` optional. |
| `rainbow` | `duration`                                             | Hue sweep. Optional `speed`. |

Every effect must specify a finite `duration`. Effects can crossfade — set
`up_time`, `hold_time`, and `down_time` (each a `time_parameter`).

### Common parameters

- `duration`, `up_time`, `down_time`, `hold_time`, `fade`: time values. Units
  are `ms`, `s`, `beats`, `beat`, `measures`, or `measure`. **No whitespace
  between number and unit** — write `500ms`, `2s`, `4beats`, `2measures` (not
  `4 beats`). `speed` and `frequency` parameters accept the same forms
  (`speed: 1measure`, `frequency: 1beat`).
- `color`: a named color (`"red"`, `"blue"`, `"white"`, `"orange"`, …), a hex
  string (`#FF8800` or `"#FF8800"`), or `rgb(255, 128, 0)`.
- `intensity`, `dimmer`, `red`, `green`, `blue`: floats `0.0`–`1.0`, or a
  percentage like `60%`.
- `direction`: `forward | backward | random | pingpong | left_to_right | right_to_left | top_to_bottom | bottom_to_top | clockwise | counter_clockwise`.
- `layer`: `background | midground | foreground` (grandMA-inspired layers).
- `blend_mode`: `replace | multiply | add | overlay | screen`.

### Layer commands

Layer state can be managed mid-show. Each command takes parenthesised
parameters: `layer:` is required, the others are optional.

```
@01:00.000
release(layer: foreground)                       # stop all effects on this layer
clear(layer: midground, time: 250ms)             # fade everything off over 250ms
freeze(layer: background)                        # pin current output, ignore new cues
unfreeze(layer: background)                      # resume normal updates
master(layer: foreground, intensity: 50%)        # scale the layer's output
```

`master(...)` also accepts `speed:` (scales effect rates) in addition to
`intensity:`.

### Sequences and inline loops

A `sequence "Name" { … }` block defines a reusable timeline of cues. Inside a
show, reference it on its own cue line:

```
sequence "Verse" {
    @00:00.000
    front_wash: static color: "blue", duration: 4s

    @00:02.000
    movers: chase speed: 2.0, direction: forward, duration: 4s
}

show "Song" {
    @00:00.000
    sequence "Verse"                # play it once
    @00:30.000
    sequence "Verse", loop: 4       # play it 4 times back-to-back
    @02:00.000
    sequence "Verse", loop: loop    # play it indefinitely
    @02:30.000
    stop sequence "Verse"
}
```

For one-off repetition without naming, use an inline loop. Timestamps inside
the block are relative to the loop's start:

```
@00:00.000
loop {
    @00:00.000
    all_lights: static color: "red", duration: 250ms
    @00:00.250
    all_lights: static color: "black", duration: 250ms
} repeats: 8
```

### Tempo and beat-based timing

A `tempo { ... }` block enables musical-time conversion. Place it either at
file scope (applies to every show/sequence) or as the first item inside a
`show { ... }` body. Fields go on separate lines:

```
tempo {
    start: 0ms
    bpm: 120
    time_signature: 4/4
    changes: [
        @8/1  { bpm: 140 },                       # snap to 140 at bar 8
        @16/1 { bpm: 160, transition: 4 },        # ramp over 4 beats
        @24/1 { bpm: 180, transition: 2m },       # ramp over 2 measures
        @32/1 { time_signature: 3/4 },            # change meter
        @48/1 { bpm: 120, transition: snap }      # explicit snap
    ]
}
```

With tempo set, cues can use `@bar/beat[.frac]` notation: `@4/1` is bar 4
beat 1, `@3/2.5` is bar 3 halfway between beats 2 and 3. Durations may use
`Nbeats`, `Nbeat`, `Nmeasures`, or `Nmeasure` (no whitespace). Tempo-change
transition durations use a slightly different syntax: a bare number means
beats, and `Nm` means measures (`transition: 4` = 4 beats; `transition: 2m`
= 2 measures; `transition: snap` for an instantaneous change).

### Measure offsets

Inside a cue you can shift the bar/beat baseline so the next cues are
expressed relative to a new "bar 1":

```
@8/1
offset 8 measures   # cues at @N/M now mean (N+8)/M in song time
all_lights: static color: "blue", duration: 4beats

reset_measures      # back to the original baseline
```

## Groups

A cue targets one or more **groups**. Groups come from two places:

1. **Logical groups** — declared in `mtrack.yaml` under `dmx.lighting.groups`.
   These match by tag/role and survive venue changes. Defined in YAML, not the
   `.light` DSL:

   ```yaml
   lighting:
     groups:
       all_lights:
         name: "all_lights"
         constraints:
           - MinCount: 1
       front_wash:
         constraints:
           - AllOf: ["wash", "front"]
           - MinCount: 1
   ```

2. **Venue groups** — declared inside a `venue "..." { … }` block. These are
   explicit member lists tied to that venue:

   ```
   venue "main_stage" {
       fixture "Wash1" RGBW_Par @ 1:1 tags ["wash", "front"]
       fixture "Wash2" RGBW_Par @ 1:7 tags ["wash", "front"]
       group "front_wash" = Wash1, Wash2
   }
   ```

`all_lights` is **not** auto-generated — define it explicitly as a logical
group with `MinCount: 1` if you want a catch-all. Use `list_groups` from MCP
to see what's actually resolvable in the current venue before authoring cues.

## Fixture type (rarely written from MCP)

```
fixture_type "RGBW_Par" {
    channels: 6
    channel_map: { "red": 1, "green": 2, "blue": 3, "white": 4, "dimmer": 5 }
}
```

## Venue (rarely written from MCP)

```
venue "main_stage" {
    fixture "Wash1" RGBW_Par @ 1:1 tags ["wash", "front"]
    fixture "Wash2" RGBW_Par @ 1:7 tags ["wash", "front"]
    group "front_wash" = Wash1, Wash2
}
```

## Authoring tips

1. **Always validate before writing.** Call `validate_lighting` with your draft.
2. **Discover groups first.** Use `list_groups` for the active venue so cues
   target real groups (`front_wash`, `movers`, etc.) rather than guesses.
3. **Crossfade by setting `up_time`/`down_time`.** Otherwise effects snap on.
4. **End the show.** A trailing `dimmer end_level: 0%, duration: 2s` on
   `all_lights` (or whatever your "all" group is called) gives a clean fade-out.
5. **Prefer absolute times for songs without tempo metadata.** Switch to
   `@bar/beat` only after confirming a `tempo { … }` block exists.
