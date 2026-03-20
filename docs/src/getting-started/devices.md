# Discovering Devices

The **Status** page in the web UI shows all connected audio and MIDI devices. This is the
easiest way to check what hardware mtrack can see.

Alternatively, you can use the CLI commands below.

## CLI Commands

You can figure out what audio devices `mtrack` recognizes by running `mtrack devices`:

```
$ mtrack devices
Devices:
- UltraLite-mk5 (Channels=22) (Alsa)
- bcm2835 Headphones (Channels=8) (Alsa)
```

The name prior to the parentheses is the identifier for use by `mtrack`. So when referring to the first
audio device, you would use the string `UltraLite-mk5`.

You can also figure out what MIDI devices are supported by running `mtrack midi-devices`:

```
$ mtrack midi-devices
Devices:
- Midi Through:Midi Through Port-0 14:0 (Input/Output)
- UltraLite-mk5:UltraLite-mk5 MIDI 1 28:0 (Input/Output)
```

The name prior to the first colon is the identifier for use by `mtrack`. When referring to the second
MIDI device, you would use the string `UltraLite-mk5`.
