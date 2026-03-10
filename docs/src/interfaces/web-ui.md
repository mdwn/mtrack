# Web UI

`mtrack` includes a web-based interface for controlling and monitoring the player from a browser.
The web UI is always available when running `mtrack start`, served on all interfaces at
port 8080 by default (`http://0.0.0.0:8080`).

Use `--web-port` and `--web-address` to customize:

```
$ mtrack start /path/to/player.yaml --web-port 9090 --web-address 127.0.0.1
```

The web UI provides:

- **Playback control** — Play/stop/next/prev with a progress bar showing elapsed and total time.
- **Playlist management** — View and switch between the playlist and all-songs list, select songs.
- **Waveform visualization** — Per-track waveform peak display for the current song.
- **Stage view** — Interactive canvas showing fixture positions organized by tags, with real-time
  DMX channel visualization and color rendering.
- **Active effects** — Lists currently running lighting effects.
- **Log panel** — Streaming application logs with auto-scroll.
- **Lighting simulator** — The stage view doubles as a lighting simulator, allowing you to preview
  and design light shows without physical DMX hardware. When the OLA daemon is unavailable, the
  DMX engine falls back to a null client so the effects engine can still run.

The web UI also exposes a REST API for managing configuration, playlists, songs, and lighting
files, as well as a WebSocket endpoint for real-time state streaming.
