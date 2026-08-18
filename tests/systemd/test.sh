#!/bin/bash
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
#
# Systemd integration test for mtrack.
#
# Verifies that the generated systemd service:
# 1. Starts successfully under the hardened security profile
# 2. Can write to the project directory (config, songs, playlists)
# 3. Serves the web UI
#
# This script is run via `docker exec` while systemd is PID 1.

set -uo pipefail

MTRACK_PATH="${MTRACK_PATH:-/var/lib/mtrack}"
PASS=0
FAIL=0

pass() {
    echo "  PASS: $1"
    PASS=$((PASS + 1))
}

fail() {
    echo "  FAIL: $1"
    FAIL=$((FAIL + 1))
}

check() {
    local desc="$1"
    shift
    if "$@" >/dev/null 2>&1; then
        pass "$desc"
    else
        fail "$desc"
    fi
}

# Asserts a pattern is absent from a file that must exist.
#
# `! grep -q` alone reports success when the file is missing, because grep exits
# 2 rather than 1 — so a build that stopped generating the unit produced one
# honest failure and several spurious passes.
absent_from() {
    local pattern="$1" file="$2"
    test -f "$file" && ! grep -q "$pattern" "$file"
}

echo "=== mtrack systemd integration test ==="
echo ""
echo "--- Test: Service installation ---"

check "service file exists" test -f /etc/systemd/system/mtrack.service
# Generated with a library path, so the sandbox is the strict one: the whole
# filesystem read-only except the library named by ReadWritePaths. Without a
# path the unit falls back to ProtectSystem=full, which is covered by the unit
# tests — here the point is that strict actually starts and writes.
check "service file uses ProtectSystem=strict" grep -q "^ProtectSystem=strict" /etc/systemd/system/mtrack.service
check "service file does not fall back to ProtectSystem=full" absent_from "^ProtectSystem=full" /etc/systemd/system/mtrack.service 
check "service file does not contain ProtectHome" absent_from "ProtectHome" /etc/systemd/system/mtrack.service
check "environment file exists" test -f /etc/default/mtrack
check "environment file sets MTRACK_PATH" grep -q "MTRACK_PATH=$MTRACK_PATH" /etc/default/mtrack

# The unit is generated with the library path, so it must say the mtrack user
# needs access to it and declare it writable (#351). A freshly created system
# user owns none of the library, and nothing used to point at permissions when
# the service then failed.
check "service file documents the library permissions" grep -q "read/write access" /etc/systemd/system/mtrack.service
check "service file names the library in the chown hint" grep -q "chown -R mtrack:mtrack \"$MTRACK_PATH\"" /etc/systemd/system/mtrack.service
check "service file declares the library writable" grep -q "ReadWritePaths=-\"$MTRACK_PATH\"" /etc/systemd/system/mtrack.service
check "service file has no unrendered placeholders" absent_from "{{" /etc/systemd/system/mtrack.service 

echo ""
echo "--- Test: Service startup ---"

systemctl start mtrack

# Give mtrack a moment to initialize.
sleep 3

check "service is active" systemctl is-active mtrack
check "service did not fail" bash -c '! systemctl is-failed mtrack'

# Show service status for debugging.
echo ""
echo "  Service status:"
systemctl status mtrack --no-pager 2>&1 | sed 's/^/    /'

echo ""
echo "--- Test: Write access ---"

check "mtrack.yaml was created" test -f "$MTRACK_PATH/mtrack.yaml"
check "project directory is owned by mtrack" bash -c "stat -c '%U' '$MTRACK_PATH' | grep -q mtrack"

echo ""
echo "--- Test: Web UI ---"

check "web UI responds on port 8080" curl -sf -o /dev/null http://127.0.0.1:8080/
check "web UI serves HTML" bash -c "curl -sf http://127.0.0.1:8080/ | grep -q '<html'"

echo ""
echo "--- Test: API access ---"

check "status API responds" curl -sf -o /dev/null http://127.0.0.1:8080/api/status
check "songs API responds" curl -sf -o /dev/null http://127.0.0.1:8080/api/songs

echo ""
echo "--- Test: Service stop ---"

systemctl stop mtrack
check "service stopped cleanly" bash -c '! systemctl is-active mtrack'

echo ""
echo "--- Test: A blocked write explains itself (#408) ---"

# Everything above proves the sandbox works when it is configured correctly.
# This proves the *failure* is diagnosable, which is the part an operator
# actually meets: a unit whose ReadWritePaths names some other directory leaves
# the library read-only, and mtrack then dies on the first config write with
# `Read-only file system (os error 30)` for a directory root owns outright.
#
# Runs last: it replaces the unit file.
mtrack systemd /var/lib/decoy > /etc/systemd/system/mtrack.service
systemctl daemon-reload
rm -f "$MTRACK_PATH/mtrack.yaml"

# Expected to fail, and to keep failing -- the point is what it says on the way
# down, so don't let a non-zero exit end the script.
systemctl start mtrack >/dev/null 2>&1 || true
sleep 3
systemctl stop mtrack >/dev/null 2>&1 || true

blocked_log="$(journalctl -u mtrack --no-pager 2>&1 | tail -50)"

# The control: without it, a run where mtrack started *fine* would pass every
# assertion below by never having failed at all.
check "the misconfigured sandbox actually blocked the write" \
    bash -c 'grep -qi "read-only file system" <<< "$0"' "$blocked_log"
check "the failure names ReadWritePaths as the cause" \
    bash -c 'grep -q "ReadWritePaths" <<< "$0"' "$blocked_log"
check "the failure says permissions are not the problem" \
    bash -c 'grep -q "permissions are not the problem" <<< "$0"' "$blocked_log"
check "the failure names the directory to add" \
    bash -c "grep -q '$MTRACK_PATH' <<< \"\$0\"" "$blocked_log"

if [ "$FAIL" -gt 0 ]; then
    echo ""
    echo "  Journal from the blocked-write phase:"
    echo "$blocked_log" | tail -25 | sed 's/^/    /'
fi

echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="

if [ "$FAIL" -gt 0 ]; then
    echo ""
    echo "Journal output:"
    journalctl -u mtrack --no-pager 2>&1 | tail -30 | sed 's/^/    /'
    exit 1
fi
