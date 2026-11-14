# Service Hardening

The generated systemd service includes security hardening that runs `mtrack` with minimal
privileges. This is the recommended configuration for production deployments.

**User isolation**: The service runs as the unprivileged `mtrack` user instead of root. The
`audio` supplementary group provides access to ALSA and MIDI devices under `/dev/snd/`.

**Real-time scheduling**: `AmbientCapabilities=CAP_SYS_NICE` allows the `mtrack` user
to set elevated thread priorities and use `SCHED_FIFO` real-time scheduling for the audio
callback thread and the MIDI beat clock thread, without requiring root.
`CapabilityBoundingSet=CAP_SYS_NICE` ensures this is the only capability the process can
ever acquire. Without this capability, the beat clock will still function but may exhibit
more timing jitter under heavy system load.

**Filesystem restrictions**: generated with your writable directories —
`mtrack systemd /var/lib/mtrack` — the unit sets `ProtectSystem=strict`, making the
whole filesystem read-only except `/dev`, `/proc` and `/sys`, and excepts exactly
the directories you named with `ReadWritePaths`. Those are where mtrack writes
configuration, songs, playlists and lighting files.

Generated with no path it falls back to `ProtectSystem=full`, which makes `/usr`,
`/boot` and `/efi` read-only and leaves everything else writable. That is weaker,
and it is the fallback only because a unit that cannot name the directories to
except cannot safely make the rest read-only.

If a directory that mtrack writes is not listed — a `songs:` or `playlists_dir:`
pointing outside the library, for instance — the service starts and then fails
with `Read-only file system (os error 30)`. List every such directory when
generating the unit.

Logs are emitted to stdout/stderr and captured by journald. `PrivateTmp=true`
provides an isolated temporary directory.

**Kernel restrictions**: The service cannot modify kernel tunables (`ProtectKernelTunables`),
load kernel modules (`ProtectKernelModules`), access the kernel log buffer
(`ProtectKernelLogs`), or modify control groups (`ProtectControlGroups`).

**Additional hardening**: The service is further restricted with `NoNewPrivileges` (cannot
gain new privileges via setuid/setgid binaries or filesystem capabilities),
`MemoryDenyWriteExecute` (no writable-executable memory pages), `SystemCallArchitectures=native`
(only native architecture syscalls), `LockPersonality` (cannot change execution domain),
`RestrictNamespaces` (cannot create user/network/mount namespaces), and
`RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX AF_NETLINK` (only IPv4, IPv6, Unix socket
and netlink access; netlink is how the player reads its own network addresses, and without
`CAP_NET_ADMIN` it cannot change them).

## Network exposure

The sandbox above limits what the process can do on the machine. It does nothing about who
can reach it over the network, and mtrack has no user accounts: whoever can open a port can use
it. What listens, and where:

| Service | Listens on | Default | Access control |
|---|---|---|---|
| web UI (`mtrack start`) | `--web-address`, all interfaces (`0.0.0.0`) | TCP 8080 (`--web-port`) | None. Lock mode only guards against accidental edits (track gains still save while locked), and the lock itself can be toggled from any browser that reaches the page. It also carries the bytes of files (`PUT /uploads/<id>`, `GET /downloads/<id>`): an upload id is 128 random bits issued by a call that obeys the lock, and the uploads and downloads expire after ten minutes. It also serves the lighting asset store's models and thumbnails (`GET /lighting-assets/...`, read-only and open while locked). The gRPC port serves none of these three. There is no other HTTP API: every call is gRPC-Web, behind the lock. When the web UI cannot start, the player runs unlocked. |
| gRPC controller | `bind_address`, all interfaces (`0.0.0.0`) | TCP 43234 (`port`) | None. Full transport and configuration control, including the `mtrack` CLI's remote commands. Lock mode refuses its config writes; track gains still save. |
| OSC controller | all interfaces | UDP 43235 (`port`) | None. Transport control; status is broadcast to `broadcast_addresses`. |
| MCP controller | `bind_address`, localhost only (`127.0.0.1`) | TCP 43237 (`port`) | Optional `bearer_token`; set one whenever `bind_address` is not localhost ([MCP](../interfaces/mcp.md)). Lock mode refuses its writes. |

The gRPC, OSC and MCP controllers listen only when the profile's `controllers:` list configures
them ([Hardware Profiles](../configuration/hardware-profiles.md)). The web UI listens whenever
`mtrack start` runs. olad's own web server (TCP 9090) is on the same machine too, and is olad's
to secure.

So treat the machine as the trust boundary and put it on a network you control: a dedicated
show Wi‑Fi or a phone's hotspot, a wired link to the laptop, or a venue VLAN with nothing else
on it. On a shared network, firewall the ports to the addresses that need them, or bind the web
UI to the loopback (`mtrack start --web-address 127.0.0.1`) and reach it through an SSH tunnel.
Do not forward any of these ports to the internet.

**Troubleshooting**: If `mtrack` cannot access your audio or MIDI devices after setup, verify
group membership with `groups mtrack` and check device permissions with
`ls -la /dev/snd/`. If you encounter permission errors related to a specific restriction,
you can override individual directives by creating a drop-in:

```
$ sudo systemctl edit mtrack
```

```ini
# For example, to disable memory execution restrictions if a dependency requires it:
[Service]
MemoryDenyWriteExecute=false
```
