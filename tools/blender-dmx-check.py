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
"""Point a real fixture in Blender DMX with the bytes mtrack sends, and see
where its beam goes (venue-exchange design §18.4, item 4).

Blender DMX is the lighting community's reference GDTF renderer; this
drives it headless as an oracle for mtrack's pointing convention that
shares no code with mtrack. It loads a manufacturer's GDTF, mounts the
fixture where the venue has it, writes the DMX bytes, and reads the beam
emitter's world position and direction back from Blender's own scene
graph, reporting how far the beam passes from the target.

Requires Blender 4.2+ and the BlenderDMX extension installed for the user
(a zip from https://github.com/open-stage/blender-dmx/releases unpacked
into `~/.config/blender/4.2/extensions/user_default/open_stage_blender_dmx`
does). Run:

    blender --background --python tools/blender-dmx-check.py -- \
        --gdtf "lighting/library/Manufacturer@Fixture.gdtf" --mode "Basic" \
        --position 0.759 3.125 7.525 --rotation 0 0 180 --address 61 \
        --bytes 76:139 77:222 78:111 79:227 --target 0 1.5 1.2

`--bytes` are absolute DMX channels (1-based) and values, as mtrack's
`evaluate_show` or a golden test reports them. The script exits non-zero
when the beam misses the target by more than `--tolerance` metres.
"""
import argparse
import math
import os
import shutil
import sys
from types import SimpleNamespace

import addon_utils
import bpy
from mathutils import Euler, Matrix, Vector

MODULE = "bl_ext.user_default.open_stage_blender_dmx"


def parse_args():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    p = argparse.ArgumentParser(prog="blender-dmx-check")
    p.add_argument("--gdtf", required=True, help="path to the .gdtf archive")
    p.add_argument("--mode", required=True, help="the DMX mode's name")
    p.add_argument("--position", nargs=3, type=float, required=True, metavar=("X", "Y", "Z"))
    p.add_argument("--rotation", nargs=3, type=float, default=(0, 0, 0), metavar=("RX", "RY", "RZ"),
                   help="degrees about X, Y, Z applied in that order (the venue's rotation)")
    p.add_argument("--address", type=int, required=True, help="the fixture's DMX address (break 1)")
    p.add_argument("--bytes", nargs="+", required=True, metavar="CH:VAL",
                   help="absolute DMX channel:value pairs to write")
    p.add_argument("--target", nargs=3, type=float, required=True, metavar=("X", "Y", "Z"))
    p.add_argument("--tolerance", type=float, default=0.01, help="metres the beam may miss by")
    return p.parse_args(argv)


def main():
    args = parse_args()
    if addon_utils.enable(MODULE, default_set=True, persistent=True) is None:
        sys.exit(f"Blender DMX is not installed as {MODULE}")
    dmx = bpy.context.scene.dmx
    profiles = os.path.join(dmx.get_addon_path(), "assets", "profiles")
    os.makedirs(profiles, exist_ok=True)
    profile = "mtrack-check.gdtf"
    shutil.copy(args.gdtf, os.path.join(profiles, profile))
    dmx.new()

    position = Vector(args.position)
    rx, ry, rz = (math.radians(a) for a in args.rotation)
    mounting = Matrix.Translation(position) @ Euler((rx, ry, rz), "XYZ").to_matrix().to_4x4()
    breaks = [SimpleNamespace(dmx_break=1, universe=0, address=args.address)]
    dmx.addFixture(profile, args.mode, breaks, [1, 1, 1, 1], True, False,
                   position=mounting, focus_point=None, show_error=False)
    if not dmx.fixtures:
        sys.exit("Blender DMX could not build the fixture; see its log above")
    fixture = dmx.fixtures[0]
    print(f"fixture: {fixture.name} mode {fixture.mode!r}")

    data = sys.modules[MODULE].data.DMX_Data
    for pair in args.bytes:
        channel, value = pair.split(":")
        data.set(0, int(channel), int(value))
    fixture.render()
    bpy.context.view_layer.update()

    target = Vector(args.target)
    worst = None
    for ob in fixture.collection.objects:
        mobile = ob.get("mobile_type")
        if mobile in ("yoke", "head"):
            euler = [round(math.degrees(a), 3) for a in ob.rotation_euler]
            print(f"{mobile}: {ob.name} rotation {euler}")
        if ob.get("geometry_type") == "beam":
            mw = ob.matrix_world
            origin = mw.translation
            direction = (mw.to_3x3() @ Vector((0, 0, -1))).normalized()
            v = target - origin
            along = v.dot(direction)
            miss = (v - along * direction).length
            print(
                f"beam {ob.name}: from {[round(x, 4) for x in origin]} "
                f"along {[round(x, 5) for x in direction]}; passes {miss:.4f} m from the "
                f"target at {along:.2f} m throw"
            )
            worst = miss if worst is None else max(worst, miss)
    if worst is None:
        sys.exit("the fixture has no beam geometry")
    if worst > args.tolerance:
        sys.exit(f"MISS: the beam passes {worst:.4f} m from the target")
    print(f"HIT: the beam passes {worst:.4f} m from the target")


main()
