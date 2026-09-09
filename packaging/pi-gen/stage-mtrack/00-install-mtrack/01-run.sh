#!/bin/bash -e
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
# Stages the mtrack package into the rootfs for the chroot script that follows.
#
# The package is passed in rather than built here: the image then contains the
# exact binary the release published, so "which build is on this card" has the
# same answer as "which build is in the tarball".

if [ -z "${MTRACK_DEB:-}" ]; then
	echo "MTRACK_DEB is not set -- point it at the .deb to install." >&2
	echo "See packaging/pi-gen/README.md." >&2
	exit 1
fi

if [ ! -f "${MTRACK_DEB}" ]; then
	echo "MTRACK_DEB does not exist: ${MTRACK_DEB}" >&2
	exit 1
fi

# The architecture is worth checking here rather than discovering it as a
# confusing dpkg error deep inside the chroot.
deb_arch="$(dpkg-deb -f "${MTRACK_DEB}" Architecture)"
if [ "${deb_arch}" != "arm64" ]; then
	echo "MTRACK_DEB is an ${deb_arch} package; a Raspberry Pi image needs arm64." >&2
	exit 1
fi

install -m 644 "${MTRACK_DEB}" "${ROOTFS_DIR}/tmp/mtrack.deb"
