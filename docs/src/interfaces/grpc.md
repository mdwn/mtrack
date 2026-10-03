# gRPC Control

The player exposes a gRPC service, `PlayerService`, defined in
[`player.proto`](https://github.com/mdwn/mtrack/blob/main/src/proto/player/v1/player.proto).
The gRPC controller listens on port 43234 unless the profile's `grpc` controller sets another
`port` (see [Player Configuration](../configuration/player-config.md)).

Any number of clients can connect, including clients on other machines. That makes it a
fallback for when a MIDI controller is not behaving: you can still start, stop and move
through the playlist from a laptop or phone.

## The `mtrack` client

The `mtrack` command itself has subcommands that call the running player over gRPC:

```
$ mtrack play
$ mtrack play --from "1:23.456"  # Start playback from a specific time
$ mtrack seek "1:23.456"  # Jump to a time in the current song (all subsystems resync)
$ mtrack seek chorus      # Jump to the start of a named section
$ mtrack previous
$ mtrack next
$ mtrack stop
$ mtrack switch-to-playlist all_songs
$ mtrack switch-to-playlist my-playlist
$ mtrack status
$ mtrack active-effects  # Print all active lighting effects
$ mtrack cues  # List all cues in the current song's lighting timeline
```

Every one of these takes `--host-port <host:port>` (short form `-H`) to name the server. Without
it, the client connects to `127.0.0.1:43234`, so on the player's own machine no option is needed:

```
$ mtrack status -H 192.168.1.50:43234
```

`switch-to-playlist` accepts `all_songs` or the name of any playlist loaded from the config's
`playlists_dir`. The player refuses to switch while a song is playing.

The [command-line reference](../reference/cli.md) lists these alongside the other commands.

## RPCs

The service has 27 RPCs. The "CLI" column names the `mtrack` subcommand that calls it; an
empty cell means the RPC is available to gRPC clients only.

| RPC | Purpose | CLI |
|---|---|---|
| `Play` | Plays the current song in the playlist, if nothing is playing. | `play` |
| `PlayFrom` | Plays the current song starting at a time. | `play --from` |
| `PlaySongFrom` | Plays a specific song starting at a time. | |
| `Seek` | Jumps to a position in the current song. While playing, everything resyncs at the position; while stopped, the position is used by the next play. | `seek <time>` |
| `SeekToSection` | Jumps to the start of a named section of the current song. | `seek <section>` |
| `Previous` | Moves the playlist to the previous song. | `previous` |
| `Next` | Moves the playlist to the next song. | `next` |
| `Stop` | Stops the playing song. | `stop` |
| `SwitchToPlaylist` | Switches the active playlist. | `switch-to-playlist` |
| `Status` | Returns the player's current status. | `status` |
| `GetCues` | Lists the cues in the current song's lighting timeline. | `cues` |
| `GetActiveEffects` | Returns the running lighting effects as a formatted string. | `active-effects` |
| `StopSamples` | Stops all triggered samples that are playing. | |
| `LoopSection` | Activates a loop on a named section. | |
| `StopSectionLoop` | Ends the section loop; the current pass finishes and the song continues from the section end. | |
| `SectionAck` | Acknowledges the current section in reactive looping mode, arming the loop so it engages at the section end. | |
| `SetTrackGain` | Sets the gain of one output track, in dB. | |
| `SetTrackMute` | Mutes or unmutes an output track without changing its gain. Mute state is not persisted. | |
| `GetTrackGains` | Returns the gains of all output tracks, in dB. | |
| `GetConfig` | Returns the current configuration as YAML with a checksum. | |
| `UpdateAudio` | Updates the audio configuration section. | |
| `UpdateMidi` | Updates the MIDI configuration section. | |
| `UpdateDmx` | Updates the DMX configuration section. | |
| `UpdateControllers` | Updates the controllers configuration. | |
| `AddProfile` | Adds a hardware profile. | |
| `UpdateProfile` | Updates the profile at an index. | |
| `RemoveProfile` | Removes the profile at an index. | |

## Security

The gRPC server has no authentication. Do not run the player on a public network, and disable
the gRPC controller if the network the player is on is open to people you do not trust. See
[Security](../deployment/security.md).
