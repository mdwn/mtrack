# Lighting Configuration

The lighting system uses a three-layer architecture:

1. **Configuration Layer**: Define logical groups with constraints in `mtrack.yaml`
2. **Venue Layer**: Tag physical fixtures with capabilities in DSL files
3. **Song Layer**: Reference `.light` DSL files in song YAML files, which use logical groups

## Main Configuration (`mtrack.yaml`)

```yaml
dmx:
  # ... existing DMX configuration ...

  # New lighting system configuration
  lighting:
    # Current venue selection - determines which physical fixtures to use
    current_venue: "main_stage"

    # Simple inline fixture definitions (for basic cases)
    # These can be used instead of or alongside venue definitions
    fixtures:
      emergency_light: "Emergency @ 1:500"

    # Logical groups with role-based constraints
    groups:
      # Front wash lights - requires wash + front tags, needs 4-8 fixtures
      front_wash:
        name: "front_wash"
        constraints:
          - AllOf: ["wash", "front"]
          - MinCount: 4
          - MaxCount: 8

      # Moving head lights - accepts moving_head OR spot tags, prefers premium
      movers:
        name: "movers"
        constraints:
          - AnyOf: ["moving_head", "spot"]
          - Prefer: ["premium"]
          - MinCount: 2
          - MaxCount: 4

      # All lights - accepts any light type
      all_lights:
        name: "all_lights"
        constraints:
          - AnyOf: ["wash", "moving_head", "spot", "strobe", "beam"]
          - MinCount: 1

    # Directory configuration for DSL files (auto-discovered)
    directories:
      fixture_types: "lighting/fixture_types"
      venues: "lighting/venues"
```

## File versions

Lighting DSL files may declare a format version at the top of the file:

```light
version: 2
```

Versions mark **breakages, not expansions** — new syntax additions never bump
the version. A file without a declaration is version 2 (the version at which
the marker was introduced), so today the line is optional and purely
documentary; `mtrack migrate` writes it into generated files. If a future
format break mints version 3, those files must declare it, and an older
mtrack will refuse them with a clear "upgrade mtrack" error instead of a
parse failure.

## Fixture Type Definitions (`lighting/fixture_types/*.fixture`)

Each channel is declared on its own line: a name, its 1-based DMX offset, an
optional `fine` byte for 16-bit channels, and an optional block when the
channel has structure (DMX-range functions with physical values).

```light
# RGBW Par Can fixture type definition
fixture_type "RGBW_Par" {
  channels: 4
  channel "dimmer" @ 1
  channel "red" @ 2
  channel "green" @ 3
  channel "blue" @ 4
}

# RGB + Strobe fixture (e.g. Astera PixelBrick in 4-channel RGBS mode)
fixture_type "Astera-PixelBrick" {
  channels: 4
  channel "red" @ 1
  channel "green" @ 2
  channel "blue" @ 3
  channel "strobe" @ 4 {
    functions: { "off": 0..6, "strobe": 7..255 -> 0.4hz..25hz }
  }
}

# Moving Head fixture type definition
fixture_type "MovingHead" {
  channels: 16
  channel "dimmer" @ 1
  channel "pan" @ 2 fine 3
  channel "tilt" @ 4 fine 5
  channel "color_wheel" @ 6
  channel "gobo_wheel" @ 7
  channel "gobo_rotation" @ 8
  channel "focus" @ 9
  channel "zoom" @ 10
  channel "iris" @ 11
  channel "frost" @ 12
  channel "prism" @ 13
  channel "effects" @ 14
  channel "strobe" @ 15
  channel "control" @ 16
  max_strobe_frequency: 20.0
}
```

> **Migrating from `.light` fixture files:** the older `channel_map:` syntax and
> the `.light` extension still load, with a deprecation warning. Run
> `mtrack migrate` to rewrite fixture type files into the form above (and rename
> venue files to `.venue`); support for the old form will be removed.

**Strobe frequency range:**

