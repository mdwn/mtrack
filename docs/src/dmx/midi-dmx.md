# MIDI-Based DMX

mtrack supports two approaches to lighting control: the **tag-based DSL system** (described in
the [lighting overview](../lighting/overview.md)) and the **MIDI-based DMX system** described
here. Both can be used independently or together in the same project.

> **Tip**: If you're starting fresh, the DSL-based system is recommended for most users.
> It provides venue-agnostic shows, high-level effects, and integrates with the web UI's
> visual timeline editor. The MIDI-based system is best suited for workflows that require
> direct per-channel DMX control or integration with DAW-authored MIDI lighting data.

## When to Use Each System

**Use the Tag-Based DSL System when:**
- You want venue-agnostic lighting shows
- You prefer high-level effect definitions (static, cycle, chase, strobe, etc.)
- You want intelligent fixture selection via logical groups
- You want to use the timeline editor for visual cue authoring

**Use the MIDI-Based DMX System when:**
- You need precise per-channel DMX control
- You're integrating with existing MIDI workflows or DAWs
- You prefer programming lights as MIDI data
- You want to use live MIDI controllers for real-time DMX control

## Basic DMX Information

DMX is a standard that allows for the controlling of stage devices, primarily lights. Each of these devices will react to data being
fed into one or more DMX channels. Each DMX channel can be set from `0` to `255`. For example, a multicolor stage light might have 3
DMX channels: 1 for red, 1 for green, 1 for blue. In order to set the color of the light, you would have to supply these channels with
data representing the color that you want. DMX data is arranged into universes, where 1 universe consists of 512 channels of DMX data.

## Configuring mtrack for MIDI DMX Playback

MIDI-based light shows need olad running on the playback machine with your DMX interface
patched to a universe; the [OLA getting started guide](https://www.openlighting.org/ola/getting-started/)
covers that, and [The universe must exist in olad](../lighting/configuration.md#the-universe-must-exist-in-olad)
the one step most often missed. mtrack talks to olad on the same machine (port 9010 by
default, `dmx.ola_port`).

The universes mtrack streams to are declared in the hardware profile's `dmx` section, each
with a name a song refers to. `dim_speed_modifier` scales the dimming engine below (default
`1.0`):

```yaml
dmx:
  dim_speed_modifier: 0.25
  universes:
    - universe: 1
      name: light-show
```

A song then lists its MIDI light shows under `light_shows:`. Each names the universe to play
on, the MIDI file to read as DMX (relative to the song directory), and optionally which of the
file's MIDI channels (1–16) carry lighting data; with `midi_channels` omitted, every channel
does. This is `examples/songs/another-cool-song/song.yaml`, whose one MIDI file carries both
the song's MIDI playback and, on channel 15, its lighting:

```yaml
kind: song
name: Another cool song

midi_playback:
  file: song.mid
  exclude_midi_channels:
  - 15

light_shows:
- universe_name: light-show
  dmx_file: song.mid
  midi_channels:
  - 15

tracks:
- name: click
  file: click.wav
```

A light show whose `universe_name` matches no universe in the active profile is not sent
anywhere.

### Live MIDI to DMX mapping

mtrack can also feed live MIDI events into the DMX engine, so a MIDI controller drives lights
in real time. The mapping is in the profile's `midi` section: a MIDI channel (1–16) to listen
on and the universe to drive, with optional transformers that turn one incoming message into
several:

```yaml
midi:
  device: "My MIDI Interface"
  midi_to_dmx:
    - midi_channel: 15
      universe: light-show
      transformers:
        - type: note_mapper
          input_note: 60
          convert_to_notes: [60, 61, 62]
        - type: control_change_mapper
          input_controller: 1
          convert_to_controllers: [1, 2, 3]
```

- `note_mapper`: maps one note to several, all with the incoming velocity, for both note on and
  note off.
- `control_change_mapper`: maps one control change to several, all with the incoming value.

Collision behaviour between transformers is undefined.

## MIDI format

The MIDI engine was heavily inspired by the [DecaBox MIDI to DMX converter](https://response-box.com/gear/product/decabox-protocol-converter-basic-firmware/), with the MIDI to DMX conversion mechanism being described
[here](http://67.205.146.177/books/decabox-midi-to-dmx-converter).

MIDI values are 7-bit (`u7`, 0–127) where DMX values are 8-bit (0–255), so the conversion
doubles values and shifts channel numbers by one:

| MIDI Event | Outputs | Description |
|------------|---------|-------------|
| note on/off | key (`u7`), velocity (`u7`) | DMX channel = key + 1 (keys 0–127 address DMX channels 1–128); value = velocity × 2 (0–254). Subject to dimming. |
| program change | program (`u7`) | Sets the dimming speed: program × `dim_speed_modifier` seconds. 0 means instantaneous. |
| control change | controller (`u7`), value (`u7`) | DMX channel = controller + 1; value = value × 2. Ignores dimming. |

Because a key or controller number is 7-bit, the MIDI path reaches DMX channels 1–128 of a
universe; a fixture patched above 128 cannot be driven this way (the DSL system has no such
limit). A doubled velocity tops out at 254, not 255.

The general idea here is to create a MIDI file that generally describes the way you want your lights to display. Much like regular MIDI
automation, you can program some pretty dynamic lights this way.

## Dimming engine

The dimming engine is controlled by program change (PC) commands. The value of the PC command is multiplied by
`dim_speed_modifier` to produce a duration, and subsequent note on/off commands move their channel to its new value
over that duration. For example, a `dim_speed_modifier` of `0.25` and a PC command of `1` produce a dimming duration of `0.25`:
new note events take 0.25 seconds to reach their value. PC 0 makes changes instantaneous.

Each channel dims on its own. Take this sequence, with `dim_speed_modifier: 1.0`:

```
PC5 --> note_on(key 0, velocity 127) --> PC10 --> note_on(key 1, velocity 127)
```

The first PC sets a 5-second dim. The first note (key 0) drives DMX channel 1 from 0 to 254 over 5 seconds.
The second PC sets a 10-second dim, and the second note (key 1) drives DMX channel 2 from 0 to 254 over 10 seconds.
Channel 1 is unaffected and still completes in 5 seconds.

Control change (CC) messages ignore dimming.
