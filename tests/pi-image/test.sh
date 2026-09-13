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
# Raspberry Pi image inspection.
#
# Mounts the image's root filesystem and checks what the stage was supposed to
# put there. It never boots anything, which is the point: a build that succeeds
# and produces a card that comes up with nothing running is the failure this
# guards against, and it costs seconds rather than an emulator.
#
# What it cannot tell you: whether the thing boots, whether audio comes out, or
# whether a DMX interface works. Those need a Pi.
#
# Usage:
#   tests/pi-image/test.sh <image.img|image.img.xz> [rootfs-mountpoint]

set -euo pipefail

IMAGE="${1-}"
[ -n "$IMAGE" ] || { echo "usage: $0 <image.img|image.img.xz>" >&2; exit 1; }
[ -f "$IMAGE" ] || { echo "no such image: $IMAGE" >&2; exit 1; }

SUDO=""
[ "$(id -u)" -eq 0 ] || SUDO="sudo"

WORK="$(mktemp -d)"
MNT="$WORK/rootfs"
mkdir -p "$MNT"

cleanup() {
    if mountpoint -q "$MNT" 2>/dev/null; then
        $SUDO umount "$MNT" || true
    fi
    rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

failures=0
# Both on stdout, deliberately. Splitting ok and fail across stdout and stderr
# lets the log collector interleave them, and the run that found the bug above
# printed its summary line in the middle of the check list. The exit code is
# what reports the verdict; this output is for a human reading the log in order.
ok()   { echo "    ok  $*"; }
fail() { echo "  FAIL  $*"; failures=$((failures + 1)); }

have() { # have <description> <path-relative-to-rootfs>
    if $SUDO test -e "$MNT/$2"; then ok "$1"; else fail "$1 (missing /$2)"; fi
}

have_link() { # have_link <description> <path-relative-to-rootfs> <expected unit name>
    # Deliberately -L and readlink rather than -e. A systemd enable symlink
    # holds an ABSOLUTE target (/etc/systemd/system/mtrack.service), so inside a
    # mounted image -e resolves it against the HOST root, finds nothing, and
    # reports a correct image as broken. Checking the link itself, and the name
    # it points at, asks the question that can actually be answered from here.
    if ! $SUDO test -L "$MNT/$2"; then
        fail "$1 (no symlink at /$2)"
        return
    fi
    target="$($SUDO readlink "$MNT/$2" 2>/dev/null || true)"
    if [ "$(basename "$target")" = "$3" ]; then
        ok "$1"
    else
        fail "$1 (/$2 points at '$target', expected a link to $3)"
    fi
}

grep_file() { # grep_file <description> <path> <extended-regex>
    if $SUDO grep -qE "$3" "$MNT/$2" 2>/dev/null; then ok "$1"; else fail "$1 (no /$3/ in /$2)"; fi
}

echo "== unpack =="
IMG="$IMAGE"
case "$IMAGE" in
    *.xz)
        IMG="$WORK/image.img"
        # Keep the original: callers upload it after this runs.
        xz -dc "$IMAGE" > "$IMG"
        ok "decompressed $(basename "$IMAGE")"
        ;;
    *) ok "using $(basename "$IMAGE") as-is" ;;
esac

echo
echo "== mount the root filesystem =="
# The partition table is read here rather than shelled out to sfdisk, and the
# filesystem identified by its superblock magic rather than by blkid. Both of
# those live in sbin and are absent from plenty of container images, and when
# they are missing the failure says "command not found" rather than anything
# about the image. An MBR entry and an ext4 superblock are a few bytes at fixed
# offsets; reading them directly costs less than depending on the tools.
#
# A Raspberry Pi OS image is MBR with a FAT boot partition and an ext4 root.
# Find the root by probing each entry rather than assuming it is the second.
root_offset="$(python3 - "$IMG" <<'PYTHON'
import struct, sys

SECTOR = 512
EXT_SUPERBLOCK_OFFSET = 1024      # the superblock starts 1 KiB into the fs
EXT_MAGIC_OFFSET = 0x38           # s_magic, within the superblock
EXT_MAGIC = 0xEF53

