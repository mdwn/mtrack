# Tested hardware and limitations

mtrack has been tested with the following hardware:

## Audio cards
- MOTU UltraLite-mk5
- Behringer X32 (through X-Live card)
- Behringer Wing Rack

## MIDI
- MOTU UltraLite-mk5
- Roland UM-ONE
- CME U6MIDI Pro
- Morningstar MC4 Pro (SysEx song name display)

## DMX

DMX output goes through OLA, so any interface olad drives should work; the devices that have been explicitly tested:

- Enttec DMX USB Pro
- RatPac Satellite (Art-Net and sACN)
- Cinelex Skycast A (sACN)

Colour effects drive RGB(W) and CMY fixtures. Colour wheels are not modelled: `color:`, colour
cycles and rainbows leave a wheel-only fixture white, and Stage 3D draws it white whatever slot it
is on. A slot can still be chosen with a `static` naming the wheel's channel. See
[GDTF fixture types](../lighting/configuration.md#gdtf-fixture-types).

## MIDI Beat Clock

The MIDI beat clock uses a dedicated real-time thread to deliver 24-ppqn timing
messages. Accurate tempo requires elevated thread priority:

- **Linux**: Requires `CAP_SYS_NICE` for `SCHED_FIFO` real-time scheduling. The
  systemd service unit grants this automatically. Without it, jitter may increase
  under load.
- **macOS**: Crossplatform thread priority elevation works without special
  privileges. POSIX `SCHED_FIFO` is not available on macOS CoreAudio threads;
  this is expected and does not affect timing.

The thread priority can be tuned with `MTRACK_THREAD_PRIORITY` (0–99, default 70)
or disabled entirely with `MTRACK_DISABLE_RT_AUDIO=1`.

The beat clock follows every loop as the notes, lights and audio do: a section loop, and a
looping song's passes, its pulses switching to the new pass where it starts (through a
`loop_crossfade` too: the clock does not overlap). By default its pulses run straight through each
seam and nothing else is sent, so gear counting pulses counts on past the loop. With
`midi.beat_clock_seams: relocate`, each jump sends Stop, a Song Position Pointer and Continue
(Start back at the top of the song); the pointer counts 16th notes, so a section starting off a
16th goes on from the next 16th, and its pulses before that are not sent, and a section with no
16th in it runs through. A section shorter than a pulse has no pulse to go back to: the clock
pauses while it goes round. Song Position Pointer reaches 16,383 16ths (about 34 minutes at 120
BPM); past that it is clamped.

The beat clock is timed on the computer's clock and steered gently onto the audio's, by at most
250 ppm, so that gear following it sees a steady tempo. An audio device whose clock runs further
off than that (one resampling, say) leaves the beat clock drifting from the audio over a song;
the notes, which follow the audio at its own rate, do not drift.

The notes share the beat clock's zero, measured against the audio over a song's first half
second; until then they go on the computer's clock, which on a device far off it (10%, say)
puts them that far off the audio for that half second. On a sound card, whose callbacks come
within a millisecond or so of their time, the notes keep within a few milliseconds of the beat
clock's grid. An xrun or a stall can put them up to one audio period ahead of the audio (what
was due before the audio stopped); they then hold until it catches up. On a device whose
callbacks come periods early or late as a rule, they keep within a few periods of the audio.

Before bar 1, a `tempo:` count-in pulses only in whole bars (rounded down to whole quarter notes);
any leftover before them has no pulses, in the first pass or later ones.

## Web UI file management

The web UI's song management, file upload, lighting editor, and playlist editor features
require all project files to reside under a single project root directory (the directory
containing `mtrack.yaml`). Files referenced via absolute paths or on separate mounts will
play back correctly but cannot be edited, uploaded to, or managed through the web UI.

mtrack must have write access to the project root directory for management features to work.
On read-only filesystems, playback and monitoring work but editing is disabled.
