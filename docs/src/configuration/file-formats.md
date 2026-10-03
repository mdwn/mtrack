# File Formats

## Configuration files

Configuration files are YAML. When mtrack scans a directory (the song repository,
`profiles_dir`, `playlists_dir`) it reads only files with a `.yaml` or `.yml` extension.

Each configuration file carries a `kind` field that identifies what it is, so that mtrack can
tell the file types apart when it scans a directory:

| `kind` | File | Lives in |
|--------|------|----------|
| `song` | a song definition, usually `song.yaml` | the [song repository](song-config.md) |
| `playlist` | a playlist | `playlists_dir` (or the file named by `playlist`) |
| `hardware_profile` | a [hardware profile](hardware-profiles.md) | `profiles_dir` |

`mtrack.yaml` itself has no `kind`. A song or profile file without a `kind` is still loaded;
adding it is recommended, because a song declared with `kind: song` that fails to load is shown
as an error in the web UI, where a `kind`-less file that fails is skipped silently.

## Audio files

`mtrack` decodes audio through the [symphonia](https://github.com/pdeljanov/Symphonia) library.
Supported formats, by file extension:

- **WAV** (`.wav`; PCM, various bit depths)
- **FLAC** (`.flac`)
- **MP3** (`.mp3`)
- **OGG Vorbis** (`.ogg`)
- **AAC / M4A** (`.aac`, `.m4a`, `.mp4`)
- **AIFF** (`.aiff`, `.aif`)

Every audio file is transcoded to the audio device's configuration (sample rate, bit depth, and
format) as it plays, each file on its own, so files can be mixed and matched within a song —
a WAV click track next to an MP3 backing track, at different sample rates.

## MIDI files

Standard MIDI files (`.mid`) with metrical (ticks-per-beat) timing. SMPTE timecode-based files
are rejected.

## Lighting files

Shows, fixture types and venues are written in the lighting DSL in `.light` files (fixture
types are also read from `.fixture`, venues from `.venue`); `.gdtf` fixture descriptions and
`.mvr` rig exports are imported. See the [Lighting overview](../lighting/overview.md).
