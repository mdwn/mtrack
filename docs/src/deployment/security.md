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

**Filesystem restrictions**: `ProtectSystem=full` makes `/usr`, `/boot`, and `/efi`
read-only while leaving other paths writable for the service user. This allows mtrack
to write configuration, songs, playlists, and lighting files to the project directory.
Logs are emitted to stdout/stderr and captured by journald. `PrivateTmp=true` provides
an isolated temporary directory.

**Kernel restrictions**: The service cannot modify kernel tunables (`ProtectKernelTunables`),
load kernel modules (`ProtectKernelModules`), access the kernel log buffer
(`ProtectKernelLogs`), or modify control groups (`ProtectControlGroups`).

**Additional hardening**: The service is further restricted with `NoNewPrivileges` (cannot
gain new privileges via setuid/setgid binaries or filesystem capabilities),
`MemoryDenyWriteExecute` (no writable-executable memory pages), `SystemCallArchitectures=native`
(only native architecture syscalls), `LockPersonality` (cannot change execution domain),
`RestrictNamespaces` (cannot create user/network/mount namespaces), and
`RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX` (only IPv4, IPv6, and Unix socket access).

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
