# gRPC Control

The player exposes three gRPC services, `PlayerService` (what a performer or a remote uses during a
show, and the calls below), `ConfigService` (what the editor pages use; see
[ConfigService](#configservice)) and `LightingService` (the lighting pages' calls; see
[LightingService](#lightingservice)), defined in
[`player.proto`](https://github.com/mdwn/mtrack/blob/main/src/proto/player/v1/player.proto).
The gRPC controller listens on port 43234 unless the profile's `grpc` controller sets another
`port` (see [Player Configuration](../configuration/player-config.md)). It listens on every
interface unless the controller sets `bind_address`: `127.0.0.1` keeps it to the player's own
machine, where the `mtrack` CLI still reaches it, the phone app does not connect over Wi-Fi, and
`GetNetworkInfo` names no port.

Any number of clients can connect, including clients on other machines. That makes it a
fallback for when a MIDI controller is not behaving: you can still start, stop and move
through the playlist from a laptop or phone. The [phone app](bluetooth.md#over-wi-fi) uses this
port over Wi-Fi, so the gRPC controller has to be enabled for it; there is no login on it, so
anyone on the player's network can connect.

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
| `ListPlaylists` | Lists the playlists' names, sorted, and names the active one (`active`). With `detailed: true` it also answers `playlists`, each playlist with its `song_count` and whether it is the player's own `all_songs` (`reserved`), as the web UI's list shows them. | |
| `GetPlaylist` | Returns a playlist's entries (`song_name`, `duration` and `settings`) and `position`, the index of the entry it is on (absent while the song playing is one an edit took off the list). An entry's `settings` are its fades, auto play and ambient light as the playlist file has them, empty when it sets none; `ambient` is the playlist's own ambient light, absent when it leaves it to the profile. Lengths are the player's own strings (`2beats`, `1s`), and a fade carries all it has (`curve`, `exclude`, `lights`). `revision` is a short fingerprint of the songs and all those settings, for `SavePlaylist`. An empty `name` is the active playlist; an unknown one is `NOT_FOUND`. With `with_library: true` it also answers `available_songs` (the songs the player has loaded, sorted) and `song_tracks` (each one's tracks), which the web UI's playlist editor offers to add and to leave out of a fade. | |
| `SavePlaylist` | Sets a playlist's songs and their order, as the web UI's save does when it sends only the songs: an existing file keeps each entry's own settings with its song, and the playlist's, and a song listed twice stays listed twice. `create` makes a new playlist and refuses (`ALREADY_EXISTS`) one that exists; without it, a playlist that does not exist is `NOT_FOUND`. A name the player refuses (`all_songs`, or one that is empty or not a plain name) or a song it does not have is `INVALID_ARGUMENT`; a file that does not read is `FAILED_PRECONDITION`, not written over. With `set_settings`, `settings` (one per song, in order) replaces each entry's settings, and with `set_ambient`, `ambient` replaces the playlist's (absent: the profile's); without them a save keeps the settings as described. Settings are checked as the web UI's save checks them, and one the player would not load (a length or gap it cannot read, a fade longer than the song it plays under, a track the song does not have, an ambient show outside the project) is `INVALID_ARGUMENT` with a message saying which. With `check_expected` set, `expected` is the songs the caller last saw, and `expected_revision`, when not empty, the `revision` `GetPlaylist` gave it; a file that differs in either is `FAILED_PRECONDITION` with a message starting "the playlist has changed". Answers with the playlist as `GetPlaylist` now reports it. Refused while the player is locked; allowed while a song plays. | |
| `ListLightShows` | Lists the `.light` files an ambient light can play, as paths relative to the project (how a playlist's `ambient.show` is written), sorted: those under the songs directory, as the web UI's lighting list finds them, and those in the project's `lighting/` directory. Open while locked. | |
| `ListSongs` | A page of the song library: every song, loaded or failed, sorted by name, each with its `duration` (absent for a song that failed), whether it `loops`, `has_lighting`, `has_midi` or `has_samples`, `failure`, why it failed to load (cut to 200 characters), and `base_dir`, the song's directory relative to the songs directory with `/` between its parts, as the web UI's song list has it (`Live/2026/Encore`; empty for a song not under the songs directory). The web UI groups songs by `base_dir` less its last part, the songs at the top first. `base_dir` is absent from a player from before it was sent. `start` is where the page begins (0 first); `next`, when present, is where the next page begins, and `total` is how many songs there are. A page holds at most 48 KiB, so it fits one Bluetooth message. With `detailed: true` the answer is every song from `start` in one message, a failure's error in full, and each loaded song also says what the web UI's song pages show: its `channels`, `sample_format`, `tracks`, `lighting_files` and `midi_dmx_files` (relative to the songs directory), `beat_grid`, `has_tempo_map`, `loop_crossfade_ms` and `loop_crossfade_cut`, and `sections`; it is too large for Bluetooth, which is not asked for it. Open while locked. | |
| `GetSong` | A song's settings as its file has them: `looping` (`loop_playback`, `crossfade` as written, `cut`), `tracks` (name, file, `file_channel`), its own `samples` (file, output channels or track, release, retrigger, max voices, fade time, and how it answers `velocity`), its sample `triggers` in words, an `overview` of what else it has (tempo, sections, metronome, pilot hints, MIDI file, light shows), the audio `files` in its folder with their channel counts, `revision` (a fingerprint of the file), `failure` for a song that failed to load, `read_only` when its file cannot be changed (it has none, or it does not read), `playing` when it is the song playing, and `base_dir`, as `ListSongs` has it. An unknown name is `NOT_FOUND`. Open while locked. | |
| `UpdateSongSettings` | Changes a song's `looping`, `tracks` or `samples`, each group optional and left alone when absent: the tracks are the whole list in order, the samples the song's whole set, each naming the sample it was read as in `original_name` (empty for a new one). The song's file is read, only the keys whose values differ are changed, and it is written back as the web UI's song save writes it, then the songs are reloaded; a sample's velocity and every key the call does not name stay as they are. A track's or a sample's file must be one the song already names or an audio file in its folder. A change the song would not load with (a crossfade it cannot read, a channel the file does not have, a cut track it does not have), or one that would stop a playlist playing the song from loading, is `INVALID_ARGUMENT` with the loader's words; `expected_revision`, when not empty, refuses a file changed since it was read (`FAILED_PRECONDITION`, "the song has changed"). Answers with the song as `GetSong` now reports it. Refused while the player is locked. The song playing keeps the pass it started with; the change takes effect the next time it plays, except that whether it loops changes at once what Play and Next do. | |
| `DeletePlaylist` | Deletes a playlist's file, as the web UI does: an unknown name is `NOT_FOUND`, `all_songs` is `INVALID_ARGUMENT`, and the active playlist, if it was that one, falls back to `all_songs`. Refused while the player is locked. | |
| `SetLock` | Locks (`locked: true`) or unlocks the player, as the web UI's lock does, and answers with the lock as it now stands. Open while locked. | |
| `WatchStage` | A stream of what the stage looks like, for a client that draws it: first a `StageLayout` (every fixture by name with its type, tags, position, rotation, capabilities, cells and channel names, where it points, and the venue's name, focus points and scenery), then a `StageFrame` whenever something changes. A frame is read against the most recent layout and nothing else, never against the frame before it. Each layout has an `id` (1 for the stream's first, counting up) and each frame names its layout's in `layout`, so a client drops a frame whose `layout` is not the id of the layout it holds. `channels` is every fixture's channel values end to end, in the layout's fixture order and each fixture's channel order, and strobe rates, per-cell values, poses that differ from the layout's and fixtures under test are listed by fixture index. A new layout is sent, with a frame after it, when the venue or a fixture type is reloaded, the hardware is, or the fixtures or their channels change. `max_rate` is the most frames a second to send (0 for the player's own rate, 20 a second); frames that arrive faster are dropped and the newest sent. A player with no lighting sends an empty layout and an empty frame and keeps the stream open. Open while locked. | |
| `GetLightingAsset` | A stream of one file of the lighting asset store (the stage layout's rig description, models and scenery), as the web UI's `/lighting-assets/` serves it, for a client that cannot fetch it over HTTP. `path` is the path under `/lighting-assets/`. The file arrives as `AssetChunk`s of at most 8 KiB that join, in order, into the file; the first carries `total_size`, the bytes all the chunks carry. With `gzip` the whole file is compressed first and the chunks join into a gzip stream. A path that is not in the store, not a served type (`json`, `glb`, `png`, `svg`) or over 64 MiB is `NOT_FOUND`; one that leaves the store is `INVALID_ARGUMENT`. Open while locked. | |
| `WatchStatus` | A stream of the compact status the Bluetooth controller pushes to a phone, as a `RemoteStatus`: the playlist and song names, the song's length, whether it plays, the elapsed time, the playlist position, the song waiting to start by itself, the section a loop is set on, `gains_revision`, the ambient light's mode and state and `locked`. The current status comes at once, then another whenever anything but the elapsed time changes, and once a second while a song plays, so a client can run its own clock between them. A client that reads slowly is sent the latest status, not the ones it missed. The stream ends when the client goes away. Open while locked. | |
| `Watch` | One stream of everything the web UI's live feeds carry, as a `WatchEvent` that holds one of: `playback` (the playlist, the song's tracks with their gains, sections, beat grid and tempo, the section loop's state, what comes next and how it arrives, the ambient light and `locked`; about five times a second while someone watches), `logs` (the lines the player wrote since the last event), `waveform` (the peaks of the song the playlist is on, per track, when the song changes), `config_changed` (the config store changed), `reload` (a song's light show was reloaded from disk, with the error when it failed) and `hardware` (the build, every hardware subsystem, the controllers and `locked`, as the web UI's status page shows them). `topics` in the request picks the feeds (`PLAYBACK`, `LOGS`, `WAVEFORM`, `CONFIG_CHANGED`, `RELOAD`, `HARDWARE`); an empty list means all of them, so `grpcurl -plaintext -d '{}' host:43234 player.v1.PlayerService/Watch` shows everything. When the stream opens it sends the current playback, the cached waveform of the current song if there is one, and the hardware status (each if asked for); after that, each feed's news as it happens, and the hardware status again whenever it changes (it is compared once a second). A client that reads slowly skips what it missed and the stream goes on. Where `WatchStatus` is the compact status for a phone, `Watch` carries the whole dashboard: it is the one stream the web UI reads over gRPC-Web, in place of its former websocket and status poll. It is not allowed over Bluetooth. Open while locked. | |
| `GetNetworkInfo` | Where the player is on the network: its `hostname`, its `addresses` (IPv4, then IPv6, without loopback or link-local ones) and `grpc_port`, the port the gRPC controller is listening on (absent when none is configured). A phone connected over Bluetooth asks it to learn where to find the player over Wi-Fi. Open while locked. | |
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
| `GetConfig` | Returns the current configuration as YAML with a checksum, which a write presents as its `expected_checksum`. Open while locked. A player with no config store answers `UNIMPLEMENTED`. | |
| `UpdateAudio` | Updates the audio configuration section. | |
| `UpdateMidi` | Updates the MIDI configuration section. | |
| `UpdateDmx` | Updates the DMX configuration section. | |
| `UpdateControllers` | Updates the controllers configuration. | |
| `AddProfile` | Adds a hardware profile, given as a JSON string (`profile_json`). | |
| `UpdateProfile` | Updates the profile at an index. A section `profile_json` leaves out keeps its current value; one sent as `null` is removed. | |
| `RemoveProfile` | Removes the profile at an index. | |

Each config write sends the `expected_checksum` it read with `GetConfig`; one that is no longer the
config's (someone else changed it, or none was sent) is `FAILED_PRECONDITION`, a change that would
not validate, or an index that is not there, is `INVALID_ARGUMENT`, and the config is left as it
was. The answer, `UpdateConfigResponse`, is the config after the change (`yaml` and `checksum`) and
what became of the hardware reload: `applied`, and `reload` (`applied`, `deferred` or `failed`) with
a `message` when it is not applied. A change made while a song plays is saved and its reload
deferred until the player is idle; `awaits_reload` is then the generation of the reload it waits
for, which has started once the `Watch` stream's `hardware_reload_started` reaches it, and
`awaits_process` the process that counts it (a restarted mtrack counts from none, and has loaded the
saved config). `songs_error` says why the songs could not be reloaded beside it, when they could
not.

## ConfigService

`player.v1.ConfigService` is served on the same port as `PlayerService` and by the web UI's
gRPC-Web, behind the same [lock](#lock-mode). It is listed in reflection and reported by the health
service, and is not reachable over Bluetooth. The web UI's config editor calls it for the samples,
the metronome defaults, the hardware profile files of the `profiles_dir`, the audio and MIDI
device lists, the audio device test, the trigger calibration, the restart of the controllers, and
the Bluetooth pairing;
the song pages and the import pages call it for a song's config, its files, its waveform and tempo
guess, and for browsing the project and importing songs from it; and the pages that take a file
call it to announce the upload and to use it (see [Files](#files)).

| RPC | Description |
|---|---|
| `UpdateSamples` | Replaces the sample definitions (`samples_json`, a JSON object keyed by sample name) and the player-wide voice limit (`max_sample_voices`, absent for none). Answers `UpdateConfigResponse`, as the other config writes do. |
| `UpdateMetronome` | Replaces the player-wide metronome defaults (`metronome_json`; empty or `null` removes them), applies the change and reloads the songs, which take the default from it. A song reload that fails is answered in `songs_error` beside the save. Defaults that are not valid are `INVALID_ARGUMENT`. |
| `ListProfileFiles` | Lists the profile files of the config's `profiles_dir`, sorted by stem and file name, each with its hostname and which sections it has. `file` is the address (two files may share a stem); `addressable` is false for a name the other calls cannot take back as written; `error` says why a file does not parse (it is still listed). Open while locked. |
| `GetProfileFile` | Reads a profile file by full file name (`stage.yml`) or stem (`stage`: its `.yaml`, else its `.yml`): the profile (`profile_json`), the file as written (`yaml`) and its `version`. `NOT_FOUND` for no such file. Open while locked. |
| `SaveProfileFile` | Creates or updates a profile file. Sections `profile_json` leaves out keep their values in the file, `null` removes one, and keys the profile does not model are kept. `expected_version` is the `version` the file was read at (absent saves whatever is there); `create` makes the save only a create. The change goes into the running config and is applied; the answer, `ProfileFileSaved`, is the file's new `version` and the reload facts `UpdateConfigResponse` carries. |
| `DeleteProfileFile` | Deletes a profile file, drops it from the running config and applies the change. Answers `ProfileFileSaved` (no `version`). |
| `ListAudioDevices` | Lists the audio devices the player can see, each with its channel count (`channels_known` is false when that is the backend's clamp, not the hardware's), host, sample rates and formats. Open while locked. |
| `ProbeAudioDevice` | Opens a device (with the format settings of the request, as saving that profile would), confirms its output callback runs, and closes it again: silent, so safe with the PA up. Answers a `ProbeOutcome` (`STREAMING`, `OPENED_BUT_SILENT`, `COULD_NOT_OPEN` with a `reason`, `OPENED_CANNOT_REPORT`, or `IN_USE` for a device the player holds, which is answered from its health and not opened). A probe takes about two seconds, and probes run one at a time. `INVALID_ARGUMENT` for no device; `FAILED_PRECONDITION` during playback (the device being played through is still answered). Refused while locked. |
| `ListMidiDevices` | Lists the MIDI devices, or none when the MIDI subsystem is not available. Open while locked. |
| `StartCalibration` | Records a channel's noise for `duration` seconds (3 when absent, kept between 0.5 and 30), so the call takes that long, and answers its noise floor. `NOT_FOUND` for no such device; `INVALID_ARGUMENT` for a channel under 1 or above the device's, or a format it cannot stream; `UNAVAILABLE` when it would not open. Replaces any session open. Refused while locked. |
| `CaptureCalibration` | Starts recording the session's channel for hits (up to 60 seconds). `FAILED_PRECONDITION` without a session. Refused while locked. |
| `StopCalibration` | Ends the session and answers the channel's calibrated parameters. `FAILED_PRECONDITION` without a session or before a capture (the session is closed either way). Refused while locked. |
| `CancelCalibration` | Closes any session; nothing to cancel is not an error. Refused while locked. |
| `RestartControllers` | Restarts every controller from the current config and answers their status. `FAILED_PRECONDITION` during playback. Open while locked. |
| `GetSongConfig` | Reads a song's config file as written (`yaml`), from the file the song was loaded from. A song that is loaded but whose file is gone answers a `summary` of it instead. `NOT_FOUND` for no such song, or a failed one with no file. Open while locked. |
| `CreateSong` | Creates a song: a folder `name` under the songs directory (a nested song's has `/`) with a `song.yaml` of `yaml`, validated first, then reloads the songs. `ALREADY_EXISTS` when the song has a config already, or the name `yaml` declares is another song's (that name is in the `x-taken-name-bin` metadata); `FAILED_PRECONDITION` for a `song.yaml` beside it that is another song's. Refused while locked. |
| `SaveSongConfig` | Validates `yaml` and writes it to the file the song was loaded from, then reloads the songs. The song's folder must exist (`NOT_FOUND`). A config that is not valid is `INVALID_ARGUMENT`, with every problem in the message, separated by `; `. `ALREADY_EXISTS` for a rename onto another song's name (with `x-taken-name-bin`); `FAILED_PRECONDITION` when the file is another song's config. There is no revision to present: the last save wins. Refused while locked. |
| `DeleteSong` | Deletes a song's config file, leaving its audio, MIDI and lighting files, and takes the song off the playlists that name it. `NOT_FOUND` when there is none; `FAILED_PRECONDITION` for the song playing, or a file that is another song's. Refused while locked. |
| `GetSongWaveform` | A song's peaks, track by track (`Waveform`, the message `Watch` carries): at most 500 packed floats a track, whatever the song's length, so 16 tracks are about 32 KB. Computed on first ask and kept; the metronome and pilot tracks are rendered fresh each time. `NOT_FOUND` for no such song; `INVALID_ARGUMENT` for one that failed to load. Open while locked. |
| `GuessSongTempo` | A tempo map for a song, from its MIDI file's tempo events if it has one, else from its click track's beat grid: the guess (`tempo`), the `tempo:` block it would be (`config`), and its `source`. `NOT_FOUND` for no such song or one with neither source; `INVALID_ARGUMENT` for one that failed to load; `FAILED_PRECONDITION` for a beat grid with too few beats. Open while locked. |
| `ListSongFiles` | The files directly in a song's folder, sorted by name, each with its `FileKind`; the song's config is left out, and hidden files unless the config names them. `NOT_FOUND` / `INVALID_ARGUMENT` as `GuessSongTempo`. Open while locked. |
| `ImportSongFile` | Copies a file of the project (`path`, which must resolve to a file within the folder holding `mtrack.yaml`; anything else is `INVALID_ARGUMENT`) into the song's folder, as `filename` if given, and reloads the songs. A song with no folder yet is made, with its config. The name must be a safe file name of a type the player plays (audio, MIDI or `.light`, in any case). Answers the file's name and the name the song has, which is not the one asked for when that is taken. Refused while locked. |
| `BrowseDirectory` | Lists a folder of the project (`path`, relative to the folder holding `mtrack.yaml`; empty for the folder itself), directories first, then files by name, hidden ones left out. `NOT_FOUND` for a path that is not a folder within the project, symlinks that leave it included. Open while locked. |
| `CreateSongInDirectory` | Makes a song of a project folder: scans it for audio, MIDI and lighting files, splits stereo and multichannel files into a track per channel, writes the `song.yaml` and reloads the songs. `name` is optional (the folder's, past the names taken). `ALREADY_EXISTS` when the folder has a config or `name` is another song's (with `x-taken-name-bin`); `INVALID_ARGUMENT` for a path that is not a folder; `NOT_FOUND` for one outside the project. Refused while locked. |
| `BulkImportSongs` | Makes a song of each song folder below a project folder that has no config, and reloads the songs once. Folders with a config are `skipped`; a folder that fails is in `failed`, not an error. Errors as `CreateSongInDirectory`. Refused while locked. |
| `BeginUpload` | Announces a file about to be sent to the web server: its `purpose` (`TRACK` of a `song`, `SAMPLE`, `GDTF_IMPORT`, `MVR_INSPECT` or `MVR_IMPORT`), its `file_name` and its `size` in bytes (required). Answers an `upload_id` (128 random bits, hex) and the `path` to `PUT` the bytes to, `/uploads/<upload_id>`. The name is checked here by the rules the claiming call holds it to, which checks it again: a track's by a song's file name rules, a sample's by the sample rules, a GDTF's or an MVR's reduced to its final component (an MVR's must end in `.mvr`). A GDTF or an MVR may declare at most 272 MiB; a track or a sample has no cap beyond its declared size. `INVALID_ARGUMENT` for a purpose, name, song or size that is not valid; `RESOURCE_EXHAUSTED` when 32 uploads and staged downloads are pending. Every purpose but `MVR_INSPECT` (a pure read) is refused while locked, with the lock's `FAILED_PRECONDITION`. |
| `AddTrackFiles` | Adds the uploaded files (`upload_ids`, begun as `TRACK` of this `song`) to a song, which is made, with its config, when it is new, then reloads the songs. A file of that name already in the song is replaced (`replaced`). Answers the name the song has (a new one's is not the one asked for when that is taken) and each file. An id that is not known or has expired is `NOT_FOUND`; one for another purpose or song, not sent whole, or not as long as announced is `FAILED_PRECONDITION`; a name or file type that is not valid is `INVALID_ARGUMENT`. Each upload is spent whatever the outcome. Refused while locked. |
| `AddSampleFile` | Stores the uploaded file (`SAMPLE`) in the project's `samples/` directory, made if need be, and answers the `path` a sample definition names it by (`samples/<file>`). Errors as `AddTrackFiles`. Refused while locked. |
| `GetBluetooth` | The Bluetooth controller as it is now, as a `BluetoothStatus`: `configured` (the running config has a `ble` controller), `running` (it is up), `error` (why it is not running, or what the adapter refused when listing phones), the `pairing` window (`open`, `seconds_left`, rounded up, 0 when closed) and the paired `devices` (`address`, `name` when the phone gave one, `connected`). A device the owner trusts in BlueZ (a BLE MIDI pedal, say) is not listed. Open while locked. |
| `SetBluetoothPairing` | Opens the pairing window for `seconds` (120 when absent, kept between 10 and 600) or, with `open: false`, closes it, and answers the `BluetoothStatus` after. `FAILED_PRECONDITION` when Bluetooth is not running; `UNAVAILABLE` when the radio stack refuses. Refused while locked. |
| `ForgetBluetoothDevice` | Forgets a paired phone by `address` (it must pair again) and answers the `BluetoothStatus` after. `INVALID_ARGUMENT` for an address that is none; `NOT_FOUND` for one that is not a phone paired with the player (the owner's own trusted devices included); `FAILED_PRECONDITION` when Bluetooth is not running; `UNAVAILABLE` when the radio stack refuses. Refused while locked. |

The refusals are statuses: a version the file has since left, or a `create` onto a file that is
there (including one a stem reaches, `stage.yaml` with only `stage.yaml.yaml`), is `ABORTED` (the
first carries the file's current version in the `x-current-version` metadata); a new name that
differs from a file only in case is `ALREADY_EXISTS`; a name or a profile that is not valid,
including a name that would leave the directory, is `INVALID_ARGUMENT`; no such file is
`NOT_FOUND`; a config that sets no `profiles_dir` is `FAILED_PRECONDITION`; a player with no config
store answers the samples and metronome writes `UNIMPLEMENTED`. `UpdateSamples` and `UpdateMetronome`
send `expected_checksum` like the other config writes (a stale one is `FAILED_PRECONDITION`).

## LightingService

`player.v1.LightingService` is served on the same port as `PlayerService` and by the web UI's
gRPC-Web, behind the same [lock](#lock-mode). It is listed in reflection and reported by the health
service, and is not reachable over Bluetooth. The web UI's lighting pages and a song's lighting
editor call it for a song's light show files, their validation and evaluation, the Lighting
overview's readiness checks and the Fit shows page, the groups a cue can target, the fixture test,
the fixture types with their GDTF details and settings, and the venues with the MVR export's
summary and keep. The MVR import, inspector and download, the GDTF import and the lighting assets
are still the web API's.

| RPC | Description |
|---|---|
| `GetLightShow` | Reads the text of a song's light show file (`source`). `name` is the file's path under the songs directory, as the song list names it (`my_song/lighting/show.light`). `NOT_FOUND` when there is no such file, or the name leads outside the songs directory (by `..`, an absolute path or a link). Open while locked. |
| `SaveLightShow` | Validates and writes a song's light show, in a folder that is there (`NOT_FOUND` otherwise), against the tempo of the song the file belongs to and with its `include`s resolved where it is written and kept inside the songs directory. A show that does not parse is `INVALID_ARGUMENT`, with every problem in the message (each `Invalid light show DSL: ...`, joined by `; `); a name that is not a `.light` file, or is in the venues or fixture types directory this host reads, is `INVALID_ARGUMENT`; a name outside the songs directory, or a link whose target is, is `PERMISSION_DENIED`. There is no revision to present: the last save wins. Refused while locked. |
| `DeleteLightShow` | Deletes a song's light show file (a link is removed, never what it leads to). `NOT_FOUND` when there is none; the other refusals are `SaveLightShow`'s. Refused while locked. |
| `ValidateLightShow` | Checks light show text (`source`) without saving it, and answers normally: `valid`, and the `errors` when it is not. `song` names the song whose tempo map the text is read against (measure and beat timing does not parse without one); `file` is the file the text is a draft of, so its `include`s resolve where saving it would (a `file` outside the songs directory is `PERMISSION_DENIED`). Open while locked. |
| `GetLightingReadiness` | The facts behind the Lighting overview's five checks (`LightingReadiness`: the venue, its fixture types, the groups the shows target, every song's shows and what lint says, the DMX output and olad's patch), and the checks' verdicts on them, each with its `summary` parts and `findings`. Open while locked. |
| `GetLightingFit` | The facts and suggestions behind the Fit shows page (`LightingFit`): the venue's fixtures, and for each group the shows target what it needs, what it finds and the tags that would satisfy it; the focus points the venue lacks; the universes with no output. Open while locked. |
| `EvaluateSongLighting` | What a song's shows would be doing at each of `times` (seconds), offline, without touching the running engine (`LightingPreview`: for each time the stage values, as a live frame carries them, and the active effects, and the fixtures no cue targets). `NOT_FOUND` for no such song; `INVALID_ARGUMENT` for a song whose lighting did not load or has none, or times that are not non-negative seconds. Open while locked. |
| `ListLightingGroups` | The logical groups a cue can target, with the fixtures each resolves to in the loaded venue. Without a DMX engine the names come from the config and `resolved` is `false`. A config that does not load is `INTERNAL`. Open while locked. |
| `StartFixtureTest` | Starts or updates a test of one fixture at a universe and address: the `controls` (colour, level, strobe rate, pan and tilt, further channel levels) and `raw` channel values become the DMX a show would send for it, laid over the engine's output for a few seconds; the page keeps it alive while it is open. Answers the frame, how long it holds, and warnings (fixtures of the venue it is lit over, a span past address 512, olad not answering or not patched, controls the mode has no channel for). A refusal says why in the `x-refusal-reason` metadata: `playing`, `no_dmx` and `no_universes` (all `FAILED_PRECONDITION`), `universe`, `address`, `controls`, `mode` and `raw` (`INVALID_ARGUMENT`), `type` (`NOT_FOUND`), `files` (`INTERNAL`). Refused while locked. |
| `StopFixtureTest` | Releases the test output now. Open while locked: letting go only hands the lights back to the show. |
| `GetFixtureTestOptions` | What a test of a fixture type can do: its modes (each with the controls it supports and its channels), the running profile's universes with olad's patch state, and whether a test can run now (`available`, else `unavailable` says why). `NOT_FOUND` for a fixture type that is not there. Open while locked. |

The fixture type calls take a name and, optionally, the directories to work in: `dir` is the fixture
types directory and `venues_dir` the venues directory (where a GDTF type's users are found, and a
rename's venue lines), both project-relative, defaulting to this host's profile (a config that does
not load is `INTERNAL`). A name is a plain file name (`INVALID_ARGUMENT` for one with a path in it, `..`
or a reserved name such as `lock`); a directory that leaves the project by `..`, an absolute path or
a link is `INVALID_ARGUMENT` or `PERMISSION_DENIED`, and a save never makes a directory beneath such
a link.

| RPC | Description |
|---|---|
| `ListFixtureTypes` | Every fixture type of the directory and below (`fixture_types`, by name), and the files that did not read (`errors`). Each carries its file, its extension (`light` or `fixture`), the `owner` file and whether it `editable`, whether the channel-map form can say all of it (`form_editable`), its channels, and, for a GDTF type, a card of what its archive says (`gdtf`, unset when the archive does not read) with the venues' use of each mode. A directory that is not there lists nothing. Open while locked. |
| `GetFixtureType` | One type with the text of its file (`dsl`). A GDTF in the library with no record of its own is a fixture with no file. `NOT_FOUND` when nothing declares the name. Open while locked. |
| `SaveFixtureType` | Creates, updates or renames a fixture type from the file's whole text (`dsl`, which must declare this one type and nothing else) or from the channel-map form (`channel_map`: channel offsets and the strobe figures; refused, saying to edit as text, for a type with more in it than the form can say). A new name is a `.fixture` file; an existing one is written where its file is, and only if that file is the type's alone (`FAILED_PRECONDITION` naming the file and why otherwise). `file_form` `FIXTURE` moves a `.light` to a `.fixture`; `LIGHT` keeps a `.light` and is refused for a `.fixture`. `from` renames and retypes every venue line naming the old name in `venues_dir`, all or nothing. A text that does not parse, a GDTF archive that is missing or outside the project, and a text that declares another name are `INVALID_ARGUMENT`. There is no `expected_revision`: the file is replaced as it stands. Refused while locked. |
| `DeleteFixtureType` | Deletes the type's file when it is the type's alone (`FAILED_PRECONDITION` otherwise). A GDTF fixture's archive goes with its record unless another type uses it (`archive_removed`, `archive_kept`); a GDTF in the library with no record is its archive, deleted. `NOT_FOUND` when neither has the name. Refused while locked. |
| `GetFixtureTypeGdtfDetails` | What a GDTF fixture's archive holds, for its page (not the archive): every mode with its footprint, channel map and what a show can do in it (or why it cannot be imported), the venue fixtures using the type in their own modes, the beam, and the asset store paths of the rig model and thumbnail (`rig`, `thumbnail`, served by HTTP under `/lighting-assets/`). `NOT_FOUND` for no such type or a native one; `INVALID_ARGUMENT` for an archive that is missing or does not parse. Open while locked. |
| `GetFixtureTypeSettings` | A GDTF fixture's own settings: its name, movement limits and strobe curve, the GDTF's strobe table, and the `revision` they are at (`unrecorded` for a fixture with no record yet). `NOT_FOUND` for a native type. Open while locked. |
| `SaveFixtureTypeSettings` | Plans (`write` false) or saves a GDTF fixture's settings and, for a new name (`new_name`), the rename's rewrite of every venue line that names the type. `expected_revision` is the revision the settings were read at and `venue_revisions` the venue files' revisions from the plan: a record or venue file that has changed since is `ABORTED` (the record's current revision in `x-current-version`); so is every other conflict (a file that is not the type's alone, a name already taken), which makes a client read the settings again. Refused while locked, a plan too. |

The venue calls take a name and, optionally, the directories to work in, with the same meaning and
the same checks as the fixture type calls: `dir` (the venues directory; `venues_dir` on the calls
that also act on another directory) and `fixture_types_dir` are project-relative and default to this
host's profile. A name is a plain file name (the words `lock`, `validate` and `activate` are kept
for the web API's own routes and are refused as names), and a directory that leaves the project is
refused as above.

| RPC | Description |
|---|---|
| `ListVenues` | Every venue of the directory and below (`venues`, by name), each with its file and whether it is `editable` (`owners`), the `revisions` of the files they were read from, and the files that did not read (`errors`). A name two files declare is reported against both. A directory that is not there lists nothing. Open while locked. |
| `GetVenue` | One venue with the text of its file (`dsl`) and the file's `revision`. `NOT_FOUND` when no file that parses declares the name. Open while locked. |
| `SaveVenue` | Creates, updates or renames a venue from the file's whole text (`dsl`, which must declare this one venue and nothing else) or from the structured venue (`form`: fixtures as rows, focus points, provenance), which patches an existing file so its comments, its TODO lines and its layout survive. A new name is a `.venue` file; an existing one is written where its file is, and only if that file is the venue's alone (`FAILED_PRECONDITION` naming the file and why otherwise); a `.light` that gains a mode, a position or a focus point becomes a `.venue`. `from` renames. A text or a venue that does not hold is `INVALID_ARGUMENT` with the reason. `expected_revision`, the revision the venue was read at, makes a file that has changed since `ABORTED`, with its current revision in `x-current-version`. The answer has the new `revision`, whether the running engine re-read the venue (`reloaded`), and the venue's load failure (`venue_error`) when the save left the current venue failing: the save stands. Refused while locked. |
| `DeleteVenue` | Deletes the venue's file when it is the venue's alone (`FAILED_PRECONDITION` otherwise). `NOT_FOUND` when no file has the name. Refused while locked. |
| `GetVenuePatch` | The addresses each fixture of a venue occupies, read from the files with no DMX engine: each fixture's span (its `footprint` is unset when its type or mode does not load, never guessed), and the venue's `overlaps` and `overruns`. `NOT_FOUND` for a venue the files do not declare. Open while locked. |
| `GetVenueScene` | What the 3D view draws for a venue from its files: each fixture's type, mode, placement and rig path, and the venue's focus points and scenery. `NOT_FOUND` for a venue the files do not declare. Open while locked. |
| `AddVenueAimPoints` | Adds a focus point `<fixture> aim` to the venue's file for each fixed fixture with a position that an MVR export would not link to a focus point, where its rest beam meets the deck. The file is edited in place and the engine reloads the venue if it is playing it. `expected_revision` and `x-current-version` as for `SaveVenue`. Refused while locked. |
| `GetMvrExportSummary` | What an MVR export of a venue would hold, without writing or sending the archive: counts, the GDTFs embedded and generated, warnings, every positioned fixed fixture with the focus point it links to, and the `file_name` a download would carry. `INVALID_ARGUMENT` with the reason when it cannot be made. Open while locked. |
| `KeepMvrExport` | Writes the export to `lighting/export/` and answers where (`kept`). A file of that name already there is left alone and named (`existing`) unless `overwrite` is set. Refused while locked. |

| `ImportGdtf` | Imports the uploaded GDTF (`GDTF_IMPORT`) through the importer behind `mtrack import-gdtf`: a copy into `lighting/library/`, where it is a fixture, and a warmed rig for its page. `name` pins the fixture's name in a record. Answers what the user has: `type_name`, `already_imported`, `renamed_from`, the `refused_modes`. `FAILED_PRECONDITION` for a different archive of a file name already in the library, or a record name or file that is taken; `INVALID_ARGUMENT` for anything else refused (such as a file that is not a GDTF); the `x-import-kind` metadata says which kind of refusal (`name`, `library`, `record`, `write`, `unreadable`). Refused while locked. |
| `InspectMvr` | Reads the uploaded MVR (`MVR_INSPECT`): what an import would do (`report`, the plan) and the file seen from above (`scene`: every fixture's and focus point's position in the file's millimeters, the scenery, and the stage floor's bounds when it has one). Writes nothing. `options` are the venue `name`, the `origin_mm`, the `venues_dir` and `fixture_types_dir`. `INVALID_ARGUMENT` for a file the importer refuses, with the parser's reason; `FAILED_PRECONDITION` with `x-import-kind` `venue` for a venue name the one rule for lighting files refuses (another name answers it) and `record` for an embedded fixture's record. Open while locked. |
| `ImportMvr` | Imports the uploaded MVR (`MVR_IMPORT`): `write` false plans only (what an `InspectMvr`'s report says) and writes nothing; `write` true writes the archive and its embedded GDTFs to `lighting/library/`, a record per referenced type and a seeded or merged venue, and says whether the running engine re-read the venue (`reloaded`) and the venue's failure to load, if the import left one (`venue_error`). `options.keep` lists the hand edits a merge keeps. Never makes the venue current. Errors as `InspectMvr`. Refused while locked. |
| `ExportMvr` | Builds a venue's MVR archive and stages it for download: answers `path`, `/downloads/<id>` on the web server, the `file_name` it carries and its `size`. `GET <path>` answers the archive as an attachment, any number of times, until ten minutes after this call. Nothing is written to the project (`KeepMvrExport` does that). Counts toward the 32 pending uploads and downloads. `INVALID_ARGUMENT`, with the reason, when it cannot be made. Open while locked. |

### Files

Calls are gRPC; bytes are HTTP. A file travels in three steps, and the middle one is on the
**web server** (the web UI's port, 8080), not on the gRPC port, which does not serve it. A
script that talks to the gRPC port alone is handed a path only the web port serves.

1. `BeginUpload` announces the file and answers an `upload_id` and a `path`.
2. `PUT <web address><path>` sends the bytes, as the whole body, exactly the `size` announced. It
   answers 204; 404 for an id that is not known (or has expired), 409 for one that has its bytes
   already, 413 for a body longer than announced (reading stops and the upload is dropped). The
   id is the permission: the `PUT` is not behind the lock, because the id was issued under it. A
   body shorter than announced is received, and the claiming call refuses it.
3. The call that uses the file (`AddTrackFiles`, `AddSampleFile`, `ImportGdtf`, `InspectMvr`,
   `ImportMvr`) names the upload and answers what the file became. An upload is good for one
   purpose and one use; its staged file is deleted by the call whether it succeeds or not.

```bash
# 1. announce a track of the song "Opener"
grpcurl -plaintext -d '{"purpose":"UPLOAD_PURPOSE_TRACK","file_name":"kick.wav","size":882044,"song":"Opener"}' \
  host:43234 player.v1.ConfigService/BeginUpload
# { "uploadId": "9f0c…", "path": "/uploads/9f0c…" }

# 2. send the bytes to the web server
curl -sf -T kick.wav http://host:8080/uploads/9f0c…

# 3. add it to the song
grpcurl -plaintext -d '{"song":"Opener","upload_ids":["9f0c…"]}' \
  host:43234 player.v1.ConfigService/AddTrackFiles
```

An upload not yet sent is dropped ten minutes after `BeginUpload`, and one received but not used
ten minutes after its last byte; one still arriving is not dropped. At most 32 uploads and
staged downloads are pending at once. The staging files are in a private directory (mode `0700`)
made when the player starts, named by the id alone, and removed when it shuts down.

`ExportMvr` goes the other way: it answers a `path` (`/downloads/<id>`) on the web server, and
`curl -OJ http://host:8080/downloads/<id>` saves the archive under the name it carries. A
download is fetched as often as needed until it expires, ten minutes after `ExportMvr`.

3D models and thumbnails stay plain `GET`s under `/lighting-assets/` on the web port (not
the gRPC port, which has `GetLightingAsset` instead), which the stage's scene data names. They
are reads and are not checked against the lock.

## Lock mode

While the player is [locked](web-ui.md#lock-mode), the config writes (`UpdateAudio`,
`UpdateMidi`, `UpdateDmx`, `UpdateControllers`, `AddProfile`, `UpdateProfile`, `RemoveProfile`
and `ConfigService`'s `UpdateSamples`, `UpdateMetronome`, `SaveProfileFile`,
`DeleteProfileFile`, `ProbeAudioDevice`, the calibration calls, `CreateSong`, `SaveSongConfig`,
`DeleteSong`, `ImportSongFile`, `CreateSongInDirectory`, `BulkImportSongs`, `AddTrackFiles`,
`AddSampleFile`, `SetBluetoothPairing` and
`ForgetBluetoothDevice`, and `LightingService`'s `SaveLightShow`, `DeleteLightShow`,
`StartFixtureTest`, `SaveFixtureType`, `DeleteFixtureType`, `SaveFixtureTypeSettings`, `SaveVenue`, `DeleteVenue`,
`AddVenueAimPoints`, `KeepMvrExport`, `ImportGdtf` and `ImportMvr`) are refused with `FAILED_PRECONDITION` and the message "Player is locked.
Unlock to make changes." The other RPCs work while locked. `SetTrackGain` is a performance
control: a fader move still saves the track's gain to the active profile in `mtrack.yaml`,
after a short pause, while locked. `SavePlaylist` and `DeletePlaylist` are playlist edits and are
refused too. `BeginUpload` is refused for every purpose but `MVR_INSPECT`, a read, which `InspectMvr` and
`ExportMvr` are too. Unlock from the web UI, or with `SetLock`, which is open while locked. When the web UI cannot start, the
player runs unlocked, since the web UI is how the lock is normally released.

## Health checking

The player serves the standard [`grpc.health.v1.Health`](https://github.com/grpc/grpc/blob/master/doc/health-checking.md)
service on the gRPC port, and on the web UI's port over gRPC-Web. `Check` answers `SERVING` for the
server as a whole (the empty service name) and for `player.v1.PlayerService`,
`player.v1.ConfigService` and `player.v1.LightingService`; any other name is
`NOT_FOUND`. It answers while the player is locked. Tools that speak the protocol, such as
`grpc_health_probe` and load balancers, can use it as they would for any gRPC server:

```bash
grpcurl -plaintext host:43234 grpc.health.v1.Health/Check
```

`curl` cannot read a gRPC answer. A script that only has `curl` can fetch the web UI's page
(`curl -sf http://host:8080/`) to tell the server is up.

## Security

The gRPC server has no authentication. Do not run the player on a public network, and disable
the gRPC controller if the network the player is on is open to people you do not trust. See
[Security](../deployment/security.md).
