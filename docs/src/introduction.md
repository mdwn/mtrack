# Introduction

[![Actions Status](https://github.com/mdwn/mtrack/actions/workflows/mtrack.yaml/badge.svg)](https://github.com/mdwn/mtrack/actions)
[![codecov](https://codecov.io/gh/mdwn/mtrack/graph/badge.svg?token=XWEK2BIPZL)](https://codecov.io/gh/mdwn/mtrack)
[![Crates.io Version](https://img.shields.io/crates/v/mtrack)](https://crates.io/crates/mtrack)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-2.1-4baaaa.svg)](https://github.com/mdwn/mtrack/blob/main/CODE_OF_CONDUCT.md)

`mtrack` is a multitrack player intended for running on small devices like the Raspberry Pi. It can output
multiple tracks of audio as well as MIDI out via class compliant interfaces. The general intent here is to
allow `mtrack` to be controlled remotely from your feet as opposed to needing to drive a computer or tablet
on stage.

## Hands free multitrack playing

The idea behind `mtrack` is to provide a way to play multitracks in a live situation without using your hands.
In live situations, I frequently found myself babysitting a DAW while performing. The point of `mtrack` is to
avoid this situation by providing a very simple mechanism for playing back songs.

`mtrack` can read from multiple audio files and rearrange and combine the channels present in those files into
a singular audio stream that is routed to a class complaint audio interface. Additionally, `mtrack` can
simultaneously play back a MIDI file along with your audio, which allows for automation of on stage gear. `mtrack`
can also emit MIDI events on song selection, as well as listen for MIDI events in order to control the `mtrack`
player.

### The general behavior of mtrack

`mtrack` intends to have the following behavior loop:

1. `mtrack` starts on the first item in the user defined playlist. The item is selected, but not playing.
2. While no song is playing, the user can select a song on the playlist by using the `next` and `previous`
   events. `next` and `previous` are inactive when a song is playing.
3. The user can start a song using the `play` event and stop a currently playing song using the `stop` event.
   While a song is playing, `play` will perform no action, and while a song is not playing, `stop` will
   perform no action.
4. If a user needs to play a song not represented in their playlist, the user can use the `all_songs`
   event to move to a playlist that comprises a sorted list of all songs in a user's song repository. If the
   user would like to use their original playlist, the `playlist` event can be used.

The events listed above can be triggered using MIDI messages.
