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
# Runs the hardware end-to-end suite against whatever hardware this machine has.
#
# The suite probes for audio devices, MIDI ports, DMX, and loopback cabling,
# runs the areas it can, and reports the ones it cannot. A machine with only
# audio, or only MIDI, is a normal run rather than a failure.
#
# USAGE-BEGIN
# Usage:
#   ./scripts/hardware-test.sh                  # everything available
#   ./scripts/hardware-test.sh --only lighting  # one area or case-name filter
#   ./scripts/hardware-test.sh --list           # what would run, then exit
#   ./scripts/hardware-test.sh --self-test      # prove every check can fail
#   ./scripts/hardware-test.sh --repeat 20      # repeat, to hunt intermittents
#   ./scripts/hardware-test.sh --rediscover     # re-measure cabling, ignore cache
#   ./scripts/hardware-test.sh --probe-all      # probe every device pair, not just the selected one
#   ./scripts/hardware-test.sh --json out.json  # also write machine-readable results
#   ./scripts/hardware-test.sh --no-build       # skip the build step
# USAGE-END

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

FILTER=""
REPEAT=1
LIST_ONLY=false
SELF_TEST=false
SKIP_BUILD=false

# Printed from a marked block rather than a line range: the range silently
# dropped --no-build the moment a usage line was inserted above it.
usage() {
    sed -n '/^# USAGE-BEGIN$/,/^# USAGE-END$/p' "${BASH_SOURCE[0]}" \
        | grep -v 'USAGE-\(BEGIN\|END\)' | sed 's/^# \{0,1\}//'
    exit "${1:-0}"
}

# `shift 2` fails and shifts nothing when only one argument remains, and there
# is no `set -e`, so a bare `--only` used to spin forever. Options taking a
# value check for one explicitly and shift twice.
need_value() {
    if [[ $# -lt 2 ]]; then
        echo "Option $1 requires a value." >&2
        usage 1
    fi
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --only)
            need_value "$@"
            if [[ -z "$2" ]]; then
                echo "--only needs a non-empty filter." >&2
                usage 1
            fi
            FILTER="$2"; shift; shift ;;
        --repeat)
            need_value "$@"
            # Validated here: [[ -gt ]] arithmetic-evaluates, so "abc" and "0"
            # would silently run once and "1.5" would print a bash syntax error
            # and carry on.
            if [[ ! "$2" =~ ^[1-9][0-9]*$ ]]; then
                echo "--repeat needs a positive whole number, got '$2'." >&2
                usage 1
            fi
            REPEAT="$2"; shift; shift ;;
        --json)       need_value "$@"; JSON_OUT="$2"; shift; shift ;;
        --list)       LIST_ONLY=true; shift ;;
        --self-test)  SELF_TEST=true; shift ;;
        --rediscover) export MTRACK_E2E_REDISCOVER=1; shift ;;
        --probe-all)  export MTRACK_E2E_PROBE_ALL=1; shift ;;
        --no-build)   SKIP_BUILD=true; shift ;;
        -h|--help)    usage 0 ;;
        *)            echo "Unknown option: $1" >&2; usage 1 ;;
    esac
done

cd "$PROJECT_ROOT" || { echo "Cannot enter $PROJECT_ROOT" >&2; exit 1; }

# The player serves the web UI from src/webui/svelte/dist, which cargo does not
# build. On the test rig it was two months old at a bless on 2026-09-28: the
# Stage 3D page answered "Not Found", and nothing in the report said so. Build
# it where npm exists (make skips it when the sources are unchanged), and
# wherever the stamp make leaves does not match the sources, say so loudly at
# both ends of the report. The checks do not drive the UI, so this warns
# rather than fails: the UI simply is not what this run vetted.
SVELTE_DIR="$PROJECT_ROOT/src/webui/svelte"
# The same hash as the Makefile's FRONTEND_HASH, so its stamp compares:
# generated src/gen left out, as it is there.
ui_hash() {
    find "$SVELTE_DIR/src" "$PROJECT_ROOT/src/proto" -type f -not -path "$SVELTE_DIR/src/gen/*" \
        2>/dev/null | sort | xargs cat 2>/dev/null | git hash-object --stdin
}
if [[ "$SKIP_BUILD" != "true" ]] && command -v npm >/dev/null 2>&1; then
    echo "=== Building the web UI ==="
    if ! make -C "$PROJECT_ROOT" build-ui; then
        echo "Web UI build failed." >&2
        exit 1
    fi
    echo
