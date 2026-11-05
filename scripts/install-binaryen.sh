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
# install-binaryen.sh -- install a pinned wasm-opt from the binaryen release.
#
# `make gen-wasm` optimises the web UI's rules module with wasm-opt. wasm-pack
# would download a binaryen of its own choosing for that; this installs one
# version, the same on every machine and in CI, so a build never fetches an
# unpinned binary.
#
# Usage:
#   scripts/install-binaryen.sh [version]
#
# The version may also come from $BINARYEN_VERSION; the argument wins. Installs
# wasm-opt under $BINARYEN_PREFIX, default /usr/local.

set -euo pipefail

BINARYEN_VERSION="${1:-${BINARYEN_VERSION:-130}}"
BINARYEN_PREFIX="${BINARYEN_PREFIX:-/usr/local}"

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)   asset_platform="x86_64-linux" ;;
    Linux-aarch64)  asset_platform="aarch64-linux" ;;
    Darwin-x86_64)  asset_platform="x86_64-macos" ;;
    Darwin-arm64)   asset_platform="arm64-macos" ;;
    *)
        echo "install-binaryen: no binaryen release for $(uname -s) $(uname -m)" >&2
        exit 1
        ;;
esac

release="version_${BINARYEN_VERSION}"
asset="binaryen-${release}-${asset_platform}.tar.gz"
url="https://github.com/WebAssembly/binaryen/releases/download/${release}/${asset}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

echo "Fetching ${asset}"
curl -fsSL --retry 3 -o "$tmp/binaryen.tar.gz" "$url"
tar -xzf "$tmp/binaryen.tar.gz" -C "$tmp"

SUDO=""
[ "$(id -u)" -eq 0 ] || SUDO="sudo"

# wasm-opt is dynamically linked against libbinaryen in the release, so the
# library ships beside it.
$SUDO mkdir -p "$BINARYEN_PREFIX/bin" "$BINARYEN_PREFIX/lib"
$SUDO install -m 0755 "$tmp/binaryen-${release}/bin/wasm-opt" "$BINARYEN_PREFIX/bin/wasm-opt"
$SUDO cp -a "$tmp/binaryen-${release}/lib/." "$BINARYEN_PREFIX/lib/"

"$BINARYEN_PREFIX/bin/wasm-opt" --version
