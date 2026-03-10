# Terminal UI (TUI)

`mtrack start` can optionally launch a terminal-based user interface built with
[ratatui](https://ratatui.rs/) using the `--tui` flag. The TUI provides a complete view of the
player state without requiring any external clients:

- **Playlist panel**: Shows the current playlist with the selected song highlighted.
- **Now Playing panel**: Displays the current song name, a progress bar with elapsed/total
  time, and the track listing.
- **Fixtures panel**: Shows real-time fixture colors from the lighting engine (when DMX is
  configured).
- **Active Effects panel**: Lists currently running lighting effects.
- **Log panel**: Displays tracing output (INFO/WARN/ERROR) inline, color-coded by severity.
- **Key hints bar**: Shows available keyboard shortcuts.

## Keyboard controls

| Key | Action |
|-----|--------|
| `Space` / `Enter` | Play / Stop |
| `←` / `→` or `p` / `n` | Previous / Next song |
| `a` | Switch to all songs |
| `l` | Switch to playlist |
| `q` / `Esc` | Quit |

## Enabling the TUI

Enable the TUI with the `--tui` flag:

```
$ mtrack start /path/to/player.yaml --tui
```

Without `--tui`, `mtrack` runs in headless mode with log output to stderr. The TUI runs
alongside all configured controllers (gRPC, OSC, MIDI) and the web UI, so you can use
the keyboard, browser, and remote control simultaneously.
