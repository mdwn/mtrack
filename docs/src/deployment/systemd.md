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

Next, generate and install the systemd service file:

```
$ sudo mtrack systemd > /etc/systemd/system/mtrack.service
```

The service expects that `mtrack` is available at the location `/usr/local/bin/mtrack`. It also
expects you to define your player configuration in `/etc/default/mtrack`. This file
should contain one variable: `MTRACK_CONFIG`:

```
# The configuration for the mtrack player.
MTRACK_CONFIG=/mnt/storage/mtrack.yaml
```

Make sure the `mtrack` user can read your configuration and song files:

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
