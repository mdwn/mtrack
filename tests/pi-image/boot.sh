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
# Raspberry Pi image boot check.
#
# test.sh reads what the stage left in the image. This boots it: the image's
# own root filesystem under its own systemd, in a container, and then asks the
# questions only a running system answers -- did the service start inside its
# sandbox and stay up, does the web UI answer, can the player write its project
# directory, did olad's SysV script become a unit that runs.
#
# What it still cannot tell you: anything below the root filesystem. The
# container runs on the host's kernel, so the image's kernel, firmware, boot
# partition and device tree are never exercised, nor is Raspberry Pi Imager's
# first-boot customisation, wifi, audio or a DMX interface. Those need a card
# in a Pi.
#
# Needs an arm64 host (the binaries run natively; nothing here emulates),
# systemd-nspawn (Debian/Ubuntu: systemd-container) and root.
#
# Usage:
#   tests/pi-image/boot.sh <image.img|image.img.xz>

set -euo pipefail

IMAGE="${1-}"
[ -n "$IMAGE" ] || { echo "usage: $0 <image.img|image.img.xz>" >&2; exit 1; }
[ -f "$IMAGE" ] || { echo "no such image: $IMAGE" >&2; exit 1; }

case "$(uname -m)" in
    aarch64|arm64) ;;
    *)
        echo "this boots the image's arm64 userland natively and needs an arm64 host" >&2
        echo "(this one is $(uname -m)); test.sh inspects an image on any host" >&2
        exit 1
        ;;
esac
command -v systemd-nspawn >/dev/null || {
    echo "systemd-nspawn not found -- install systemd-container" >&2
    exit 1
}

SUDO=""
[ "$(id -u)" -eq 0 ] || SUDO="sudo"

# /var/tmp rather than /tmp: the working copy is a few gigabytes, and /tmp is
# memory on a Pi.
WORK="$(mktemp -d -p "${TMPDIR:-/var/tmp}" mtrack-image-boot.XXXXXX)"
ROOT="$WORK/rootfs"
IMG="$WORK/image.img"
MACHINE="mtrack-image-$$"
CONSOLE="$WORK/console.log"
mkdir -p "$ROOT"
booted=0

cleanup() {
    if [ "$booted" -eq 1 ]; then
        $SUDO machinectl terminate "$MACHINE" >/dev/null 2>&1 || true
        # The container owns the mounts until it is gone.
        for _ in $(seq 1 30); do
            $SUDO machinectl show "$MACHINE" >/dev/null 2>&1 || break
            sleep 1
        done
    fi
    if mountpoint -q "$ROOT/boot/firmware" 2>/dev/null; then
        $SUDO umount "$ROOT/boot/firmware" || true
    fi
    if mountpoint -q "$ROOT" 2>/dev/null; then
        $SUDO umount "$ROOT" || true
    fi
    $SUDO rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

failures=0
ok()   { echo "    ok  $*"; }
fail() { echo "  FAIL  $*"; failures=$((failures + 1)); }
note() { echo "        $*"; }
expect() { # expect <actual> <wanted> <what is true when they match> <what to say when not>
    if [ "$1" = "$2" ]; then ok "$3"; else fail "$4"; fi
}

# Runs a command inside the booted image and prints its output.
inside() {
    $SUDO systemd-run --machine="$MACHINE" --wait --pipe --quiet "$@" 2>/dev/null
}
unit_is() { # unit_is <unit> <is-active|is-enabled> -> prints the state
    $SUDO systemctl --machine="$MACHINE" "$2" "$1" 2>/dev/null | head -n 1 || true
}
# The web UI's API, from inside the image. Prints the body; empty on failure.
api() { # api <path> [curl args...]
    local path="$1"
    shift
    inside curl -sS -m 15 "$@" "http://127.0.0.1:8080/api$path" || true
}

echo "== a working copy =="
# Booting writes: machine-id, journals, the player's first config. The image
# under test is an artifact somebody is about to flash or upload, so it is
# never the thing that is booted.
case "$IMAGE" in
    *.xz) xz -dc "$IMAGE" > "$IMG" ;;
    *)    cp --sparse=always "$IMAGE" "$IMG" ;;
esac
ok "copied $(basename "$IMAGE") to $IMG"

echo
echo "== mount it =="
# Same reading of the MBR as test.sh, for the same reason: the tools that
# would do it are absent from plenty of hosts. The root is the ext4 entry and
# the firmware partition is the FAT one.
offsets="$(python3 - "$IMG" <<'PYTHON'
import struct, sys

SECTOR = 512
EXT_MAGIC_AT = 1024 + 0x38
EXT_MAGIC = 0xEF53
FAT_TYPES = {0x0B, 0x0C, 0x0E}

