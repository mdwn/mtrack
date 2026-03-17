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

## DSL Lighting Features

- **Venue-Agnostic Shows**: Songs use logical groups instead of specific fixture names,
  so the same show works across different venues.
- **Tag-Based Group Resolution**: Fixtures are tagged with capabilities and roles.
  The system automatically selects optimal fixtures based on constraints.
- **Effects Engine**: Built-in effects (static, cycle, chase, strobe, pulse, dimmer,
  rainbow) with layering, blend modes, and timing control.
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

## Getting Started

1. Define fixture types and venues (see [Configuration](configuration.md))
2. Create a `.light` file for your song (see [Effects Reference](effects.md) and
   [Cueing Features](cueing.md))
3. Reference the light file in your song's `song.yaml`
4. Use the web UI's timeline editor to visually author and preview your show
