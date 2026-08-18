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

# Starts the service for a failure case and returns only that attempt's journal.
#
# `reset-failed` because a phase that leaves the unit restart-looping trips
# systemd's start limiter, and the next `systemctl start` is refused outright --
# which reads as "the diagnostic is missing" when the service never ran at all.
#
# The cursor file because two phases sharing `tail -50` see each other's output,
# so one phase's message satisfies the other's assertions.
run_failing_start() {
    systemctl reset-failed mtrack >/dev/null 2>&1 || true
    journalctl -u mtrack --no-pager --cursor-file=/tmp/mtrack.cursor >/dev/null 2>&1 || true
    # Expected to fail, and to keep failing: the point is what it says on the
    # way down, so a non-zero exit must not end the script.
    systemctl start mtrack >/dev/null 2>&1 || true
    sleep 3
    systemctl stop mtrack >/dev/null 2>&1 || true
    journalctl -u mtrack --no-pager --cursor-file=/tmp/mtrack.cursor 2>&1
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
echo "--- Test: A write denied by ownership explains itself (#408) ---"

# The other half of the advice, and the half that cannot be checked by reading
# the code: it names the account the service runs as, taken from $USER, on the
# belief that systemd sets it from the unit's User=. If that belief is wrong
# every real deployment gets the vaguer wording instead of a command to run.
#
# The library stays writable by the unit -- this is ownership, not the sandbox --
# so the library is handed to root and mtrack is left unable to create its
# config in it.
systemctl stop mtrack >/dev/null 2>&1 || true
rm -f "$MTRACK_PATH/mtrack.yaml"
chown -R root:root "$MTRACK_PATH"
chmod 755 "$MTRACK_PATH"

denied_log="$(run_failing_start)"

# The control: if the write somehow succeeded there is no advice to assert.
check "the ownership actually denied the write" \
    bash -c 'grep -qi "permission denied" <<< "$0"' "$denied_log"
check "the failure blames ownership rather than the sandbox" \
    bash -c 'grep -q "owns nothing it did not create" <<< "$0"' "$denied_log"
check "the failure names the service account, so systemd does set USER" \
    bash -c "grep -q 'chown -R mtrack:mtrack $MTRACK_PATH' <<< \"\$0\"" "$denied_log"

echo ""
echo "  What the operator sees:"
echo "$denied_log" | grep -A 2 "Error:" | tail -6 | sed 's/^/    /'

# Hand it back, so the sandbox case below fails for its own reason.
chown -R mtrack:mtrack "$MTRACK_PATH"

echo ""
echo "--- Test: A blocked directory names itself, not its parent (#408) ---"

# The regression guard for the review's worst finding. `songs` is a directory,
# and the advice used to name its *parent* -- so a blocked /var/lib/outside-songs
# told the operator to unseal or chown /var/lib, handing a system account most
# of the machine and defeating the sandbox this message exists to explain.
#
# A directory that failed to be created cannot be inspected to find out it was
# meant to be one, which is why the caller has to say so; nothing but an
# end-to-end run proves the caller actually does.
OUTSIDE_SONGS=/var/lib/outside-songs
rm -rf "$OUTSIDE_SONGS"

# The unit here is the good one: the library is writable, and only the songs
# directory is out of bounds.
cat > "$MTRACK_PATH/mtrack.yaml" <<YAML
songs: $OUTSIDE_SONGS
YAML
chown mtrack:mtrack "$MTRACK_PATH/mtrack.yaml"

outside_log="$(run_failing_start)"

check "the songs directory outside the sandbox was blocked" \
    bash -c 'grep -qi "read-only file system" <<< "$0"' "$outside_log"
check "the failure names the songs directory itself" \
    bash -c "grep -q 'Add $OUTSIDE_SONGS to ReadWritePaths=' <<< \"\$0\"" "$outside_log"
# The finding itself: /var/lib is the parent, and naming it is the bug.
check "the failure does not name the parent directory" \
    bash -c '! grep -q "Add /var/lib to ReadWritePaths=" <<< "$0"' "$outside_log"

echo ""
echo "  What the operator sees:"
echo "$outside_log" | grep -A 2 "Error:" | tail -6 | sed 's/^/    /'

rm -f "$MTRACK_PATH/mtrack.yaml"

echo ""
echo "--- Test: A blocked write explains itself (#408) ---"

# Everything above proves the sandbox works when it is configured correctly.
# This proves the *failure* is diagnosable, which is the part an operator
# actually meets: a unit whose ReadWritePaths names some other directory leaves
# the library read-only, and mtrack then dies on the first config write with
# `Read-only file system (os error 30)` for a directory root owns outright.
#
# Runs last: it replaces the unit file.
# Generated to a temp file and moved into place only on success. Redirecting
# straight at the unit truncates it first, so a generator that failed for any
# reason would leave an empty unit and the four checks below would report a
# missing diagnostic rather than a unit that never generated.
cp /etc/systemd/system/mtrack.service /tmp/mtrack.service.good
if mtrack systemd /var/lib/decoy > /tmp/mtrack.service.decoy; then
    mv /tmp/mtrack.service.decoy /etc/systemd/system/mtrack.service
    systemctl daemon-reload
    rm -f "$MTRACK_PATH/mtrack.yaml"
else
    fail "could not generate a unit for the blocked-write case"
fi

blocked_log="$(run_failing_start)"

# The control: without it, a run where mtrack started *fine* would pass every
# assertion below by never having failed at all.
check "the misconfigured sandbox actually blocked the write" \
    bash -c 'grep -qi "read-only file system" <<< "$0"' "$blocked_log"
check "the failure names ReadWritePaths as the cause" \
    bash -c 'grep -q "ReadWritePaths" <<< "$0"' "$blocked_log"
check "the failure says permissions are not the cause" \
    bash -c 'grep -q "permissions are not the cause" <<< "$0"' "$blocked_log"
# Asserts the regenerate command, not just the path: the path alone appears in
# unrelated INFO lines all over the journal, so that assertion passed with the
# explanation removed entirely.
check "the failure gives the fix, naming the blocked directory" \
    bash -c "grep -q 'Add $MTRACK_PATH to ReadWritePaths=' <<< \"\$0\"" "$blocked_log"

# Printed whether or not the checks passed: the assertions above match
# substrings, and a mangled message -- newlines escaped, the path missing --
# satisfies them while being no use to the operator who has to read it.
echo ""
echo "  What the operator sees:"
echo "$blocked_log" | grep -A 4 "Error:" | tail -12 | sed 's/^/    /'

# Put the working unit back. Without this the case has to stay last forever,
# and a second run of this script in the same container fails the installation
# checks against the decoy unit it left behind.
mv /tmp/mtrack.service.good /etc/systemd/system/mtrack.service
systemctl daemon-reload

echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="

if [ "$FAIL" -gt 0 ]; then
    echo ""
    echo "Journal output:"
    journalctl -u mtrack --no-pager 2>&1 | tail -30 | sed 's/^/    /'
    exit 1
fi