Fixtures with a dedicated strobe channel declare their variable-strobe DMX range
and frequency range as a `strobe` function, as in the PixelBrick example above:
`"strobe": 7..255 -> 0.4hz..25hz` means DMX values 7–255 map to 0.4–25 Hz. A
fixture where only the maximum frequency is known can instead set the standalone
`max_strobe_frequency:` field (default 20.0), as in the MovingHead example.

This matters because many LED fixtures map the DMX strobe channel linearly to
*period* (1/frequency) rather than frequency, so a simple linear frequency-to-DMX
mapping produces incorrect results. `mtrack` uses period-linear interpolation to
match this behavior. At 10 Hz, the PixelBrick receives DMX 248 (period-linear),
not 103 (frequency-linear).

## Venue Definitions (`lighting/venues/*.venue`)

```light
# Main Stage venue definition
venue "main_stage" {
  # Front wash lights
  fixture "Wash1" RGBW_Par @ 1:1 tags ["wash", "front", "rgb", "premium"]
  fixture "Wash2" RGBW_Par @ 1:7 tags ["wash", "front", "rgb", "premium"]
  fixture "Wash3" RGBW_Par @ 1:13 tags ["wash", "front", "rgb"]
  fixture "Wash4" RGBW_Par @ 1:19 tags ["wash", "front", "rgb"]

  # Moving head lights
  fixture "Mover1" MovingHead @ 1:37 tags ["moving_head", "spot", "premium"]
  fixture "Mover2" MovingHead @ 1:53 tags ["moving_head", "spot", "premium"]
  fixture "Mover3" MovingHead @ 1:69 tags ["moving_head", "spot"]

  # Strobe lights
  fixture "Strobe1" Strobe @ 1:85 tags ["strobe", "front"]
  fixture "Strobe2" Strobe @ 1:87 tags ["strobe", "back"]
}

# Small Club venue definition (same logical groups work!)
venue "small_club" {
  # Limited front wash (only 2 fixtures)
  fixture "Wash1" RGBW_Par @ 1:1 tags ["wash", "front", "rgb"]
  fixture "Wash2" RGBW_Par @ 1:7 tags ["wash", "front", "rgb"]

  # Single moving head
  fixture "Mover1" MovingHead @ 1:13 tags ["moving_head", "spot", "premium"]

  # Single strobe
  fixture "Strobe1" Strobe @ 1:29 tags ["strobe", "front"]
}
```

**Positions and focus points (optional):**

Fixtures can carry a stage position (meters) and mounting rotation (degrees), and
a venue can bind named focus points to stage coordinates. Coordinates are
right-handed Z-up with the origin at downstage-center on the deck: +x stage-left,
+y upstage, +z up. A venue without positions still plays; these exist for the
stage view and for upcoming movement features.

```light
venue "kellys-basement" {
  fixture "Spot1" MovingHead @ 1:1
    tags ["spot", "rear"]
    position (-2.0, 3.5, 4.2) rotation (0, 0, 180)

  focus "drummer" (0.0, 2.8, 1.4)
  focus "center-stage" (0.0, 1.5, 1.7)
}
```

## Song Lighting Definitions

Lighting shows are defined in separate `.light` files using the DSL format. Songs reference these files:

```yaml
# Example song.yaml file
kind: song
name: "My Song"
lighting:
  - file: "lighting/main_show.light"  # Path relative to song directory
  - file: "lighting/outro.light"      # Multiple shows can be referenced
tracks:
  - name: "backing-track"
    file: "backing-track.wav"  # Can be WAV, MP3, FLAC, OGG, AAC, ALAC, etc.
```

The `.light` files use the DSL format and can reference logical groups defined in your `mtrack.yaml`:

```light
show "Main Show" {
    # Front wash on - uses logical group from mtrack.yaml
    @00:05.000
    front_wash: static color: "red", dimmer: 80%, duration: 10s

    # Movers join with color cycle - uses logical group
    @00:10.000
    movers: cycle color: "red", color: "blue", color: "green", speed: 2.0, dimmer: 100%, duration: 8s
}
```

> **Note:** All effects require an explicit `duration` parameter. Effects without a duration
> will be rejected by the parser. See the [Effects Reference](effects.md) for details.

See the [Light Show Verification](verification.md) section for information on validating your `.light` files.
