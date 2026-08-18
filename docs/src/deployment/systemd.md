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
and `/sys` — and excepts what you named with `ReadWritePaths`.

**List every directory mtrack writes, not just the library.** `songs`,
`playlists_dir`, `profiles_dir` and `samples` are used as given when they are
absolute paths, so a config with `songs: /mnt/nas/songs` writes outside the
project directory:

```
$ sudo mtrack systemd /mnt/storage /mnt/nas/songs > /etc/systemd/system/mtrack.service
```

A directory that is not listed is read-only, and the service fails on its first
write there with `Read-only file system (os error 30)`.

**Create the directories before you name them.** systemd bind-mounts every
`ReadWritePaths=` entry and cannot mount a path that does not exist. The
generated unit prefixes each entry with `-`, so a missing one is skipped rather
than fatal — but skipped means still read-only, and the service fails exactly as
it did before. An entry you add by hand without that prefix is worse: the unit
fails namespace setup and the service never starts, so there is no message
explaining why.

The generated unit also emits `RequiresMountsFor=` for these paths, so systemd waits for whatever
they live on before starting mtrack. Without that, a library on a USB stick or network share is
simply absent when the service starts, and `Restart=on-failure` exhausts the default start limit in
a few seconds — leaving the unit failed for a drive that appeared a moment later.

This works only for mounts systemd knows about: an fstab entry or a `.mount` unit. A drive mounted
on demand by udisks2 has no unit while it is unmounted, so there is nothing to wait for and the
service starts regardless. Give anything mtrack must not start without an fstab entry.

> **Upgrading an existing install: regenerate your unit.** mtrack no longer creates a configured
> directory that lies outside the project — see
> [Player Configuration](../configuration/player-config.md). If your `songs` (or `playlists_dir`,
> `profiles_dir`, or a lighting directory) points outside the project and might be absent at boot,
> because it lives on a drive that mounts late, startup now fails instead of quietly writing under
> the mount point. A unit generated before this change has neither the `RequiresMountsFor=` that
> waits for the drive nor the widened restart window that lets a late mount recover, so it can land
> in a permanently failed state. Regenerate it:
>
> ```
> $ sudo mtrack systemd /mnt/storage /mnt/nas/songs > /etc/systemd/system/mtrack.service
> $ sudo systemctl daemon-reload
> ```

These paths are baked into the unit when it is generated. They are not read from
`$MTRACK_PATH`, so if you move your library later, regenerate the unit as well as
editing `/etc/default/mtrack`.

The path is optional, and without it the unit falls back to
`ProtectSystem=full`: `/usr`, `/boot` and `/efi` read-only, everything else
writable. That is weaker, and it is the fallback only because a unit that cannot
name the directory to except cannot safely make the rest read-only. A service
generated that way still runs; it is simply less contained.

Note that neither setting grants the `mtrack` user permission to write your
library — that is the `chown` above, and it is required either way. If the
service fails to start with `Read-only file system (os error 30)`, the sandbox
is the cause; if it fails with a permission error naming a file, the ownership
is. mtrack says which in the journal when systemd started it, along with the
directory to add or to `chown`, so `journalctl -u mtrack` should tell you
without needing this page.

The service expects that `mtrack` is available at the location `/usr/local/bin/mtrack`. It also
expects you to define your project directory in `/etc/default/mtrack`. This file
should contain one variable: `MTRACK_PATH`:

```
# The project directory for mtrack (contains songs, config, playlists, lighting).
MTRACK_PATH=/mnt/storage
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
