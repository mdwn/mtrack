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
# install-protoc.sh -- install a pinned protoc from the upstream release.
#
# ubuntu-22.04 ships protoc 3.12, which refuses the proto3 optional fields in
# src/proto/player/v1/player.proto ("--experimental_allow_proto3_optional was
# not set") -- the failure that broke the v0.16.0 Linux binaries. The runner is
# 22.04 deliberately, since it sets the glibc floor the released binaries are
# built against, so protoc has to come from somewhere newer than its archive.
#
# arduino/setup-protoc used to bridge that gap. It is unmaintained and
# permanently on the deprecated node20 runtime, so this does the same job with
# no third-party action in the path. See the repository issue about it.
#
# Usage:
#   scripts/install-protoc.sh [version]
#
# The version may also come from $PROTOC_VERSION; the argument wins. Installs
# under $PROTOC_PREFIX, default /usr/local.

set -euo pipefail

# Pinned rather than floating. The action this replaces asked for "29.x", so
# the protoc that compiled a release was whichever 29 was newest that day.
PROTOC_VERSION="${1:-${PROTOC_VERSION:-29.6}}"
PROTOC_PREFIX="${PROTOC_PREFIX:-/usr/local}"

# The release assets do not name architectures the way uname does: aarch64 is
# published as "aarch_64".
case "$(uname -m)" in
    aarch64) asset_arch="aarch_64" ;;
    x86_64)  asset_arch="x86_64" ;;
    *)
        echo "install-protoc: no protoc release for architecture $(uname -m)" >&2
        exit 1
        ;;
esac

asset="protoc-${PROTOC_VERSION}-linux-${asset_arch}.zip"
url="https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/${asset}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

echo "Fetching ${asset}"
# -f so an HTTP error becomes a failure here rather than a saved error page that
# fails later as a confusing unzip error.
curl -fsSL --retry 3 -o "$tmp/protoc.zip" "$url"

SUDO=""
[ "$(id -u)" -eq 0 ] || SUDO="sudo"

# include/ carries the well-known types. player.proto imports
# google/protobuf/duration.proto, so without them protoc cannot resolve the
# import and the build fails on something that reads like a missing file
# rather than a missing install step.
$SUDO unzip -q -o "$tmp/protoc.zip" -d "$PROTOC_PREFIX" 'bin/protoc' 'include/*'
$SUDO chmod 0755 "$PROTOC_PREFIX/bin/protoc"

"$PROTOC_PREFIX/bin/protoc" --version
