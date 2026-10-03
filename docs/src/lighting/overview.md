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
  rainbow, move) with layering, blend modes, and timing control. All effects require an
  explicit duration — there are no perpetual or permanent effects.
- **Movers and Positions**: The `move` effect aims moving heads at named focus points in a
  `.venue` file, or at explicit pan and tilt angles. See the
  [Move Effect](effects.md#move-effect) and [Venue files with positions](configuration.md#venue-files-with-positions-venue).
- **GDTF and MVR**: A manufacturer's GDTF file is a fixture type as it stands: import it (or
  copy it into `lighting/library/`) and every one of its modes is available, with nothing to
  transcribe. Each fixture in a venue names the mode its unit is set to. A venue's MVR comes in
  as a positioned venue, and a venue goes back out as MVR. See
  [GDTF fixture types](configuration.md#gdtf-fixture-types)
  and [Importing a venue's MVR](configuration.md#importing-a-venues-mvr).
- **Testing a fixture**: A fixture's page sends to the real light — full white, swatches,
  sliders, every channel raw — and says what to check when nothing happens. See
  [Testing a fixture](web-ui.md#testing-a-fixture).
- **Pixel Fixtures**: Fixtures with cells can be driven per pixel with `per: cell` and
  `spread`. See [Rich channel definitions](configuration.md#rich-channel-definitions-fixture).
- **Stage 3D**: The stage card (on the dashboard and the Venues page) switches between the plot
  and the venue as a 3D room with live beams, and on the Venues page previews a song's show at
  any moment. See [Stage 3D](configuration.md#stage-3d).
- **Timeline Editor**: Visual DAW-style cue authoring in the web UI with integrated
  audio playback and real-time stage preview.
- **Sequences**: Reusable cue patterns that can be referenced from multiple shows.
- **Tempo-Aware Cueing**: Cues can be placed at measure/beat positions with automatic
  tempo change support.

## The Lighting area

The web UI's **Lighting** item holds all of it: an **Overview** that checks whether your shows
will reach the lights and says what to fix, **Fixture types** (import a GDTF, see a fixture in
3D with all its modes, test it, set what the GDTF does not say), **Venues** (patch, place and aim
fixtures on a stage plot or in 3D), **Groups** (the current venue and the logical groups, per
hardware profile) and **Fit shows** (tag an imported venue for your shows). See
[Lighting](web-ui.md) in the web UI guide.

## Configuration Structure

The lighting system uses a three-layer architecture:

1. **Configuration Layer**: Define logical groups with constraints, and pick the current venue,
   in the hardware profile (`dmx.lighting` in `mtrack.yaml` or a profile file)
2. **Venue Layer**: Tag physical fixtures with capabilities in DSL files
3. **Song Layer**: Reference `.light` DSL files in song YAML files, which use logical groups

## The files

| File | What it holds | Where it lives | Who writes it |
|---|---|---|---|
| `*.light` (show) | A song's cues: effects on groups, timed | The song's directory, named in `song.yaml` under `lighting:` | The song's timeline editor in the web UI, or you |
| `*.light` (fixture type) | A hand-written fixture type as a channel map | `lighting/fixture_types/` | The Fixture types page, or you |
| `*.fixture` | A hand-written fixture type with full channel definitions and cells, or the record of a GDTF fixture (its name, movement limits, strobe curve) | `lighting/fixture_types/` | The Fixture types page, `import-gdtf --name`, or you |
| `*.light` / `*.venue` (venue) | The rig at one place: fixtures, their types and modes, addresses, tags; `.venue` also positions, rotations and focus points | `lighting/venues/` | The Venues page, `import-mvr --write`, or you |
| `*.gdtf` | A manufacturer's fixture description; each one in the library is a fixture type | `lighting/library/` | `import-gdtf`, the Fixture types page, `import-mvr`, or a copy you make |
| `*.mvr` | A rig exchanged with a console or pre-viz tool | Input from anywhere; exports in `lighting/export/` | `export-mvr` and the Venues page; imports come from the venue |
| `lighting/.cache/` | Expanded GDTF modes, the library index, and the meshes and rig models the 3D view draws; rebuildable | `lighting/.cache/` | mtrack. Keep it out of version control |
| `mtrack.yaml` / profile files | The machine's DMX universes, its current venue and its logical groups | The project root, `profiles_dir` | The Config and Groups pages, or you |

All paths are relative to the directory holding `mtrack.yaml`; `lighting.directories` in the
profile moves the fixture types and venues directories. The formats are described in
[Configuration](configuration.md).

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

- **Explicit durations** — Every effect must have a `duration` (or `hold_time`) parameter, and
  the parser rejects one without. Two exceptions, both ways: `dimmer`'s `duration` defaults to
  1 s, and `move` needs `duration` itself, because a travel time is not something `hold_time`
  can stand in for.
- **No replacement semantics** — Multiple effects can coexist on the same layer simultaneously.
  The blend mode determines how overlapping effects combine.
- **No persistent state** — When an effect's duration expires, its contribution to the output
  is removed. Dimmer effects do not persist their final brightness level.

This model simplifies reasoning about light shows: each effect is a self-contained block with
a defined start time and duration. The timeline editor's layer lanes (foreground, midground,
background) make it easy to visualize how effects overlap and compose.

## Getting Started

[First Light](first-light.md) takes one fixture from its GDTF to a cue in a song, all in the web
UI, and is the place to start. The files behind each of its steps are described in
[Configuration](configuration.md); the show language in the [Effects Reference](effects.md) and
[Cueing Features](cueing.md).
