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
# Usage:
#   ./scripts/hardware-test.sh                  # everything available
#   ./scripts/hardware-test.sh --only lighting  # one area or case-name filter
#   ./scripts/hardware-test.sh --list           # what would run, then exit
#   ./scripts/hardware-test.sh --repeat 20      # repeat, to hunt intermittents
#   ./scripts/hardware-test.sh --rediscover     # re-measure cabling, ignore cache
#   ./scripts/hardware-test.sh --probe-all      # probe every device pair, not just the selected one
#   ./scripts/hardware-test.sh --json out.json  # also write machine-readable results
#   ./scripts/hardware-test.sh --no-build       # skip the build step

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

FILTER=""
REPEAT=1
LIST_ONLY=false
SKIP_BUILD=false

usage() {
    sed -n '17,29p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
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
        --rediscover) export MTRACK_E2E_REDISCOVER=1; shift ;;
        --probe-all)  export MTRACK_E2E_PROBE_ALL=1; shift ;;
        --no-build)   SKIP_BUILD=true; shift ;;
        -h|--help)    usage 0 ;;
        *)            echo "Unknown option: $1" >&2; usage 1 ;;
    esac
done

cd "$PROJECT_ROOT"

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

HARNESS="$PROJECT_ROOT/target/debug/mtrack-harness"

# Pin the player only when this script built it. Doing so unconditionally broke
# --no-build on a release-only machine, where the harness's own release-first
# fallback would have found a perfectly good binary.
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

exec "$HARNESS" "${ARGS[@]}"
