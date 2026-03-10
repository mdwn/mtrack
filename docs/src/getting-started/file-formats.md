# File Formats

## Configuration files

`mtrack` now uses [config-rs](https://github.com/rust-cli/config-rs) for configuration parsing, which
means we should support any of the configuration file formats that it supports. Testing for anything
other than YAML is limited at the moment.

## Audio files

`mtrack` supports a wide variety of audio formats through the [symphonia](https://github.com/pdeljanov/Symphonia) library. Supported formats include:

- **WAV** (PCM, various bit depths)
- **FLAC** (Free Lossless Audio Codec)
- **MP3** (MPEG Audio Layer III)
- **OGG Vorbis**
- **AAC** (Advanced Audio Coding)
- **ALAC** (Apple Lossless, in M4A containers)

All audio files are automatically transcoded to match your audio device's configuration (sample rate, bit depth, and format). Files can be mixed and matched within a song - for example, you can use a WAV file for your click track and an MP3 file for your backing track.
