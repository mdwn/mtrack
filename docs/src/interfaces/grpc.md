# gRPC Control

The player can now be controlled using gRPC calls. The definition for the service can be found
[here](https://github.com/mdwn/mtrack/blob/main/src/proto/player/v1/player.proto). By default, this runs on port 43234.

The `mtrack` command itself supports several subcommands for gRPC interaction of the running
player:

```
$ mtrack play
$ mtrack play --from "1:23.456"  # Start playback from a specific time
$ mtrack previous
$ mtrack next
$ mtrack stop
$ mtrack switch-to-playlist all_songs|playlist
$ mtrack status
$ mtrack active-effects  # Print all active lighting effects
$ mtrack cues  # List all cues in the current song's lighting timeline
$ mtrack loop-section "verse"  # Activate section loop during playback
$ mtrack stop-section-loop     # Stop the active section loop
```

This will allow for multiple, arbitrary connections to the player, potentially from clients
outside the device the player is running on. It should also be handy for "oh crap" moments at
gigs when your MIDI controller isn't behaving well. While not ideal, you'll still at least
be able to control the player.

One note: there is no security on this at present. I don't advise running `mtrack` on a public
network to begin with, but I would advise disabling the gRPC server if for some reason the
network the player is running on is wide open.
