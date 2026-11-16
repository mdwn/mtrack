# mtrack-core

The part of [mtrack](https://github.com/mdwn/mtrack) that needs no audio, MIDI
or DMX device: it reads a project (its configuration, songs, playlists and
light shows), checks it, and works out what a light show does.

mtrack, the player, is built on this crate and re-exports it. It is released
with the player, at the same version, and has no stable API of its own: use it
at the exact version of the player you mean to agree with.
