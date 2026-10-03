# Command-Line Reference

`mtrack <command> --help` prints every option of a command. This page lists the commands and
where each is described in full.

## Running the player

| Command | What it does |
|---|---|
| `mtrack start [path]` | Starts the player on a project directory (or an `mtrack.yaml`), with the web UI on port 8080 (`--web-port`, `--web-address`) and `--tui` for the terminal UI. See [Quick Start](../getting-started/quick-start.md). |
| `mtrack systemd [paths…]` | Prints a systemd unit for running on startup. See [Running on Startup](../deployment/systemd.md). |

## Controlling a running player

These talk to the player's gRPC server (`-H host:port`); see [gRPC Control](../interfaces/grpc.md).

| Command | What it does |
|---|---|
| `mtrack play [--from <time>]` | Plays the current song, optionally from a time. |
| `mtrack seek <time or section>` | Seeks within the current song. |
| `mtrack previous`, `mtrack next`, `mtrack stop` | Moves through the playlist, or stops. |
| `mtrack switch-to-playlist <name>` | Switches playlist. |
| `mtrack status` | Prints the player's status. |
| `mtrack active-effects`, `mtrack cues` | Prints the running lighting effects, or the current song's cues. |

## Checking a project and its hardware

| Command | What it does |
|---|---|
| `mtrack songs <path> [--init]` | Lists and checks the songs under a directory. See [Song Configuration](../configuration/song-config.md). |
| `mtrack playlist <songs> <playlist>` | Checks a playlist against the songs. |
| `mtrack verify <mtrack.yaml>` | Checks the songs against the player config (`--check`, `--hostname`). |
| `mtrack devices`, `mtrack midi-devices` | Lists audio or MIDI devices. See [Discovering Devices](../getting-started/devices.md). |
| `mtrack test-audio <mtrack.yaml>` | Opens the configured audio device silently and confirms it streams. |
| `mtrack calibrate-triggers <device>` | Measures an audio input for trigger settings. See [Trigger Configuration](../configuration/triggers.md). |
| `mtrack verify-light-show <file> [--config <mtrack.yaml>]` | Checks a `.light` show. See [Light Show Verification](../lighting/verification.md). |

## Lighting files

Each of these works on a project directory (`--project`, default the current directory) and
the default lighting directories, which `--fixture-types-dir` and `--venues-dir` move as
`directories` does in the config
([Where the lighting files live](../lighting/configuration.md#where-the-lighting-files-live)).

### `mtrack import-gdtf`

```sh
mtrack import-gdtf fixture.gdtf --list-modes      # the modes and their footprints; writes nothing
mtrack import-gdtf fixture.gdtf                   # copy it into lighting/library/
mtrack import-gdtf fixture.gdtf --name "House Brick"
```

Copies a GDTF into `lighting/library/`, where it is a fixture type with all of its modes. A
fixture type has no mode: each venue fixture of it names its own (`mode "…"` on the fixture
line). `--name` gives the type a name other than the one in the file, which writes a small
record for it. See [GDTF fixture types](../lighting/configuration.md#gdtf-fixture-types).

### `mtrack import-mvr`

```sh
mtrack import-mvr venue.mvr                           # the plan; writes nothing
mtrack import-mvr venue.mvr --origin 0,-3500,0 --write
```

Seeds a `.venue` from an MVR (or merges a revised one into it), copying its GDTFs into the
library. `--name` names the venue. See
[Importing a venue's MVR](../lighting/configuration.md#importing-a-venues-mvr).

### `mtrack export-mvr`

```sh
mtrack export-mvr house                      # lighting/export/house.mvr
mtrack export-mvr house --output tour.mvr --layers-from-tags
```

Writes a venue as an MVR for a console or pre-viz tool, under `lighting/export/`.

### `mtrack migrate`

```sh
mtrack migrate              # what it would change; writes nothing
mtrack migrate --apply      # do it, keeping mtrack.yaml.bak
```

Moves an old single-file `mtrack.yaml` onto the current layout: inline hardware profiles into
a profiles directory, the legacy playlist into a playlists directory, and legacy top-level
hardware settings out of the file once they live in profiles. For lighting, it
moves inline fixtures (a `fixtures:` map under `lighting`, which the engine does not patch) into
a venue, `lighting/venues/inline_migrated.light`, and clears them from the config. Make that
venue current, or copy its lines into yours, for the fixtures to light. It does not touch fixture
types, GDTFs or venues that already exist.
