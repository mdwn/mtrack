# MIDI-Based DMX System

> **Note**: The new tag-based lighting system (described in the [lighting overview](../lighting/overview.md)) is the recommended approach for most users. The MIDI-based DMX system is still supported for specific use cases requiring direct channel control.

## When to Use Each System

**Use the New Tag-Based System when:**
- You want venue-agnostic lighting shows
- You prefer high-level effect definitions
- You want intelligent fixture selection
- You're creating new lighting shows

**Use the MIDI-Based DMX System when:**
- You have existing MIDI-based lighting shows
- You need precise channel-level control
- You're integrating with existing MIDI workflows
- You prefer direct DMX channel programming

## Basic DMX Information

DMX is a standard that allows for the controlling of stage devices, primarily lights. Each of these devices will react to data being
fed into one or more DMX channels. Each DMX channel can be set from `0` to `255`. For example, a multicolor stage light might have 3
DMX channels: 1 for red, 1 for green, 1 for blue. In order to set the color of the light, you would have to supply these channels with
data representing the color that you want. DMX data is arranged into universes, where 1 universe consists of 512 channels of DMX data.

## Configuring mtrack for Legacy DMX Playback

In order to use legacy light shows, you'll need to set up OLA on your playback device and map your DMX devices into DMX universes. I recommend
following [this tutorial](https://www.openlighting.org/ola/getting-started/). mtrack assumes that OLA is running on the same device.

mtrack can be configured to stream DMX data to OLA universes. This can be done through the mtrack configuration file when using `mtrack start`
or through the command line when using `mtrack play` using the `--dmx-dimming-speed-modifier` argument and the `dmx-universe-config` arguments.
The `dmx-universe-config` argument format is:

```
universe=1,name=light-show;universe=2,name=another-light-show
```

Legacy light shows can be defined in `Song` files and consist of an array of "universe names" and MIDI files. These universe names correlate to the
names used in the mtrack configuration. For instance, a song with a light show with a universe name of `light-show` will play on the mtrack
universe with the equivalent name.

Additionally, songs can be defined to only recognize specific MIDI channels from the given MIDI file as lighting data. For instance, if you
have a single MIDI file that contains all of your automation, you can restrict light shows to only recognize events from channel 15.

Examples for these configuration options are in the song definition example and mtrack player examples above.

### Live MIDI to DMX mapping

mtrack is also capable of mapping live MIDI events into the DMX engine. This allows for live control of lighting using a
MIDI controller. Additionally, transformations can be applied to the incoming MIDI events that allow for singular MIDI messages
to be transformed into multiple MIDI messages and then fed into the DMX engine, allowing for the control multiple lights. Right now,
the transformers supported are:

- Note Mapper: This maps one note on a MIDI Channel to multiple notes, all with the same velocity. Works for both note_on and note_off events.
- Control Change Mapper: This maps one control change event on a MIDI Channel to multiple control change events, all with the same value.

Right now collision behavior is undefined. The intention is to provide some sort of composable mechanism here, so it's very possible that
this interface will change in the future.

## MIDI format

The MIDI engine was heavily inspired by the [DecaBox MIDI to DMX converter](https://response-box.com/gear/product/decabox-protocol-converter-basic-firmware/), with the MIDI to DMX conversion mechanism being described
[here](http://67.205.146.177/books/decabox-midi-to-dmx-converter).

Note here that MIDI is an older protocol and doesn't have the same resolution that DMX does. As a result, we have
to do some munging here in order to make this work. Some other notes:

- `u7` is an unsigned 7 bit integer, which ranges from 0-127.
- mtrack only supports 127 DMX channels per universe at present.

MIDI data is converted into DMX data as follows:

| MIDI Event | Outputs | Description |
|------------|---------|-------------|
| key on/off | key (`u7`), velocity(`u7`) | The value of _key_ is interpreted as the DMX channel, and _velocity_ is doubled and assigned to the channel |
| program change | program (`u7`) | The dimming speed. 0 means instantaneous, any other number is multiplied by the dimming speed modifier and used as a duration. |
| continuous controller | controller (`u7`), value (`u7`) | Similar to key on/off, the value of the _controller_ is interpreted as the DMX channel, and _value_ is doubled and assigned to the channel. Ignores dimming. |

The general idea here is to create a MIDI file that generally describes the way you want your lights to display. Much like regular MIDI
automation, you can program some pretty dynamic lights this way.

## Dimming engine

The dimming engine built into mtrack is controlled by program change (PC) commands. The value of the PC command will be multiplied by
the dimming speed modifier and will produce a duration. Subsequent key on/off commands will gradually progress to their new value
over this duration. For example, a dimming speed modifier of `0.25` and a PC command with a `1` will produce a dimming duration of `0.25`.
New key on/off events will take `0.25` seconds to reach the new value. PC0 will ensure color changes are instantaneous.

Dimming of channels is independent of one another. Imagine a lifecycle that looks like this, assuming a dimming speed modifier of 1:

```
PC5 --> key_on(0, 127) --> PC10 --> key_on(1, 127)
```

A PC command instructs the dimmer to dim over 5 seconds. The first `key_on` event will gradually progress channel from 0 to 127 over 5 seconds.
After this, another PC command instructs the dimmer to dim over 10 seconds. The second `key_on` event will gradually progress channel 1 from 0
to 127 over 10 seconds. This will not affect channel 0, which will still only take 5 seconds.

Continuous controller (CC) messages will ignore dimming.
