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

Next, generate and install the systemd service file. Pass your project directory
and the generated unit will name it in that reminder and declare it as a
writable path:

```
$ sudo mtrack systemd /mnt/storage > /etc/systemd/system/mtrack.service
```

The path is optional — without it the unit is identical apart from a generic
reminder in place of the specific one.

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
