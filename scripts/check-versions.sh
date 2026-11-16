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
# check-versions.sh -- fail unless mtrack and mtrack-core are at one version.
#
# The two are released together (.github/workflows/publish.yaml): mtrack-core
# first, then mtrack, which crates.io takes only once the mtrack-core it asks
# for is there. So the player's version, mtrack-core's version and the version
# of mtrack-core the player asks for have to be the same; with a tag given,
# that tag (v<version>) too.
#
# Run from the repository root.
#
# Usage:
#   scripts/check-versions.sh [tag]

set -euo pipefail

version_of() {
    grep -m1 '^version = ' "$1" | cut -d'"' -f2
}

player=$(version_of Cargo.toml)
core=$(version_of crates/mtrack-core/Cargo.toml)
wanted=$(grep -m1 '^mtrack-core = .*version = ' Cargo.toml | sed 's/.*version = "\([^"]*\)".*/\1/')

status=0
if [ "$core" != "$player" ]; then
    echo "crates/mtrack-core is at $core and mtrack at $player: bump them together." >&2
    status=1
fi
if [ "$wanted" != "$player" ]; then
    echo "mtrack asks for mtrack-core $wanted and is itself at $player: bump the dependency too." >&2
    status=1
fi
if [ "$#" -ge 1 ] && [ "v$player" != "$1" ]; then
    echo "Tag $1 does not match Cargo.toml version $player." >&2
    echo "Bump the version (and Cargo.lock) before tagging." >&2
    status=1
fi
[ "$status" -eq 0 ] && echo "mtrack and mtrack-core are at $player."
exit "$status"