with open(sys.argv[1], "rb") as image:
    image.seek(446)               # MBR partition table: 4 x 16-byte entries
    table = image.read(64)
    for entry in range(4):
        # Each entry: status(1) chs(3) type(1) chs(3) lba_start(4) sectors(4)
        kind = table[entry * 16 + 4]
        start = struct.unpack_from("<I", table, entry * 16 + 8)[0]
        if kind == 0 or start == 0:
            continue
        offset = start * SECTOR
        image.seek(offset + EXT_SUPERBLOCK_OFFSET + EXT_MAGIC_OFFSET)
        magic = image.read(2)
        if len(magic) == 2 and struct.unpack("<H", magic)[0] == EXT_MAGIC:
            print(offset)
            break
PYTHON
)"
if [ -z "$root_offset" ]; then
    echo "  FAIL  no ext4 partition found in $(basename "$IMG")" >&2
    echo "        (checked all four MBR entries for an ext4 superblock)" >&2
    exit 1
fi
$SUDO mount -o ro,loop,offset="$root_offset" "$IMG" "$MNT"
ok "mounted the ext4 root at byte offset $root_offset, read-only"

echo
echo "== the player =="
have "mtrack installed" usr/bin/mtrack
if $SUDO file -b "$MNT/usr/bin/mtrack" 2>/dev/null | grep -q "ARM aarch64"; then
    ok "mtrack is an arm64 binary"
else
    fail "mtrack is not arm64: $($SUDO file -b "$MNT/usr/bin/mtrack" 2>/dev/null | cut -c1-60)"
fi

echo
echo "== the service =="
have "unit generated" etc/systemd/system/mtrack.service
grep_file "unit runs the packaged binary" etc/systemd/system/mtrack.service '^ExecStart=/usr/bin/mtrack start'
grep_file "unit uses the strict sandbox" etc/systemd/system/mtrack.service '^ProtectSystem=strict'
# The one that matters most. The package's postinst skips enabling under a
# chroot, so the stage has to do it; if that step is ever dropped the image
# still builds, still contains mtrack, and boots to nothing listening.
have_link "service ENABLED for multi-user" etc/systemd/system/multi-user.target.wants/mtrack.service mtrack.service
have "defaults file present" etc/default/mtrack
grep_file "defaults name the project directory" etc/default/mtrack '^MTRACK_PATH='

echo
echo "== the service account and its library =="
if $SUDO grep -q '^mtrack:' "$MNT/etc/passwd" 2>/dev/null; then ok "mtrack user exists"
else fail "mtrack user missing from /etc/passwd"; fi
if $SUDO grep -qE '^audio:.*[:,]mtrack(,|$)' "$MNT/etc/group" 2>/dev/null; then
    ok "mtrack user is in the audio group"
else
    fail "mtrack user not in audio group (ALSA and MIDI would be inaccessible)"
fi
have "project directory created" var/lib/mtrack
# stat -c %U resolves the numeric owner through the HOST's passwd database
# rather than the image's, so on a machine where that uid belongs to some other
# account it reports a correct image as wrong -- on a GitHub runner it came back
# as 'systemd-timesync'. Same mistake as -e on the enable symlinks: asking the
# host a question only the image can answer. Compare the raw uid against the one
# the image's own /etc/passwd gives mtrack.
want_uid="$($SUDO grep '^mtrack:' "$MNT/etc/passwd" 2>/dev/null | cut -d: -f3 || true)"
have_uid="$($SUDO stat -c %u "$MNT/var/lib/mtrack" 2>/dev/null || echo '?')"
if [ -n "$want_uid" ] && [ "$have_uid" = "$want_uid" ]; then
    ok "project directory owned by mtrack (uid $want_uid)"
else
    fail "project directory is uid '$have_uid'; the image's mtrack user is uid '${want_uid:-unknown}'"
fi

echo
echo "== discovery and lighting =="
# Without avahi the player is only reachable by hunting for its address, which
# on a machine with no monitor is the difference between usable and not.
have_link "avahi enabled" etc/systemd/system/multi-user.target.wants/avahi-daemon.service avahi-daemon.service
grep_file "hostname answers as mtrack" etc/hostname '^mtrack$'
# olad ships a SysV script and no unit, so this is what "enabled" looks like
# for it -- systemd's sysv generator makes olad.service from this at boot.
have_link "olad enabled (SysV)" etc/rc2.d/S01olad olad

echo
if [ "$failures" -eq 0 ]; then
    echo "All image checks passed."
else
    echo "$failures image check(s) failed."
    exit 1
fi
