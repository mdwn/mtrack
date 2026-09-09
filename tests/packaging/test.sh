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
# Debian package integration test.
#
# Installs the package on the host and checks what the maintainer scripts did.
# Everything here is a case that has to keep working across releases and that
# nothing else covers: `mtrack systemd` has unit tests, but the postinst logic
# that decides *which* unit to render, and whether to overwrite an operator's,
# only exists in the package.
#
# The service is not started -- whether the unit actually runs is
# tests/systemd/'s job. This is about what lands on disk.
#
# Destructive: it installs, reinstalls and purges mtrack, and creates the
# mtrack system user. Intended for CI and throwaway containers.
#
# Usage (via Makefile):
#   make test-deb

set -euo pipefail

DEB="${1-}"
[ -n "$DEB" ] || { echo "usage: $0 <path-to-.deb>" >&2; exit 1; }
[ -f "$DEB" ] || { echo "no such package: $DEB" >&2; exit 1; }
DEB="$(cd "$(dirname "$DEB")" && pwd)/$(basename "$DEB")"

SUDO=""
[ "$(id -u)" -eq 0 ] || SUDO="sudo"

UNIT=/etc/systemd/system/mtrack.service
LIB=/var/lib/mtrack

failures=0
ok()   { echo "    ok  $*"; }
fail() { echo "  FAIL  $*" >&2; failures=$((failures + 1)); }

check() { # check <description> <command...>
    desc="$1"; shift
    if "$@" >/dev/null 2>&1; then ok "$desc"; else fail "$desc"; fi
}

grep_unit() { # grep_unit <description> <pattern>
    if grep -qE "$2" "$UNIT" 2>/dev/null; then ok "$1"; else fail "$1 (no /$2/ in $UNIT)"; fi
}

echo "== package metadata =="
control="$(dpkg-deb -f "$DEB")"
case "$control" in
    # The alternation is what keeps one package installable on both sides of
    # the libasound2 -> libasound2t64 time_t rename. A regression here would
    # only show up as an install failure on a distro CI does not run.
    *"libasound2t64 | libasound2"*) ok "depends carry the libasound2t64 alternation" ;;
    *) fail "depends lost the libasound2t64 alternation: $(echo "$control" | grep -i '^Depends:')" ;;
esac
check "recommends olad for DMX" sh -c "echo '$control' | grep -qE '^Recommends:.*\bola\b'"
check "conffile registered" sh -c "dpkg-deb --ctrl-tarfile '$DEB' | tar xO ./conffiles | grep -qx /etc/default/mtrack"

echo
echo "== install =="
# --allow-downgrades because a machine with the mtrack apt archive configured
# -- a maintainer's own Pi, most likely -- can hold a version that makes the
# package under test look like a downgrade, which apt refuses under -y.
$SUDO apt-get install -y --allow-downgrades --no-install-recommends "$DEB" >/dev/null
check "binary installed" test -x /usr/bin/mtrack
check "service account exists" getent passwd mtrack
check "service account is in the audio group" sh -c "id -nG mtrack | tr ' ' '\n' | grep -qx audio"
check "project directory created" test -d "$LIB"
if [ "$(stat -c %U "$LIB")" = "mtrack" ]; then ok "project directory owned by mtrack"
else fail "project directory owned by $(stat -c %U "$LIB"), not mtrack"; fi

echo
echo "== generated unit =="
check "unit written" test -f "$UNIT"
# The whole point of generating rather than shipping the unit: MTRACK_PATH is
# already known, so the strict sandbox is the default rather than the reward
# for remembering an argument.
grep_unit "unit runs the packaged binary" '^ExecStart=/usr/bin/mtrack start'
grep_unit "unit uses the strict sandbox" '^ProtectSystem=strict'
grep_unit "unit excepts the project directory" "^ReadWritePaths=-\"$LIB\""

echo
echo "== upgrade must not clobber an edited unit =="
# The deployment guide tells operators to add RequiresMountsFor= by hand, so
# this is a case that will come up in the field, not a hypothetical.
echo 'RequiresMountsFor=/mnt/nas' | $SUDO tee -a "$UNIT" >/dev/null
out="$($SUDO dpkg -i "$DEB" 2>&1 || true)"
check "operator edit survives reinstall" grep -q "RequiresMountsFor=/mnt/nas" "$UNIT"
case "$out" in
    *"has local changes"*) ok "reinstall reports that it left the unit alone" ;;
    *) fail "reinstall did not report skipping the edited unit" ;;
esac

echo
echo "== an unedited unit is re-rendered =="
$SUDO rm -f "$UNIT" /var/lib/mtrack-packaging/mtrack.service.md5
$SUDO dpkg -i "$DEB" >/dev/null 2>&1
grep_unit "unit regenerated when absent" '^ProtectSystem=strict'
$SUDO dpkg -i "$DEB" >/dev/null 2>&1
check "second reinstall leaves a matching unit in place" grep -q '^ProtectSystem=strict' "$UNIT"

echo
echo "== a stale /usr/local/bin install must not capture ExecStart =="
# The installation guide has recommended /usr/local/bin for years, and it comes
# first on a default PATH -- resolving the binary through PATH would hand anyone
# upgrading from a tarball a unit pointing at their old copy.
$SUDO cp /usr/bin/mtrack /usr/local/bin/mtrack
$SUDO rm -f "$UNIT" /var/lib/mtrack-packaging/mtrack.service.md5
out="$($SUDO dpkg -i "$DEB" 2>&1 || true)"
grep_unit "ExecStart still names the packaged binary" '^ExecStart=/usr/bin/mtrack start'
case "$out" in
    *"shadows the packaged"*) ok "the shadowing copy is reported" ;;
    *) fail "no warning about /usr/local/bin/mtrack shadowing the package" ;;
esac
$SUDO rm -f /usr/local/bin/mtrack

echo
echo "== purge must not take the song library with it =="
$SUDO install -o mtrack -g mtrack -m 644 /dev/null "$LIB/precious-song.yaml"
$SUDO apt-get purge -y mtrack >/dev/null
check "library directory survives purge" test -d "$LIB"
check "library contents survive purge" test -f "$LIB/precious-song.yaml"
# Removing the account would strand those files under a uid a reinstall would
# not reallocate.
check "service account survives purge" getent passwd mtrack
check "binary removed" sh -c "! test -e /usr/bin/mtrack"
check "generated unit removed" sh -c "! test -e $UNIT"
check "package state removed" sh -c "! test -e /var/lib/mtrack-packaging"

echo
if [ "$failures" -eq 0 ]; then
    echo "All packaging checks passed."
else
    echo "$failures packaging check(s) failed." >&2
    exit 1
fi
