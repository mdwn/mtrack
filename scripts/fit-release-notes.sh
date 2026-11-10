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
# fit-release-notes.sh -- cut release notes down to what a GitHub release holds.
#
# Usage: fit-release-notes.sh <notes-file> <changelog-url>
#
# A GitHub release body is refused past 125000 characters, and 0.18.0's
# changelog section is 170000. Notes that fit are left alone. Longer ones are
# cut after the last whole changelog entry that fits (a `- ` bullet or a `###`
# heading at the start of a line), and a closing line points at the changelog
# for the rest. The file is rewritten in place.

set -euo pipefail

notes=${1:?usage: fit-release-notes.sh <notes-file> <changelog-url>}
url=${2:?usage: fit-release-notes.sh <notes-file> <changelog-url>}

# Counted in bytes, which are never fewer than characters, with room left for
# the closing line.
limit=${RELEASE_NOTES_LIMIT:-120000}

if [ "$(wc -c < "$notes")" -le "$limit" ]; then
  exit 0
fi

fitted=$(mktemp)
LC_ALL=C awk -v limit="$limit" '
  # A new entry or section starts here: everything before it is whole.
  /^(- |### )/ { whole = kept }
  {
    size += length($0) + 1
    if (size > limit) { exit }
    lines[++kept] = $0
  }
  END {
    # No boundary at all: keep the lines that fit.
    if (whole == 0) { whole = kept }
    for (i = 1; i <= whole; i++) { print lines[i] }
  }
' "$notes" > "$fitted"

printf '\n*These notes are longer than a GitHub release can hold. They continue in the [changelog](%s).*\n' "$url" >> "$fitted"
mv "$fitted" "$notes"
