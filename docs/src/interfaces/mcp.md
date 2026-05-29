# MCP Control

mtrack can expose its running player and project files to [Model Context
Protocol](https://modelcontextprotocol.io) (MCP) clients — such as Claude Desktop or Claude
Code — letting an AI assistant inspect playback, drive the player, and author configuration and
lighting shows on your behalf.

The MCP server is enabled by adding an `mcp` controller to the
[player configuration](../configuration/player-config.md). It runs over a Streamable HTTP
transport mounted at `/mcp` on its own listener, kept separate from the web UI so the two can be
enabled, disabled, and bound independently.

## Configuration

```yaml
controllers:
  - kind: mcp
    # Port to listen on. Defaults to 43237.
    port: 43237

    # Bind address. Defaults to 127.0.0.1 (localhost-only).
    # Set to 0.0.0.0 to expose the server on the network.
    bind_address: 127.0.0.1

    # Optional bearer token. When set, every request must carry an
    # `Authorization: Bearer <token>` header or it is rejected with HTTP 401.
    # Strongly recommended whenever bind_address is not localhost.
    # bearer_token: "your-secret-token"

    # Idle session timeout in seconds. A session whose inbound and outbound
    # traffic is quiet for this long is closed and its resources released.
    # Defaults to 14400 (4 hours). Set to null to disable eviction.
    # idle_session_timeout_secs: 14400
```

The endpoint is then reachable at `http://<bind_address>:<port>/mcp`.

## Connecting a client

For a local client such as Claude Code, point it at the HTTP endpoint:

```
claude mcp add --transport http mtrack http://127.0.0.1:43237/mcp
```

If a `bearer_token` is configured, supply it as an `Authorization: Bearer <token>` header in your
client's MCP server settings.

## What it exposes

### Tools

Roughly 45 tools are available. They fall into a few groups:

- **Status & discovery** — current playback status, host/runtime info, and listings of songs,
  playlists, groups, venues, and fixture types.
- **Playback control** — play, stop, next/previous, play-from-a-time, play-a-named-song-from-a-time,
  switch playlist, stop triggered samples, and section-loop control (loop a section, stop the loop,
  acknowledge the current section in reactive looping).
- **Configuration editing** — read the full config and update the `audio`, `midi`, `dmx`, and
  `controllers` subsections, plus add / update / remove hardware profiles.
- **Song & playlist authoring** — read, write, and patch `song.yaml` and playlist files, plus
  detailed song metadata and beat-grid queries.
- **Lighting authoring** — read, write, validate, and patch `.light` DSL files for songs, venues,
  and fixture types, list the lighting cues and active effects, and fetch a DSL reference primer.

Every write and patch tool validates its input (YAML schema or `.light` DSL parse) **before** it
writes to disk, so a malformed edit is rejected rather than corrupting a project file.

### Resources

Two resources can be subscribed to (via `resources/subscribe`) for live push updates:

- `mtrack://status` — a snapshot of player state (active playlist, current song, playback position).
- `mtrack://config` — the current configuration as YAML, plus a checksum.

## Security

There is no authentication unless you configure a `bearer_token`. The server binds to localhost by
default; if you change `bind_address` to expose it on a network, set a `bearer_token` so the surface
is not reachable unauthenticated. As with the other control interfaces, running mtrack on a wide-open
network is not advised — see [Service Hardening](../deployment/security.md).
