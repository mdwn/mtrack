# Discovering Devices

Before you write a [hardware profile](hardware-config.md), find the names mtrack uses for your
audio and MIDI devices.

In the web UI, the **Audio** and **MIDI** tabs of a profile on the **Config** page have a
device field that lists the devices available on the current host. This is the easiest way to
discover what hardware mtrack can see. The **Status** page shows the devices that are
currently configured and connected in the active profile.

You can also use the CLI commands below, which work without the web UI.

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
