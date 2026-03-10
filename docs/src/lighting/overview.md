# Light Shows

Light shows and DMX playback are now supported through the use of the [Open Lighting Architecture](https://www.openlighting.org/).
The lighting system has been significantly enhanced with a new tag-based group resolution system that enables venue-agnostic lighting shows.

## New Lighting System Features

The new lighting system provides:

- **Venue-Agnostic Songs**: Songs use logical groups instead of specific fixture names
- **Tag-Based Group Resolution**: Fixtures are tagged with capabilities and roles
- **Intelligent Selection**: System automatically chooses optimal fixtures based on constraints
- **Venue Portability**: Same lighting show works across different venues
- **Performance Optimization**: Cached group resolutions for fast lookups

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
- **`AllowEmpty`**: Allow group to be empty if no fixtures match (graceful degradation, e.g., `true`)

## Benefits

1. **Venue Portability**: Same lighting show works across different venues automatically
2. **Intelligent Selection**: System prefers premium fixtures when available, falls back to standard
3. **Flexible Constraints**: Support for complex requirement combinations
4. **Clear Error Handling**: Know exactly what's missing when requirements aren't met
5. **Performance**: Cached resolutions for fast lookups
6. **Maintainable**: Easy to add new venues and fixture types

## Migration Path

- **Gradual adoption** - can mix old and new group definitions
- **Venue-defined groups** - venue-defined groups are still supported alongside logical groups
