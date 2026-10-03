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

## Web UI file management

The web UI's song management, file upload, lighting editor, and playlist editor features
require all project files to reside under a single project root directory (the directory
containing `mtrack.yaml`). Files referenced via absolute paths or on separate mounts will
play back correctly but cannot be edited, uploaded to, or managed through the web UI.

mtrack must have write access to the project root directory for management features to work.
On read-only filesystems, playback and monitoring work but editing is disabled.
