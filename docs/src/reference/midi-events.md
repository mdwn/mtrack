# Supported MIDI Events

The following MIDI events can be written wherever a configuration file takes a MIDI event:
a song's `midi_event`, the MIDI controller's `play`/`prev`/`next`/... keys, status events, and
`kind: midi` trigger inputs. Every event has a `type` and a `channel`; channels are numbered
1–16, and all other values are 0–127 except `bend`, which is 0–16383.

```yaml
# Note Off: acts as if a note was released.
midi_event:
  type: note_off
  channel: 5 # Channels are expected to be from 1-16.
  key: 60
  velocity: 0 # optional, defaults to 0
---
# Note On: acts as if a note was pressed.
midi_event:
  type: note_on
  channel: 5
  key: 60
  velocity: 127 # optional, defaults to 0
---
# Polyphonic aftertouch: pressure on one key.
midi_event:
  type: aftertouch
  channel: 5
  key: 60
  velocity: 127
---
# Control Change: sets a controller to a value.
midi_event:
  type: control_change
  channel: 5
  controller: 12
  value: 27
---
# Program Change: changes banks and instruments on various devices.
midi_event:
  type: program_change
  channel: 5
  program: 20
---
# Channel aftertouch: pressure for the whole channel, no key.
midi_event:
  type: channel_aftertouch
  channel: 5
  velocity: 127
---
# Pitch bend. `bend` is the 14-bit value, 0-16383; 8192 is centre.
midi_event:
  type: pitch_bend
  channel: 5
  bend: 8192
```

When an event is used to match incoming MIDI, the two uses compare differently. The MIDI
controller's keys (`play`, `prev`, ...) match the whole event, velocity included, so a Note On
with a different velocity is not recognized. A `kind: midi` sample trigger matches a Note
On/Off on channel and key only; the incoming velocity drives the sample's velocity handling.

System Exclusive, system real-time and other message types cannot be written in configuration.
