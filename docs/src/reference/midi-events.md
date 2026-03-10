# Supported MIDI Events

As of now, the following MIDI events can be defined as part of the controller and song emit features:

```yaml
# The note_off MIDI event acts as if a note was released.
midi_event:
  type: note_off
  channel: 5 # Channels are expected to be from 1-16.
  note: 5
  velocity: 127
---
# The note_on MIDI event acts as if a note was pressed.
midi_event:
  type: note_on
  channel: 5
  note: 5
  velocity: 127
---
# The aftertouch MIDI event acts as if an aftertouch MIDI event was sent.
midi_event:
  type: aftertouch
  channel: 5
  note: 5
  velocity: 127
---
# The control_change MIDI event can controller values.
midi_event:
  type: control_change
  channel: 5
  controller: 12
  value: 27
---
# The program_change MIDI event can change banks and instruments on various devices.
midi_event:
  type: program_change
  channel: 5
  program: 20
---
# The aftertouch MIDI event acts as if a channel aftertouch MIDI event was sent.
midi_event:
  type: channel_aftertouch
  channel: 5
  velocity: 127
---
# The pitch bend MIDI event acts as if a pitch bend MIDI event was sent.
midi_event:
  type: pitch_bend
  bend: 1234
```

There are more that can be implemented, but these are just the ones that came to me at the moment.
If you'd like to add any particular ones, please file an issue. Otherwise I'll add them in as they
strike me.
