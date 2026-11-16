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
# Regenerates every raster icon from the SVG masters in design/brand/.
# Needs rsvg-convert (librsvg) and magick (ImageMagick). Run from anywhere;
# commit what it writes.
#
# Not generated here: the Android adaptive icon's vector foreground
# (app/android/app/src/main/res/drawable/ic_launcher_foreground.xml), which
# is the same geometry written by hand.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
brand="$root/design/brand"

png() { # <svg> <pixels> <out>
    rsvg-convert --width "$2" --height "$2" "$1" --output "$3"
}

# Android, for launchers older than adaptive icons (API 25 and below).
res="$root/app/android/app/src/main/res"
for spec in mdpi:48 hdpi:72 xhdpi:96 xxhdpi:144 xxxhdpi:192; do
    png "$brand/mtrack-icon-rounded.svg" "${spec#*:}" "$res/mipmap-${spec%%:*}/ic_launcher.png"
done

# iOS: square, full bleed, no alpha (the system rounds the corners).
ios="$root/app/ios/Runner/Assets.xcassets/AppIcon.appiconset"
for spec in 20x20@1x:20 20x20@2x:40 20x20@3x:60 29x29@1x:29 29x29@2x:58 \
    29x29@3x:87 40x40@1x:40 40x40@2x:80 40x40@3x:120 60x60@2x:120 60x60@3x:180 \
    76x76@1x:76 76x76@2x:152 83.5x83.5@2x:167 1024x1024@1x:1024; do
    out="$ios/Icon-App-${spec%%:*}.png"
    png "$brand/mtrack-icon.svg" "${spec#*:}" "$out"
    magick "$out" -background '#231f20' -alpha remove -alpha off "$out"
done

# The web UI's tab icon and the documentation's.
cp "$brand/mtrack-mark.svg" "$root/src/webui/svelte/public/favicon.svg"
cp "$brand/mtrack-mark.svg" "$root/docs/theme/favicon.svg"
png "$brand/mtrack-icon-rounded.svg" 32 "$root/docs/theme/favicon.png"
