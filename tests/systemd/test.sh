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

echo "=== mtrack systemd integration test ==="
echo ""
echo "--- Test: Service installation ---"

check "service file exists" test -f /etc/systemd/system/mtrack.service
check "service file uses ProtectSystem=full" grep -q "ProtectSystem=full" /etc/systemd/system/mtrack.service
check "service file does not contain ProtectHome" bash -c '! grep -q "ProtectHome" /etc/systemd/system/mtrack.service'
check "environment file exists" test -f /etc/default/mtrack
check "environment file sets MTRACK_PATH" grep -q "MTRACK_PATH=$MTRACK_PATH" /etc/default/mtrack

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
echo "=== Results: $PASS passed, $FAIL failed ==="

if [ "$FAIL" -gt 0 ]; then
    echo ""
    echo "Journal output:"
    journalctl -u mtrack --no-pager 2>&1 | tail -30 | sed 's/^/    /'
    exit 1
fi
