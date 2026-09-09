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
# build-apt-repo.sh -- lay out and sign a Debian archive from a set of .debs.
#
# The archive is a plain static file tree, so it can be served from any object
# store or static host. The pool is additive on purpose: every release ever
# published stays addressable, which is what makes `apt install mtrack=0.16.0-1`
# work as a rollback. Callers are expected to seed --out with the existing pool
# before running this, so the regenerated indices describe old and new alike.
#
# Usage:
#   scripts/build-apt-repo.sh --debs DIR --out DIR [--suite NAME] [--sign KEYID]
#
#   --debs DIR    Directory containing the .deb files to add (searched
#                 recursively).
#   --out DIR     Repository root to write. Pre-seed with the existing pool/ to
#                 keep older versions installable.
#   --suite NAME  Archive suite. Default: stable.
#   --sign KEYID  GPG key to sign with. Without it the archive is left unsigned,
#                 which apt will only accept with `[trusted=yes]` -- useful for
#                 local testing, not for anything anyone else installs.

set -euo pipefail

DEBS_DIR=""
OUT_DIR=""
SUITE="stable"
SIGN_KEY=""
ORIGIN="mtrack"
LABEL="mtrack"
COMPONENT="main"

die() { echo "error: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --debs)  DEBS_DIR="${2-}"; shift 2 ;;
        --out)   OUT_DIR="${2-}";  shift 2 ;;
        --suite) SUITE="${2-}";    shift 2 ;;
        --sign)  SIGN_KEY="${2-}"; shift 2 ;;
        --help|-h) sed -n '17,34p' "$0"; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

[ -n "$DEBS_DIR" ] || die "--debs is required"
[ -n "$OUT_DIR" ]  || die "--out is required"
[ -d "$DEBS_DIR" ] || die "--debs directory does not exist: $DEBS_DIR"
command -v apt-ftparchive >/dev/null || die "apt-ftparchive not found (apt install apt-utils)"
command -v dpkg-deb >/dev/null || die "dpkg-deb not found (apt install dpkg)"

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
DEBS_DIR="$(cd "$DEBS_DIR" && pwd)"

# --- Pool ------------------------------------------------------------------
# Debian's pool layout shards on the first letter of the source package name
# (or the first four characters for lib*), which keeps a large archive's
# directory listings manageable. mtrack sorts under m/mtrack.
POOL="$OUT_DIR/pool/$COMPONENT/m/mtrack"
mkdir -p "$POOL"

added=0
while IFS= read -r deb; do
    install -m 644 "$deb" "$POOL/$(basename "$deb")"
    added=$((added + 1))
done < <(find "$DEBS_DIR" -type f -name '*.deb' | sort)

[ "$added" -gt 0 ] || die "no .deb files found under $DEBS_DIR"
echo "Staged $added .deb file(s) into the pool."

# --- Architectures ---------------------------------------------------------
# Taken from what the pool actually holds rather than hard-coded, so adding a
# target to the build matrix does not also mean editing this script.
ARCHES="$(
    find "$OUT_DIR/pool" -type f -name '*.deb' -print0 \
        | xargs -0 -n1 dpkg-deb -f 2>/dev/null \
        | awk '/^Architecture:/ {print $2}' \
        | sort -u
)"
[ -n "$ARCHES" ] || die "could not determine any architecture from the pool"
echo "Architectures: $(echo "$ARCHES" | tr '\n' ' ')"

# --- Indices ---------------------------------------------------------------
# apt-ftparchive writes the pool path it was given straight into the Packages
# `Filename:` field, and apt resolves that relative to the archive root -- so
# this has to run from $OUT_DIR with a relative argument, or every download
# 404s against a path from the build machine.
cd "$OUT_DIR"
rm -rf "dists/$SUITE"

for arch in $ARCHES; do
    dir="dists/$SUITE/$COMPONENT/binary-$arch"
    mkdir -p "$dir"
    apt-ftparchive --arch "$arch" packages pool > "$dir/Packages"
    gzip -9nc "$dir/Packages" > "$dir/Packages.gz"
    count=$(grep -c '^Package:' "$dir/Packages" || true)
    echo "  $arch: $count package(s)"
done

apt-ftparchive \
    -o "APT::FTPArchive::Release::Origin=$ORIGIN" \
    -o "APT::FTPArchive::Release::Label=$LABEL" \
    -o "APT::FTPArchive::Release::Suite=$SUITE" \
    -o "APT::FTPArchive::Release::Codename=$SUITE" \
    -o "APT::FTPArchive::Release::Components=$COMPONENT" \
    -o "APT::FTPArchive::Release::Architectures=$(echo "$ARCHES" | tr '\n' ' ' | sed 's/ $//')" \
    release "dists/$SUITE" > "dists/$SUITE/Release"

# --- Signatures ------------------------------------------------------------
if [ -n "$SIGN_KEY" ]; then
    # InRelease (inline signature) is what current apt fetches; the detached
    # Release.gpg is kept for older clients that still ask for it.
    gpg --batch --yes --local-user "$SIGN_KEY" \
        --clearsign --output "dists/$SUITE/InRelease" "dists/$SUITE/Release"
    gpg --batch --yes --local-user "$SIGN_KEY" \
        --detach-sign --armor --output "dists/$SUITE/Release.gpg" "dists/$SUITE/Release"
    echo "Signed with $SIGN_KEY."
else
    echo "WARNING: archive is unsigned; apt will reject it without [trusted=yes]." >&2
fi

echo "Repository written to $OUT_DIR"
