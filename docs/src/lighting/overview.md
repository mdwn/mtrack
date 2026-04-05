# Light Shows

mtrack supports programmable lighting control via DMX through the
[Open Lighting Architecture](https://www.openlighting.org/). There are two approaches
to authoring light shows:

- **DSL-based lighting** (recommended) — A tag-based system with a custom lighting DSL,
  a visual timeline editor, and venue-agnostic shows. This is the primary lighting system
  and the focus of this documentation.
- **MIDI-based DMX** — Direct per-channel DMX control via MIDI files. Useful for precise
  channel programming or integration with DAW workflows. See the
  [MIDI-Based DMX](../dmx/midi-dmx.md) section.

Both systems can be used independently or combined in the same project. For most users,
the DSL system is the better starting point — it provides high-level effect definitions,
works across different venues without modification, and integrates with the web UI's
timeline editor for visual cue authoring with real-time playback preview.

## Why a custom DSL?

There is no widely adopted open standard for cue-based lighting show authoring. Existing
formats either target specific commercial platforms (e.g. proprietary console show files),
operate at the raw DMX channel level (like Art-Net recordings or MIDI-to-DMX mappings),
or focus on fixture patching rather than show programming.

mtrack's lighting DSL fills that gap. It operates at the level musicians and small
production teams think in — effects on groups of lights, timed to music — rather than
individual DMX channel values. The DSL is plain text, stored in `.light` files alongside
your songs, version-controllable, and human-readable. The web UI's timeline editor
provides a visual interface on top of it, so you don't need to write DSL by hand unless
you want to.

## DSL Lighting Features

- **Venue-Agnostic Shows**: Songs use logical groups instead of specific fixture names,
  so the same show works across different venues.
- **Tag-Based Group Resolution**: Fixtures are tagged with capabilities and roles.
  The system automatically selects optimal fixtures based on constraints.
- **Effects Engine**: Built-in effects (static, cycle, chase, strobe, pulse, dimmer,
  rainbow) with layering, blend modes, and timing control. All effects require an explicit
  duration — there are no perpetual or permanent effects.
- **Timeline Editor**: Visual DAW-style cue authoring in the web UI with integrated
  audio playback and real-time stage preview.
- **Sequences**: Reusable cue patterns that can be referenced from multiple shows.
- **Tempo-Aware Cueing**: Cues can be placed at measure/beat positions with automatic
  tempo change support.

## Configuration Structure

The lighting system uses a three-layer architecture:

1. **Configuration Layer**: Define logical groups with constraints in `mtrack.yaml`
2. **Venue Layer**: Tag physical fixtures with capabilities in DSL files
3. **Song Layer**: Reference `.light` DSL files in song YAML files, which use logical groups

## Constraint Types

The system supports several constraint types for group resolution:

- **`AllOf`**: All specified tags must be present (e.g., `["wash", "front"]`)
- **`AnyOf`**: Any of the specified tags must be present (e.g., `["moving_head", "spot"]`)
- **`Prefer`**: Prefer fixtures with these tags (e.g., `["premium"]`)
- **`MinCount`**: Minimum number of fixtures required
- **`MaxCount`**: Maximum number of fixtures allowed
- **`FallbackTo`**: Fallback to another group if primary group fails (e.g., `"all_lights"`)
- **`AllowEmpty`**: Allow group to be empty if no fixtures match (graceful degradation)

## Benefits

1. **Venue Portability**: Same lighting show works across different venues automatically
2. **Intelligent Selection**: System prefers premium fixtures when available, falls back to standard
3. **Flexible Constraints**: Support for complex requirement combinations
4. **Clear Error Handling**: Know exactly what's missing when requirements aren't met
5. **Visual Authoring**: Timeline editor with playback preview and stage visualization
6. **Maintainable**: Easy to add new venues and fixture types

## Effect Model

Effects in mtrack are **finite, independent blocks on a timeline**:

- **Explicit durations** — Every effect must have a `duration` (or `hold_time`) parameter.
  Effects that don't specify a duration are rejected by the parser.
- **No replacement semantics** — Multiple effects can coexist on the same layer simultaneously.
  The blend mode determines how overlapping effects combine.
- **No persistent state** — When an effect's duration expires, its contribution to the output
  is removed. Dimmer effects do not persist their final brightness level.

This model simplifies reasoning about light shows: each effect is a self-contained block with
a defined start time and duration. The timeline editor's layer lanes (foreground, midground,
background) make it easy to visualize how effects overlap and compose.

## Getting Started

1. Define fixture types and venues (see [Configuration](configuration.md))
2. Create a `.light` file for your song (see [Effects Reference](effects.md) and
   [Cueing Features](cueing.md))
3. Reference the light file in your song's `song.yaml`
4. Use the web UI's timeline editor to visually author and preview your show