root = boot = boot_size = ""
with open(sys.argv[1], "rb") as image:
    image.seek(446)
    table = image.read(64)
    for entry in range(4):
        kind = table[entry * 16 + 4]
        start, sectors = struct.unpack_from("<II", table, entry * 16 + 8)
        if kind == 0 or start == 0:
            continue
        offset = start * SECTOR
        image.seek(offset + EXT_MAGIC_AT)
        magic = image.read(2)
        if len(magic) == 2 and struct.unpack("<H", magic)[0] == EXT_MAGIC:
            root = offset
        elif kind in FAT_TYPES:
            boot, boot_size = offset, sectors * SECTOR
print(root, boot, boot_size)
PYTHON
)"
read -r root_offset boot_offset boot_size <<<"$offsets"
if [ -z "${root_offset:-}" ]; then
    echo "  FAIL  no ext4 partition found in $(basename "$IMAGE")" >&2
    exit 1
fi
$SUDO mount -o loop,offset="$root_offset" "$IMG" "$ROOT"
ok "mounted the root filesystem, writable (the copy)"
if [ -n "${boot_offset:-}" ]; then
    # fstab mounts the firmware partition by PARTUUID, which a container
    # cannot resolve; with it already in place systemd finds nothing to do.
    $SUDO mount -o loop,offset="$boot_offset",sizelimit="$boot_size" \
        "$IMG" "$ROOT/boot/firmware"
    ok "mounted the firmware partition at /boot/firmware"
fi

echo
echo "== boot =="
# --private-network: the image comes up with its own loopback and nothing
# else, so its 8080 cannot collide with a player on the host and its avahi
# cannot announce a second mtrack.local on the real network.
$SUDO systemd-nspawn --quiet --boot --directory="$ROOT" --machine="$MACHINE" \
    --private-network --console=pipe </dev/null >"$CONSOLE" 2>&1 &
booted=1

# "starting" lasts until the last job settles. With no network that is
# systemd-networkd-wait-online giving up, two minutes in.
state=""
for _ in $(seq 1 80); do
    state="$($SUDO systemctl --machine="$MACHINE" is-system-running 2>/dev/null || true)"
    case "$state" in
        running|degraded) break ;;
    esac
    sleep 5
done
case "$state" in
    running|degraded) ok "booted to a settled system ($state)" ;;
    *)
        fail "the image did not finish booting (state: ${state:-no answer})"
        note "console:"
        tail -n 30 "$CONSOLE" | sed 's/^/          /'
        exit 1
        ;;
esac

echo
echo "== what failed to start =="
# A container has no EEPROM, no partition to grow, no root to remount, no
# console for the first-boot wizards and (here) no network. Those units fail
# in any container and say nothing about the image. Anything else is news.
expected='rpi-eeprom-update.service systemd-firstboot.service systemd-growfs-root.service systemd-remount-fs.service systemd-networkd-wait-online.service NetworkManager-wait-online.service userconfig.service'
unexpected=""
while read -r unit _; do
    [ -n "$unit" ] || continue
    case " $expected " in
        *" $unit "*) ;;
        *) unexpected="$unexpected $unit" ;;
    esac
done < <($SUDO systemctl --machine="$MACHINE" --failed --no-legend --plain 2>/dev/null || true)
if [ -z "$unexpected" ]; then
    ok "nothing failed that a container does not explain"
else
    fail "units failed:$unexpected"
fi

echo
echo "== the player =="
expect "$(unit_is mtrack.service is-enabled)" enabled \
    "mtrack.service is enabled" "mtrack.service is not enabled"
expect "$(unit_is mtrack.service is-active)" active \
    "mtrack.service is running" "mtrack.service is not running"

show() { $SUDO systemctl --machine="$MACHINE" show mtrack.service -p "$1" --value 2>/dev/null || true; }
expect "$(show User)" mtrack "it runs as the mtrack user" "it runs as '$(show User)', not mtrack"
# Restart=on-failure hides a crash loop behind "active". The boot took two
# minutes, which is twenty-odd restart intervals: zero restarts and the same
# process a little later is a service that came up once and stayed.
pid_before="$(show MainPID)"
sleep 8
restarts="$(show NRestarts)"
if [ "$restarts" = 0 ] && [ "$(show MainPID)" = "$pid_before" ] && [ "$pid_before" != 0 ]; then
    ok "it started once and stayed up"
else
    fail "it is restarting (NRestarts=$restarts, pid $pid_before -> $(show MainPID))"
fi

errors="$($SUDO journalctl --machine="$MACHINE" -u mtrack.service --no-pager 2>/dev/null | grep -c ' ERROR ' || true)"
if [ "$errors" = 0 ]; then
    ok "it logged no errors"
