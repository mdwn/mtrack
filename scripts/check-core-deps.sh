#!/usr/bin/env bash
# Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
#
# This program is free software: you can redistribute it and/or modify it under
# the terms of the GNU General Public License as published by the Free Software
# Foundation, version 3.
#
# This program is distributed in the hope that it will be useful, but WITHOUT
# ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
# FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License along with
# this program. If not, see <https://www.gnu.org/licenses/>.
#
# check-core-deps.sh -- fail if mtrack-core depends on a hardware library.
#
# mtrack-core is the part of the player that reads and checks a project with
# no audio, MIDI, DMX or Bluetooth: it has to build on a machine that has none
# of their system libraries. Those are installed wherever the player is built,
# so a dependency on one would compile there and go unnoticed. This reads the
# crate's dependency tree for them instead.
#
# Run from the repository root.

set -euo pipefail

# The libraries the player reaches hardware through, on every platform it
# builds for (`cargo tree -p mtrack --target all` is where the names are from).
HARDWARE='^(cpal|midir|alsa|alsa-sys|ola|bluer|dbus|libdbus-sys|libudev-sys|coreaudio-rs|coreaudio-sys|coremidi|coremidi-sys|objc2-core-audio|objc2-core-audio-types|objc2-audio-toolbox|objc2-avf-audio|ndk|ndk-sys|oboe|asio-sys|jack) '

# Every target and every feature: a dependency for macOS or Windows only, or
# behind a feature, counts as much as one built here.
tree=$(cargo tree -p mtrack-core -e normal,build --target all --all-features --prefix none --locked)
if found=$(grep -E "$HARDWARE" <<<"$tree" | sort -u); then
    echo "mtrack-core depends on a hardware library:" >&2
    echo "$found" >&2
    exit 1
fi
echo "mtrack-core links no hardware library."