fi
UI_WARNING=""
if [[ "$LIST_ONLY" != "true" && "$(cat "$SVELTE_DIR/dist/.build-stamp" 2>/dev/null)" != "$(ui_hash)" ]]; then
    UI_WARNING="WARNING: the web UI the player serves ($SVELTE_DIR/dist) was not built
from this tree's sources, or its provenance is unknown (no .build-stamp). This run
vetted the player, not its web UI. On a machine with npm, run 'make build-ui' and
copy src/webui/svelte/dist, .build-stamp included, into this checkout."
    printf '%s\n\n' "$UI_WARNING" >&2
fi

if [[ "$SKIP_BUILD" != "true" ]]; then
    echo "=== Building mtrack and the harness ==="
    # The harness runs the real binary, so both must be current.
    # `if ! A && B` parses as `(! A) && B`: when A succeeded the negation was
    # false and && short-circuited, so the harness was never built at all.
    if ! cargo build --bin mtrack -p mtrack || ! cargo build -p mtrack-harness; then
        echo "Build failed." >&2
        exit 1
    fi
    echo
fi

# The harness binary: what was just built, or under --no-build the newest of
# debug and release — the same rule the harness applies to the player. It
# used to prefer release whenever one existed, and on 2026-09-20 a two-day-old
# release harness judged a fresh player by an expectation the design had since
# replaced, and ran one check fewer than the code had; a plausible report
# about the wrong judge. Its path and age are printed so that is visible.
HARNESS="$PROJECT_ROOT/target/debug/mtrack-harness"
if [[ "$SKIP_BUILD" == "true" ]]; then
    newest=""
    for candidate in "$PROJECT_ROOT/target/release/mtrack-harness" "$PROJECT_ROOT/target/debug/mtrack-harness"; do
        [[ -x "$candidate" ]] || continue
        if [[ -z "$newest" || "$candidate" -nt "$newest" ]]; then
            newest="$candidate"
        fi
    done
    [[ -n "$newest" ]] && HARNESS="$newest"
fi
if [[ -x "$HARNESS" ]]; then
    harness_age=$(( ( $(date +%s) - $(stat -c %Y "$HARNESS") ) / 60 ))
    echo "  harness: $HARNESS (${harness_age} minute(s) old)"
fi

# Pin the player to what this script just built. Under --no-build there is
# nothing to pin to, so the harness resolves it itself -- newest of debug and
# release, and it prints which one it chose.
#
# This used to leave MTRACK_BIN unset under --no-build while the harness
# preferred release unconditionally, so a stale release binary shadowed a debug
# one built minutes earlier and every check silently reported on the wrong
# code. That cost an afternoon of chasing a defect that did not exist.
if [[ "$SKIP_BUILD" != "true" ]]; then
    export MTRACK_BIN="${MTRACK_BIN:-$PROJECT_ROOT/target/debug/mtrack}"
fi

ARGS=()
[[ -n "$FILTER" ]] && ARGS+=(--only "$FILTER")
[[ "$REPEAT" -gt 1 ]] && ARGS+=(--repeat "$REPEAT")
[[ -n "${JSON_OUT:-}" ]] && ARGS+=(--json "$JSON_OUT")

if [[ "$LIST_ONLY" == "true" ]]; then
    exec "$HARNESS" --list "${ARGS[@]}"
fi

# Not exec'd, so a stale web UI is repeated under the report, where it is read.
if [[ "$SELF_TEST" == "true" ]]; then
    "$HARNESS" --self-test "${ARGS[@]}"
else
    "$HARNESS" "${ARGS[@]}"
fi
status=$?
[[ -n "$UI_WARNING" ]] && printf '\n%s\n' "$UI_WARNING" >&2
exit "$status"