else
    fail "it logged $errors error line(s)"
    $SUDO journalctl --machine="$MACHINE" -u mtrack.service --no-pager 2>/dev/null \
        | grep ' ERROR ' | head -n 5 | cut -c1-200 | sed 's/^/          /'
fi

echo
echo "== the web UI =="
status="$(api /status)"
field() { # field <json> <python expression over d>
    python3 -c 'import json,sys
try:
    d = json.loads(sys.argv[1])
    print(eval(sys.argv[2]))
except Exception:
    print("")' "$1" "$2"
}
if [ -n "$status" ]; then
    ok "the API answers on 8080"
else
    fail "the API does not answer on 8080"
fi
ui_version="$(field "$status" 'd["build"]["version"]')"
# Not `-f '${Version}'`: systemd-run expands ${...} in the command it is
# given, and hands dpkg-query an empty format.
pkg_version="$(inside dpkg-query -W mtrack | cut -f2 | tr -d '\r' || true)"
# The package version is the binary's with a Debian revision ("0.17.0-1").
if [ -z "$ui_version" ]; then
    fail "it reports no version"
else
    case "$pkg_version" in
        "$ui_version"|"$ui_version"-*) ok "it reports the package's version ($ui_version)" ;;
        *) fail "it reports version '$ui_version' but the package is '$pkg_version'" ;;
    esac
fi
expect "$(field "$status" 'd["locked"]')" True "it comes up locked" "it did not come up locked"
expect "$(field "$status" 'd["hardware"]["hostname"]')" mtrack \
    "it knows itself as mtrack" "it does not report the hostname mtrack"

shell="$(inside curl -sS -m 15 http://127.0.0.1:8080/ || true)"
asset="$(printf '%s' "$shell" | grep -o 'assets/index-[A-Za-z0-9_-]*\.js' | head -n 1 || true)"
if [ -n "$asset" ]; then
    code="$(inside curl -sS -m 15 -o /dev/null -w '%{http_code}' "http://127.0.0.1:8080/$asset" || true)"
    expect "$code" 200 "the page and its script are served" \
        "the page names $asset but it answers $code"
else
    fail "the page is not served (no script named in /)"
fi

echo
echo "== writing the project =="
# ProtectSystem=strict leaves the player one writable place. A unit that
# names the wrong one starts, serves pages, and fails the first save.
api /lock -X PUT -H 'content-type: application/json' -d '{"locked":false}' >/dev/null
saved="$(api /lighting/fixture-types/BootCheckPar -X PUT -H 'content-type: text/plain' \
    --data-binary 'fixture_type "BootCheckPar" {
  channels: 3
  channel_map: { "red": 1, "green": 2, "blue": 3 }
}')"
expect "$(field "$saved" 'd["status"]')" saved \
    "a fixture type saves" "a fixture type does not save: $saved"
saved="$(api /lighting/venues/boot-check -X PUT -H 'content-type: application/json' \
    -d '{"fixtures":[{"name":"A","fixture_type":"BootCheckPar","universe":1,"start_channel":1,"tags":[]}]}')"
expect "$(field "$saved" 'd["status"]')" saved "a venue saves" "a venue does not save: $saved"
patch="$(api /lighting/venues/boot-check/patch)"
expect "$(field "$patch" 'd["spans"][0]["footprint"]')" 3 \
    "the venue reads back with its fixture resolved" "the venue does not read back: $patch"
owner="$(inside stat -c '%U' /var/lib/mtrack/lighting/venues | tr -d '\r' || true)"
expect "$owner" mtrack "what it wrote is under /var/lib/mtrack, as mtrack" \
    "/var/lib/mtrack/lighting/venues is owned by '$owner'"

echo
echo "== the rest of the image =="
expect "$(unit_is avahi-daemon.service is-active)" active "avahi is running" "avahi is not running"
expect "$(inside hostname | tr -d '\r' || true)" mtrack \
    "the hostname is mtrack" "the hostname is not mtrack"
# olad has no unit of its own: systemd synthesises one from the SysV script at
# boot, which is exactly what an image inspection cannot see happen.
expect "$(unit_is olad.service is-active)" active \
    "olad's SysV script became a unit, and it is running" "olad is not running"
code="$(inside curl -sS -m 10 -o /dev/null -w '%{http_code}' http://127.0.0.1:9090/ || true)"
expect "$code" 200 "olad answers" "olad does not answer (HTTP $code)"
expect "$(unit_is ssh.service is-enabled)" enabled "ssh is enabled" "ssh is not enabled"

echo
if [ "$failures" -eq 0 ]; then
    echo "the image boots to a running player"
else
    echo "$failures check(s) failed"
    note "mtrack's log:"
    $SUDO journalctl --machine="$MACHINE" -u mtrack.service --no-pager -n 25 2>/dev/null \
        | cut -c1-200 | sed 's/^/          /'
    exit 1
fi
