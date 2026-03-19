# Quick Start

This page gets you from installation to a working setup using the web UI.

## Start mtrack

```
# Start in a directory containing songs (or an empty directory to start fresh)
cd /path/to/my/songs
mtrack start

# Or point at a specific directory
mtrack start /path/to/my/songs
```

mtrack creates an `mtrack.yaml` configuration file automatically if one does not exist.

> **Note:** mtrack needs **write access** to the project directory in order to manage
> configuration, songs, playlists, and lighting files through the web UI. If the directory
> is read-only, playback works but editing features are disabled.

## Open the Web UI

Navigate to **<http://localhost:8080>** in your browser. The web UI is the primary way to
manage and control mtrack.

## Dashboard

The dashboard is your main control surface:

- **Playback controls** — Play, stop, previous, next
- **Playlist selector** — Switch between playlists or view all songs
- **Song progress** — Current position and duration
- **Lock mode** — Prevent accidental configuration changes during a show

## Navigation

The nav bar links to:

- **Songs** — Browse, create, import, and edit songs. See [Importing Songs](importing-songs.md).
- **Playlists** — Create and manage setlists. See [Playlists](playlists.md).
- **Config** — Configure audio, MIDI, lighting, and controllers. See [Hardware Configuration](hardware-config.md).
- **Status** — View connected devices, controller status, and system health.

## Next Steps

- **Add songs** — Import existing audio files or create songs from scratch. See [Importing Songs](importing-songs.md).
- **Configure hardware** — Set up your audio interface, MIDI devices, and lighting. See [Hardware Configuration](hardware-config.md).
- **Build a setlist** — Create playlists for your shows. See [Playlists](playlists.md).
