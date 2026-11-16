// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//
/// The demo's stage: a small invented rig and what it looks like, as the
/// player's `WatchStage` messages (a layout, then frames read against it).
///
/// The rig has no 3D models: no fixture names a `rig` and the venue has no
/// scenery, which is how a real player with none answers. The Lights tab then
/// draws its plot, and the 3D view has nothing to ask for.
library;

import 'dart:math';
import 'dart:typed_data';

import '../../gen/player/v1/player.pb.dart' as player;

/// What the stage shows.
enum DemoLook {
  /// Dark.
  dark,

  /// The ambient show: a steady warm wash.
  ambient,

  /// A song's show: a slow colour chase along the rig.
  chase,
}

/// The id of the layout the demo sends. A stream's first layout is 1.
const int demoLayoutId = 1;

// A fixture's channels, sorted as a layout lists them.
const List<String> _channels = ['blue', 'dimmer', 'green', 'red'];

/// The rig: six pars on a front truss, four uplights along the back. Sorted
/// by name, as a layout lists fixtures.
List<player.StageFixture> _fixtures() {
  player.StageFixture fixture(
    String name,
    String type,
    List<String> tags,
    List<double> position,
    List<double> rotation,
    double beam,
  ) => player.StageFixture(
    name: name,
    type: type,
    tags: tags,
    position: player.StageVec3(x: position[0], y: position[1], z: position[2]),
    rotation: player.StageVec3(x: rotation[0], y: rotation[1], z: rotation[2]),
    beamAngle: beam,
    capabilities: const ['color', 'dimmer'],
    channels: _channels,
  );
  return [
    for (var i = 0; i < 4; i++)
      // Hung on the floor along the back, pointing up.
      fixture(
        'Back ${i + 1}',
        'LED Bar RGB',
        const ['back', 'floor'],
        [-3.0 + i * 2.0, 6.5, 0.4],
        const [180, 0, 0],
        60,
      ),
    for (var i = 0; i < 6; i++)
      // Hung from the front truss, pointing down.
      fixture(
        'Front ${i + 1}',
        'LED Par RGB',
        const ['front', 'truss'],
        [-3.75 + i * 1.5, 2.5, 4.5],
        const [0, 0, 0],
        40,
      ),
  ];
}

/// The layout the demo sends first on every stage stream.
player.StageUpdate demoLayoutUpdate() => player.StageUpdate(
  layout: player.StageLayout(
    id: demoLayoutId,
    fixtures: _fixtures(),
    venue: player.StageVenue(
      name: 'Demo stage',
      focusPoints: {
        'center': player.StageVec3(x: 0, y: 3, z: 0),
        'downstage': player.StageVec3(x: 0, y: 1.5, z: 0),
        'upstage': player.StageVec3(x: 0, y: 6, z: 0),
      }.entries,
    ),
    layers: const ['background', 'midground', 'foreground'],
  ),
);

/// How many fixtures the rig has.
int get demoFixtureCount => 10;

/// The frame for [look] at [seconds] on the demo's clock: every fixture's
/// bytes in the layout's order (blue, dimmer, green, red), end to end.
player.StageUpdate demoFrameUpdate(DemoLook look, double seconds) {
  final bytes = Uint8List(demoFixtureCount * _channels.length);
  for (var i = 0; i < demoFixtureCount; i++) {
    final (r, g, b, dimmer) = switch (look) {
      DemoLook.dark => (0, 0, 0, 0),
      DemoLook.ambient => (255, 170, 70, 90),
      DemoLook.chase => _chase(i, seconds),
    };
    final at = i * _channels.length;
    bytes[at] = b;
    bytes[at + 1] = dimmer;
    bytes[at + 2] = g;
    bytes[at + 3] = r;
  }
  return player.StageUpdate(
    frame: player.StageFrame(
      channels: bytes,
      activeEffects: switch (look) {
        DemoLook.dark => const [],
        DemoLook.ambient => const ['ambient'],
        DemoLook.chase => const ['colour chase'],
      },
      layout: demoLayoutId,
    ),
  );
}

// A slow colour chase: the hue drifts along the rig and round with time, and
// a pulse of brightness runs along it.
(int, int, int, int) _chase(int index, double seconds) {
  final along = index / demoFixtureCount;
  final hue = (seconds / 24 + along * 0.6) % 1;
  final pulse = 0.5 + 0.5 * sin(2 * pi * (seconds * 0.25 - along));
  final level = 0.4 + 0.6 * pulse;
  final (r, g, b) = _hue(hue);
  return (r, g, b, (255 * level).round());
}

(int, int, int) _hue(double hue) {
  int byte(double v) => (255 * v.clamp(0.0, 1.0)).round();
  return (
    byte((6 * hue - 3).abs() - 1),
    byte(2 - (6 * hue - 2).abs()),
    byte(2 - (6 * hue - 4).abs()),
  );
}
