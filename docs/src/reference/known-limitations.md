# Known Limitations

This has been tested with:

## Audio cards
- MOTU UltraLite-mk5
- Behringer X32 (through X-Live card)
- Behringer Wing Rack

## MIDI
- MOTU UltraLite-mk5
- Roland UM-ONE
- CME U6MIDI Pro

## DMX

DMX is expected to be well supported through OLA, but the devices that have been explicitly tested:

- Entec DMX USB Pro
- RatPac Satellite (Art-Net and sACN)
- Cinelex Skycast A (sACN)

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

## General disclaimer

This is my first Rust project, so this is likely cringey, horrible non-idiomatic Rust. Feel free to
submit PRs to make this better.
