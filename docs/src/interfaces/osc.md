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

A starting TouchOSC file has been supplied [here](https://github.com/mdwn/mtrack/blob/main/touchosc/mtrack.tosc).
