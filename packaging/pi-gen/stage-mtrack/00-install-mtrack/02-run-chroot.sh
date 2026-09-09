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
# Installs mtrack into the image and makes sure it comes up on boot.

# apt rather than dpkg so the dependencies resolve. They are already present --
# 00-packages-nr pulled them in -- but letting apt confirm that is cheaper than
# finding out from a half-configured package.
apt-get install -y --no-install-recommends /tmp/mtrack.deb
rm -f /tmp/mtrack.deb

# The package deliberately does not enable the service unless it is installed
# under a booted systemd: /run/systemd/system is how its postinst tells a real
# machine from a chroot or container, and enabling things inside an image build
# would otherwise be an error. An image build is exactly that case, so the
# service is installed here but not enabled, and this is the step that arms it.
# Without this the image boots with mtrack present and nothing running.
systemctl enable mtrack.service

# Discovery matters more than usual on a box with no monitor: this is what makes
# http://mtrack.local:8080 resolve. The package only suggests avahi, since
# pulling mDNS onto someone's desktop uninvited would be presumptuous -- on an
# appliance image it is the difference between a usable machine and a hunt
# through the router's DHCP table.
systemctl enable avahi-daemon.service

# olad is deliberately NOT enabled here. The ola package ships a SysV init
# script and no systemd unit, so there is nothing for `systemctl enable` to act
# on at build time and the call would fail the build; its postinst has already
# run update-rc.d, and systemd's sysv generator synthesises olad.service at
# boot from /etc/init.d/olad.

# A first boot on a fresh card should land on a working player rather than an
# error about a missing directory. The package creates this, but say so plainly
# in case a future reshuffle stops it.
install -d -o mtrack -g mtrack -m 755 /var/lib/mtrack
