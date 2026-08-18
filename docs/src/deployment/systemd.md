# Running on Startup

To have `mtrack` start when the system starts, first create a dedicated system user for the service:

```
$ sudo useradd --system --no-create-home --shell /usr/sbin/nologin mtrack
$ sudo usermod -aG audio mtrack
```

The `audio` group grants access to ALSA sound cards and MIDI devices. If your DMX USB adapter
requires a specific group (e.g. `plugdev` or `dialout`), add that as well:

```
$ sudo usermod -aG plugdev mtrack
```

That user also needs read and write access to your project directory. `mtrack`
writes configuration, songs, playlists and lighting files there, and the user you
just created owns none of it:

```
$ sudo chown -R mtrack:mtrack /mnt/storage
```

Add it to a group that already owns the directory instead, if you would rather
not change the ownership. Skipping this step is the most common reason the
service starts and then fails with permission errors that do not obviously point
at permissions.

Next, generate and install the systemd service file. Pass your project
directory:

```
$ sudo mtrack systemd /mnt/storage > /etc/systemd/system/mtrack.service
```

Passing it buys you the stricter sandbox. The unit then sets
`ProtectSystem=strict` — the whole filesystem read-only except `/dev`, `/proc`
and `/sys` — and excepts your project directory with
`ReadWritePaths`, which is the one path `mtrack` has to write.

The path is optional, and without it the unit falls back to
`ProtectSystem=full`: `/usr`, `/boot` and `/efi` read-only, everything else
writable. That is weaker, and it is the fallback only because a unit that cannot
name the directory to except cannot safely make the rest read-only. A service
generated that way still runs; it is simply less contained.

Note that neither setting grants the `mtrack` user permission to write your
library — that is the `chown` above, and it is required either way. If the
service fails to start with `Read-only file system (os error 30)`, the sandbox
is the cause; if it fails with a permission error naming a file, the ownership
is.

The service expects that `mtrack` is available at the location `/usr/local/bin/mtrack`. It also
expects you to define your project directory in `/etc/default/mtrack`. This file
should contain one variable: `MTRACK_PATH`:

```
# The project directory for mtrack (contains songs, config, playlists, lighting).
MTRACK_PATH=/mnt/storage
```

Make sure the `mtrack` user has read **and write** access to the project directory so the
web UI can manage configuration, songs, playlists, and lighting files:

```
$ sudo chown -R mtrack:mtrack /mnt/storage
```

Once that's defined, you can start it with:

```
$ sudo systemctl daemon-reload
$ sudo systemctl enable mtrack
$ sudo systemctl start mtrack
```

It will now be running and will restart when you reboot your machine. You'll be able to view the logs
for `mtrack` by running:

```
$ journalctl -u mtrack
```
