# Installation

## Pre-built binaries (recommended)

Download a binary from the
[latest release](https://github.com/mdwn/mtrack/releases/latest) for your
platform — Linux (x86_64, aarch64) or macOS (Intel, Apple Silicon) — extract
it, and put `mtrack` somewhere on your `PATH`:

```
$ tar xzf mtrack-<version>-<target>.tar.gz
$ sudo cp mtrack-<version>-<target>/mtrack /usr/local/bin/mtrack
```

On Linux, the binary needs `libasound2` and `libudev1` at runtime. These are
present by default on most desktop distros; on a minimal server:

```
$ sudo apt install libasound2 libudev1
```

If you use [cargo-binstall](https://github.com/cargo-bins/cargo-binstall), it
will fetch the same release binaries:

```
$ cargo binstall mtrack
```

## From source with cargo

Building from source requires a Rust toolchain plus a few system packages:
the protobuf compiler and, on Linux, ALSA and udev development headers.

On Debian/Ubuntu:

```
$ sudo apt install libasound2-dev libudev-dev pkg-config libssl-dev protobuf-compiler
```

On macOS:

```
$ brew install pkg-config protobuf
```

Then:

```
$ cargo install mtrack --locked
```

If you want to use `mtrack` on startup, I recommend copying it to
`/usr/local/bin`:

```
$ sudo cp ~/.cargo/bin/mtrack /usr/local/bin/mtrack
```
