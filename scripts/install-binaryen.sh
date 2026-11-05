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

# The release assets' SHA-256 digests, as GitHub publishes them for
# version_130. A new version means new digests here.
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)
        asset_platform="x86_64-linux"
        asset_sha256="0a18362361ad05465118cd8eeb72edaeec89de6894bc283576ef4e07aa3babcc" ;;
    Linux-aarch64)
        asset_platform="aarch64-linux"
        asset_sha256="e6ae6e09ac40f4e14bc5be6f687c58e2995c84170013975fa641809dd3b480a0" ;;
    Darwin-x86_64)
        asset_platform="x86_64-macos"
        asset_sha256="d3e2d1235b70c93c54b52eabc1625ea960965152218754f1f4eeb0f873c48e03" ;;
    Darwin-arm64)
        asset_platform="arm64-macos"
        asset_sha256="79d3ab9f417d9e215f15f598f523d001a7d9ac1e59367e5c869fbdabd1cba72e" ;;
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

if [ "$BINARYEN_VERSION" != "130" ]; then
    echo "install-binaryen: no pinned digest for version_${BINARYEN_VERSION}; add it to this script" >&2
    exit 1
fi

echo "Fetching ${asset}"
curl -fsSL --retry 3 -o "$tmp/binaryen.tar.gz" "$url"
# Verified against the pinned digest before anything is unpacked, so a
# changed or substituted asset fails here rather than installing.
if command -v sha256sum >/dev/null; then
    echo "${asset_sha256}  $tmp/binaryen.tar.gz" | sha256sum -c - >/dev/null
else
    echo "${asset_sha256}  $tmp/binaryen.tar.gz" | shasum -a 256 -c - >/dev/null
fi
tar -xzf "$tmp/binaryen.tar.gz" -C "$tmp"

# sudo only when the prefix is not ours to write.
SUDO=""
if [ ! -w "$BINARYEN_PREFIX" ] && [ "$(id -u)" -ne 0 ]; then
    SUDO="sudo"
fi

# wasm-opt is dynamically linked against libbinaryen in the release, so the
# library ships beside it.
$SUDO mkdir -p "$BINARYEN_PREFIX/bin" "$BINARYEN_PREFIX/lib"
$SUDO install -m 0755 "$tmp/binaryen-${release}/bin/wasm-opt" "$BINARYEN_PREFIX/bin/wasm-opt"
$SUDO cp -a "$tmp/binaryen-${release}/lib/." "$BINARYEN_PREFIX/lib/"

"$BINARYEN_PREFIX/bin/wasm-opt" --version
