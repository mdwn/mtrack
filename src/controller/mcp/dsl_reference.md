# mtrack lighting DSL — quick reference

The lighting DSL describes four kinds of content, split by file extension:
**fixture types** (`.fixture`), **venues** (`.venue`), and **light shows** and
**sequences** (`.light`). Comments use `#` or `//` to end of line. Whitespace
is insignificant. A file may open with `version: 2` — the format version,
bumped only on breaking changes; files without the line are version 2.

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
| `static`  | `duration`                                             | Hold a color/intensity. Use `color`, and `intensity` or `dimmer` for the level. |
| `cycle`   | one or more `color:`, `duration`                       | Iterates colors. Optional `speed`, `direction`. |
| `strobe`  | `frequency`, `duration`                                | Hz strobe. Takes no color or intensity — it gates whatever is beneath it. |
| `pulse`   | `frequency`, `duration`                                | Sinusoidal pulse. Optional `base_level` (default 50%) and `pulse_amplitude` (also spelled `intensity`). |
| `chase`   | `speed`, `duration`                                    | A moving brightness mask over the layers beneath — needs a color bed under it. Optional `direction`, `pattern: linear|snake|random`. |
| `dimmer`  | `start_level`, `end_level`, `duration`                 | Linear ramp; `curve: linear` optional. |
| `rainbow` | `duration`                                             | Hue sweep. Optional `speed`. |

Every effect must specify a finite `duration`. Effects can crossfade — set
`up_time`, `hold_time`, and `down_time` (each a `time_parameter`).

### Common parameters

- `duration`, `up_time`, `down_time`, `hold_time`: time values. Units
  are `ms`, `s`, `beats`, `beat`, `measures`, or `measure`. **No whitespace
  between number and unit** — write `500ms`, `2s`, `4beats`, `2measures` (not
  `4 beats`). `speed` and `frequency` parameters accept the same forms
  (`speed: 1measure`, `frequency: 1beat`).
- `color` (**`static` and `cycle` only**): a named color (`"red"`, `"blue"`,
  `"white"`, `"orange"`, …), a hex string (`#FF8800` or `"#FF8800"`), or
  `rgb(255, 128, 0)`. The other effect types have no color of their own — a
  `chase` or `strobe` gates whatever is beneath it, so put the color on the bed.
- `dimmer`, `red`, `green`, `blue` (**`static` only**), and `intensity`
  (**`static` and `pulse` only**): floats `0.0`–`1.0`, or a percentage like
  `60%`.

These are listed per effect deliberately. A parameter an effect does not read is
accepted by the parser and dropped, so writing one produces a setting that never
takes effect — `validate_lighting` reports it as an `unused-parameter` warning.
- `direction`: the accepted values depend on the effect — they are two
  separate sets, not one list.
  - `cycle`: `forward | backward | pingpong` (the order colours are stepped).
  - `chase`: `left_to_right | right_to_left | top_to_bottom | bottom_to_top |
    clockwise | counter_clockwise` (where the mask travels).
  - No other effect reads `direction`; on `rainbow` it is accepted and ignored.
- `layer`: `background | midground | foreground` (grandMA-inspired layers).
- `blend_mode`: `replace | multiply | add | overlay | screen`.

A `pulse`'s amplitude is added *on top of* its base level: it sweeps
`base_level` to `base_level + amplitude` rather than modulating around the level
underneath. `pulse intensity: 16%` at the default base sweeps 50%–66%, not ±16%.
Set `base_level` explicitly to place the pulse where you want it.

### Layer commands

Layer state can be managed mid-show. Each command takes parenthesised
parameters: `layer:` is required, the others are optional.

```
@01:00.000
clear(layer: midground)                          # hard cut — stop this layer now
clear()                                          # hard cut on every layer
freeze(layer: background)                        # pin current output, ignore new cues
unfreeze(layer: background)                      # resume normal updates
master(layer: foreground, intensity: 50%)        # scale the layer's output
```

`master(...)` also accepts `speed:` (scales effect rates) in addition to
`intensity:`.

