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
# ci-changes.sh -- say which slow CI jobs a pull request's changes call for.
#
# Reads the changed paths on stdin, one per line, relative to the repository
# root, and prints `name=true` or `name=false` lines in the form GitHub Actions
# takes as step outputs:
#
#   ui    the web UI's Playwright tests against the mock server
#   rust  CodeQL's Rust analysis
#
# The web UI is more than src/webui: its rules module is a WebAssembly build of
# crates/mtrack-rules-wasm, which takes files from the player's own source by
# `#[path]`. Those are read from the crate here, so the list cannot fall behind
# it. A file taken that way may declare modules of its own, which live in the
# directory named after it, so that directory counts too.
#
# Run from the repository root. Anything this cannot classify counts as a
# change to everything: a job run for nothing costs minutes, a job skipped
# wrongly costs a broken main.
#
# Usage:
#   git diff --name-only origin/main...HEAD | scripts/ci-changes.sh

set -euo pipefail

WASM_CRATE="crates/mtrack-rules-wasm"

# The player sources the wasm crate includes, as repository paths.
wasm_sources() {
    grep -rhoE '#\[path = "[^"]+"' "$WASM_CRATE/src" |
        sed -E 's/^#\[path = "//; s/"$//; s#^(\.\./)+##' |
        grep '^src/' |
        sort -u
}

ui=false
rust=false

mapfile -t included < <(wasm_sources)
if [ "${#included[@]}" -eq 0 ]; then
    # The crate moved or stopped using #[path]: do not guess.
    echo "ci-changes: no #[path] sources found under $WASM_CRATE" >&2
    ui=true
fi

while IFS= read -r path; do
    [ -n "$path" ] || continue

    case "$path" in
        *.rs | Cargo.toml | Cargo.lock | */Cargo.toml | */Cargo.lock) rust=true ;;
    esac

    case "$path" in
        src/webui/* | src/proto/* | crates/* | Makefile | scripts/* | .github/workflows/*)
            ui=true
            ;;
    esac

    # Either job's own definition changing is a reason to run it.
    case "$path" in
        .github/* | scripts/ci-changes.sh) rust=true ;;
    esac

    for source in "${included[@]}"; do
        if [ "$path" = "$source" ] || [[ "$path" == "${source%.rs}/"* ]]; then
            ui=true
        fi
    done
done

echo "ui=$ui"
echo "rust=$rust"
