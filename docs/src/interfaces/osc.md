# OSC Control

The player can also be controlled using arbitrary OSC commands. This is configurable in the OSC
controller configuration section. This allows you to define OSC addresses that will map to
player events (play, previous, next, stop, all_songs, playlist). Refer to the
[player configuration](../configuration/player-config.md) for the exact name of these events.

Additionally, information can be reported back to a fixed list of clients from the OSC server.
This will allow OSC clients to display things like the current song the playlist is pointing to,
whether or not the player is currently playing, how much time has elapsed, and the contents of
the playlist. Again, refer to the [player configuration](../configuration/player-config.md)
for the defaults for these events.

## Timeline and sections

The `/mtrack/timeline` broadcast is intended for dynamic control surfaces. Its OSC arguments are:

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