`clear(...)` is an immediate stop, not a fade. There is no layer command that
fades a layer out: every effect declares a finite duration, so author the
fade on the effect itself with `down_time:` and let it end when it should.

Layer masters and freezes last for the song that set them. They are reset when
playback stops or a new song loads, and when the layer is cleared (`clear(layer:
…)` resets that layer, `clear()` with no layer resets all of them) — so a show
stopped between a duck and its reset cue cannot leave the next song mastered
down.

### Sequences and inline loops

A `sequence "Name" { … }` block defines a reusable timeline of cues. Inside a
show, reference it on its own cue line:

```
sequence "Verse" {
    @00:00.000
    front_wash: static color: "blue", duration: 4s

    @00:02.000
    movers: chase speed: 2.0, direction: left_to_right, duration: 4s
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

A block is not always required. A show with none inherits the song's tempo —
its `tempo:` config if it has one, otherwise a map derived from the click
track's beat grid — so `@bar/beat` cueing works against a song whose timing was
never hand-written. A block in the file always wins. When writing one by hand,
check `song_details` for `lead_in_seconds` first: bar 1 beat 1 is where the
click actually starts, and `start: 0ms` is wrong by exactly the lead-in.

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

A cue targets one or more **groups**. Groups are declared in `mtrack.yaml`
under `dmx.lighting.groups` and select fixtures by the tags those fixtures
carry in the venue, so the same show works against a different rig. They are
defined in YAML, not the `.light` DSL:

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

Venues do not define groups of their own — tag the fixtures instead. A cue
cannot target a bare tag or a bare fixture name; it targets a group name from
the config.

`all_lights` is **not** auto-generated — define it explicitly as a logical
group with `MinCount: 1` if you want a catch-all. Use `list_groups` from MCP
to see what's actually resolvable in the current venue before authoring cues.

## Fixture type (rarely written from MCP)

Fixture types live in `.fixture` files. Each channel is a one-liner (name,
1-based offset, optional `fine` byte), with an optional block for DMX-range
functions carrying physical values:

```
fixture_type "RGBW_Par" {
    channels: 5
    channel "red" @ 1
    channel "green" @ 2
    channel "blue" @ 3
    channel "white" @ 4
    channel "dimmer" @ 5
}

fixture_type "Brick" {
    channels: 4
    channel "red" @ 1
    channel "green" @ 2
    channel "blue" @ 3
    channel "strobe" @ 4 {
        functions: { "off": 0..6, "strobe": 7..255 -> 0.4hz..25hz }
    }
}
```

The older `channel_map: { "red": 1, … }` form (in `.light` files) still parses
during the migration window; `mtrack migrate` rewrites it.

## Venue (rarely written from MCP)

Venues live in `.venue` files. Fixtures may optionally carry a stage
`position` (meters) and `rotation` (degrees), and a venue may bind named
`focus` points — coordinates are right-handed Z-up, origin downstage-center.

```
venue "main_stage" {
    fixture "Wash1" RGBW_Par @ 1:1 tags ["wash", "front"]
    fixture "Wash2" RGBW_Par @ 1:7 tags ["wash", "front"] position (-2.0, 3.5, 4.2)
    focus "drummer" (0.0, 2.8, 1.4)
}
```

Tags are the whole vocabulary a venue offers. Anything a show wants to address
— `front`, `left`, an odd/even split — is a tag on the fixtures, selected by a
logical group in the config.

## Authoring tips

1. **Always validate before writing.** Call `validate_lighting` with your draft.
2. **Discover groups first.** Use `list_groups` for the active venue so cues
   target real groups (`front_wash`, `movers`, etc.) rather than guesses.
3. **Crossfade by setting `up_time`/`down_time`.** Otherwise effects snap on.
4. **End the show.** A trailing `dimmer end_level: 0%, duration: 2s` on
   `all_lights` (or whatever your "all" group is called) gives a clean fade-out.
5. **Prefer absolute times for songs without tempo metadata.** Switch to
   `@bar/beat` only after confirming a `tempo { … }` block exists.
