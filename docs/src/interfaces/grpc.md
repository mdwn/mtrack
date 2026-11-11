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

The service has 33 RPCs. The "CLI" column names the `mtrack` subcommand that calls it; an
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
| `NavigateToSong` | Makes an entry of the active playlist the current one without playing it. `position` is the entry's index as `GetPlaylist` lists it, so a song listed twice is reached at either; `song_name`, when set, is the song the caller believes is there, and another song there (the playlist changed) is `FAILED_PRECONDITION` with nothing moved. A `position` past the end is `OUT_OF_RANGE`. Like `Next`, it does not move the playlist while a song plays, and answers with the song playing (unlike `Next`, it does not break a section loop). Already being on that entry changes nothing: no pause is called off and no waiting auto-play cancelled. | |
| `Stop` | Stops the playing song. | `stop` |
| `SwitchToPlaylist` | Switches the active playlist. | `switch-to-playlist` |
| `Status` | Returns the player's current status: the playlist and song, whether it plays and how far in, the audio output's health, the [ambient light](../lighting/configuration.md#ambient-mode)'s `mode`, `state` and `show` (absent without a DMX engine; when the status's read of the DMX engine did not answer, `available` is false, `reason` says why, and `mode` and `state` are empty), and a song waiting to [auto play](../getting-started/playlists.md#auto-play) (`auto_play`: its `song` and `starts_in`). | `status` |
| `ListPlaylists` | Lists the playlists' names, sorted, and names the active one (`active`). | |
| `GetPlaylist` | Returns a playlist's entries (`song_name` and `duration`) and `position`, the index of the entry it is on (absent while the song playing is one an edit took off the list). An empty `name` is the active playlist; an unknown one is `NOT_FOUND`. | |
| `SavePlaylist` | Sets a playlist's songs and their order, as the web UI's save does when it sends only the songs: an existing file keeps each entry's own settings with its song, and the playlist's, and a song listed twice stays listed twice. `create` makes a new playlist and refuses (`ALREADY_EXISTS`) one that exists; without it, a playlist that does not exist is `NOT_FOUND`. A name the player refuses (`all_songs`, or one that is empty or not a plain name) or a song it does not have is `INVALID_ARGUMENT`; a file that does not read is `FAILED_PRECONDITION`, not written over. With `check_expected` set, `expected` is the songs the caller last saw, and a file that differs is `FAILED_PRECONDITION` with a message starting "the playlist has changed". Answers with the playlist as `GetPlaylist` now reports it. Refused while the player is locked; allowed while a song plays. | |
| `DeletePlaylist` | Deletes a playlist's file, as the web UI does: an unknown name is `NOT_FOUND`, `all_songs` is `INVALID_ARGUMENT`, and the active playlist, if it was that one, falls back to `all_songs`. Refused while the player is locked. | |
| `SetLock` | Locks (`locked: true`) or unlocks the player, as the web UI's lock does, and answers with the lock as it now stands. Open while locked. | |
| `WatchStage` | A stream of what the stage looks like, for a client that draws it: first a `StageLayout` (every fixture by name with its type, tags, position, rotation, capabilities, cells and channel names, where it points, and the venue's name, focus points and scenery), then a `StageFrame` whenever something changes. A frame is read against the most recent layout and nothing else, never against the frame before it. Each layout has an `id` (1 for the stream's first, counting up) and each frame names its layout's in `layout`, so a client drops a frame whose `layout` is not the id of the layout it holds. `channels` is every fixture's channel values end to end, in the layout's fixture order and each fixture's channel order, and strobe rates, per-cell values, poses that differ from the layout's and fixtures under test are listed by fixture index. A new layout is sent, with a frame after it, when the venue or a fixture type is reloaded, the hardware is, or the fixtures or their channels change. `max_rate` is the most frames a second to send (0 for the player's own rate, 20 a second); frames that arrive faster are dropped and the newest sent. A player with no lighting sends an empty layout and an empty frame and keeps the stream open. Open while locked. | |
| `GetLightingAsset` | A stream of one file of the lighting asset store (the stage layout's rig description, models and scenery), as the web UI's `/api/lighting/assets/` serves it, for a client that cannot fetch it over HTTP. `path` is the path under `/api/lighting/assets/`. The file arrives as `AssetChunk`s of at most 8 KiB that join, in order, into the file; the first carries `total_size`, the bytes all the chunks carry. With `gzip` the whole file is compressed first and the chunks join into a gzip stream. A path that is not in the store, not a served type (`json`, `glb`, `png`, `svg`) or over 64 MiB is `NOT_FOUND`; one that leaves the store is `INVALID_ARGUMENT`. Open while locked. | |
| `GetCues` | Lists the cues in the current song's lighting timeline. | `cues` |
| `GetActiveEffects` | Returns the running lighting effects as a formatted string. | `active-effects` |
| `StopSamples` | Stops all triggered samples that are playing. | |
| `LoopSection` | Activates a loop on a named section. | |
| `StopSectionLoop` | Ends the section loop; the current pass finishes and the song continues from the section end. | |
| `SectionAck` | Acknowledges the current section in reactive looping mode, arming the loop so it engages at the section end. | |
| `SetTrackGain` | Sets the gain of one output track, in dB. | |
| `SetTrackMute` | Mutes or unmutes an output track without changing its gain. Mute state is not persisted. | |
| `GetTrackGains` | Returns the gains of all output tracks, in dB. | |
| `SetAmbientMode` | Sets the [ambient mode](../lighting/configuration.md#ambient-mode): `auto`, `on` or `off`, in any case. Another name is `INVALID_ARGUMENT`. Returns the ambient status after the change. | |
| `CancelAutoPlay` | Calls off a song waiting to [auto play](../getting-started/playlists.md#auto-play), leaving the player stopped on it with ambient light up. `cancelled` is false when none was waiting. | |
| `GetConfig` | Returns the current configuration as YAML with a checksum. | |
| `UpdateAudio` | Updates the audio configuration section. | |
| `UpdateMidi` | Updates the MIDI configuration section. | |
| `UpdateDmx` | Updates the DMX configuration section. | |
| `UpdateControllers` | Updates the controllers configuration. | |
| `AddProfile` | Adds a hardware profile. | |
| `UpdateProfile` | Updates the profile at an index. | |
| `RemoveProfile` | Removes the profile at an index. | |

## Lock mode

While the player is [locked](web-ui.md#lock-mode), the config writes (`UpdateAudio`,
`UpdateMidi`, `UpdateDmx`, `UpdateControllers`, `AddProfile`, `UpdateProfile` and
`RemoveProfile`) are refused with `FAILED_PRECONDITION` and the message "Player is locked.
Unlock to make changes." The other RPCs work while locked. `SetTrackGain` is a performance
control: a fader move still saves the track's gain to the active profile in `mtrack.yaml`,
after a short pause, while locked. `SavePlaylist` and `DeletePlaylist` are playlist edits and are
refused too. Unlock from the web UI, or with `SetLock`, which is open while locked. When the web UI cannot start, the
player runs unlocked, since the web UI is how the lock is normally released.

## Security

The gRPC server has no authentication. Do not run the player on a public network, and disable
the gRPC controller if the network the player is on is open to people you do not trust. See
[Security](../deployment/security.md).
