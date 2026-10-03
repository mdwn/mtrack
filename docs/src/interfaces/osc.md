# OSC Control

The player can be controlled with OSC messages and can report its state back to a fixed list of
OSC clients. This page lists every OSC address the controller uses. Each is a key in the OSC
controller's configuration, so each address can be renamed to match a control surface.

## Configuration

Add an OSC controller to the profile's `controllers` (see
[Player Configuration](../configuration/player-config.md) for where controllers live). Every key
except `kind` is optional; the values below are the defaults.

```yaml
controllers:
- kind: osc

  # The UDP port the OSC server listens on.
  port: 43235

  # Where status broadcasts are sent (ip:port). Empty by default: nothing is broadcast
  # until you list at least one client.
  broadcast_addresses:
  - 127.0.0.1:43236

  # Events the player receives.
  play: /mtrack/play
  pause: /mtrack/pause
  prev: /mtrack/prev
  next: /mtrack/next
  stop: /mtrack/stop
  all_songs: /mtrack/all_songs
  playlist: /mtrack/playlist
  stop_samples: /mtrack/samples/stop
  section_ack: /mtrack/section_ack
  stop_section_loop: /mtrack/stop_section_loop
  loop_section: /mtrack/loop_section
  seek: /mtrack/seek
  seek_section: /mtrack/seek_section
  track_gain: /mtrack/track/*/gain

  # Events the player broadcasts.
  status: /mtrack/status
  audio_health: /mtrack/audio/health
  playlist_current: /mtrack/playlist/current
  playlist_current_song: /mtrack/playlist/current_song
  playlist_current_song_elapsed: /mtrack/playlist/current_song/elapsed
  timeline: /mtrack/timeline
```

## Messages the player receives

| Key | Default address | Argument | Action |
|---|---|---|---|
| `play` | `/mtrack/play` | none | Plays the current song. |
| `pause` | `/mtrack/pause` | none | Stops all synchronized playback subsystems and keeps the song position. |
| `prev` | `/mtrack/prev` | none | Moves the playlist to the previous song. |
| `next` | `/mtrack/next` | none | Moves the playlist to the next song. |
| `stop` | `/mtrack/stop` | none | Stops playback and resets the next start to the beginning of the song. |
| `all_songs` | `/mtrack/all_songs` | none | Switches to the `all_songs` playlist. |
| `playlist` | `/mtrack/playlist` | none | Switches back to the configured playlist. |
| `stop_samples` | `/mtrack/samples/stop` | none | Stops every triggered sample that is playing. |
| `section_ack` | `/mtrack/section_ack` | none | Acknowledges the current section, arming its loop. |
| `stop_section_loop` | `/mtrack/stop_section_loop` | none | Breaks out of the active section loop. |
| `loop_section` | `/mtrack/loop_section` | section name (`string`) | Loops the named section. |
| `seek` | `/mtrack/seek` | seconds (`float`, `double` or `int`, not negative) | Seeks within the current song. |
| `seek_section` | `/mtrack/seek_section` | section name (`string`) | Seeks to the start of the named section. |
| `track_gain` | `/mtrack/track/*/gain` | gain in dB (`float`; `double` and `int` also accepted) | Sets the gain of one output track. |

`/mtrack/pause` leaves the song position where it is: the next `/mtrack/play` resumes from there.
`/mtrack/stop` resets the next start to the beginning of the song. `all_songs` and `playlist`
are refused while a song is playing.

### Track gain

In the `track_gain` address, the `*` is the track name: `/mtrack/track/click/gain` with `-6.0`
sets the `click` track to -6 dB. The value is clamped to the range -60 dB to +12 dB, and a gain
at or below -60 dB is silence. A message for an unknown track, or without a finite numeric
argument, is logged and ignored. The change applies immediately and is persisted shortly afterwards.

## Messages the player broadcasts

The player sends these to every address in `broadcast_addresses` every 500 ms, and again
right after it handles a control message.

| Key | Default address | Arguments |
|---|---|---|
| `status` | `/mtrack/status` | `Playing` or `Stopped` (`string`). |
| `playlist_current_song` | `/mtrack/playlist/current_song` | Name of the current song (`string`). |
| `playlist_current_song_elapsed` | `/mtrack/playlist/current_song/elapsed` | Elapsed and total time as `m:ss/m:ss` (`string`). |
| `playlist_current` | `/mtrack/playlist/current` | The playlist as one numbered, newline-separated `string` (`1. First Song`, `2. Second Song`, ...). |
| `timeline` | `/mtrack/timeline` | Elapsed seconds, total seconds, then section triples. See below. |
| `audio_health` | `/mtrack/audio/health` | The audio output's health as one `string`: `healthy`, `recovering`, `stalled` or `never_started`. Sent only when the audio device can report health. |
| `track_gain` | `/mtrack/track/<name>/gain` | The track's gain in dB (`float`), one message per output track whose name can form an OSC address segment. |

`status` reports what the player is doing, and `audio_health` reports whether audio is leaving
the machine. The player can be `Playing` while the output is `stalled`, a combination worth a red
light on a control surface.

## Timeline and sections

The `timeline` broadcast is intended for dynamic control surfaces. Its OSC arguments are:

1. elapsed time in seconds (`double`)
2. total song duration in seconds (`double`)
3. zero or more repeating section triples: section name (`string`), start seconds (`double`),
   end seconds (`double`)

For example, a song with two sections can produce:

```text
/mtrack/timeline 18.25 240.0 "Intro" 0.0 12.5 "Odd chorus" 12.5 42.0
```

Section names and boundaries come from the current song, so clients do not need to hard-code a
shared song structure. Clients can use the numeric elapsed and duration values to drive a progress
bar, draw section markers, and send a selected start time back to `/mtrack/seek`.

A starting TouchOSC file has been supplied [here](https://github.com/mdwn/mtrack/blob/main/touchosc/mtrack.tosc).
