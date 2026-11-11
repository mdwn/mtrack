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
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/src/app/asset_cache.dart';

const String rig =
    '0b6f3c1e9a2d4f5e6a7b8c9d0e1f2a3b4c5d6e7f8091a2b3c4d5e6f708192a3b/'
    'rig-3f2e1d0c9b8a7f6e-v2.json';

Uint8List bytesOf(int n, [int fill = 7]) =>
    Uint8List.fromList(List.filled(n, fill));

void main() {
  late Directory root;
  late LightingAssetCache cache;

  setUp(() async {
    root = await Directory.systemTemp.createTemp('asset_cache_test');
    cache = LightingAssetCache(() async => root);
  });

  tearDown(() async {
    await root.delete(recursive: true);
  });

  Directory dir() => Directory(
    '${root.path}${Platform.pathSeparator}${LightingAssetCache.directoryName}',
  );

  List<String> names() =>
      dir().listSync().map((e) => e.uri.pathSegments.last).toList()..sort();

  group('paths', () {
    test('the player\'s own are taken as they are', () {
      for (final path in [
        rig,
        'abc/models/Base.glb',
        'abc/models/Pan Tilt.glb',
        'abc/thumbnail.png',
        'abc/thumbnail.svg',
        'scenery/abc/scene-def-v1.json',
        'scenery/abc/models/Truss_3m-2.glb',
        'abc/models/Head.GLB',
      ]) {
        expect(checkLightingAssetPath(path), path, reason: path);
      }
    });

    test('anything else is refused', () {
      for (final path in [
        '',
        '/abc/rig.json',
        'abc//rig.json',
        '../rig.json',
        'abc/../rig.json',
        'abc/./rig.json',
        './rig.json',
        'abc/.hidden.json',
        'abc/rig.json/',
        r'abc\rig.json',
        'abc/rig%2Fx.json',
        'abc/ rig.json',
        'abc/rig .json ',
        'abc/rig.exe',
        'abc/rig',
        'abc/rig.html',
        'abc/rig.js',
        'abc/ri:g.json',
        'abc/rïg.json',
        'a' * 200 + '.json',
      ]) {
        expect(checkLightingAssetPath(path), isNull, reason: path);
      }
    });

    test('name one file each, in the cache\'s own directory', () {
      expect(
        LightingAssetCache.fileNameFor('abc/models/Pan Tilt.glb'),
        'abc%2Fmodels%2FPan%20Tilt.glb',
      );
      expect(LightingAssetCache.fileNameFor('../x.json'), isNull);
    });
  });

  test('a miss is null, and a write is then a hit', () async {
    expect(await cache.read(rig), isNull);
    await cache.write(rig, bytesOf(10));
    expect(await cache.read(rig), bytesOf(10));
    // Another cache over the same directory: the phone's next launch.
    final again = LightingAssetCache(() async => root);
    expect(await again.read(rig), bytesOf(10));
  });

  test(
    'a write replaces the file whole, and leaves nothing beside it',
    () async {
      await cache.write(rig, bytesOf(10, 1));
      await cache.write(rig, bytesOf(20, 2));
      expect(await cache.read(rig), bytesOf(20, 2));
      expect(names(), [LightingAssetCache.fileNameFor(rig)]);
    },
  );

  test('a temporary file is never read as an asset', () async {
    await dir().create(recursive: true);
    File('${dir().path}/.tmp-1234').writeAsBytesSync(bytesOf(5));
    expect(await cache.read('.tmp-1234'), isNull);
    expect(await cache.read(rig), isNull);
  });

  test('a path that is not an asset\'s is neither read nor written', () async {
    await cache.write('../escape.json', bytesOf(5));
    await cache.write('abc/rig.exe', bytesOf(5));
    expect(await cache.read('../escape.json'), isNull);
    expect(File('${root.path}/escape.json').existsSync(), isFalse);
    expect(dir().existsSync() ? names() : <String>[], isEmpty);
  });

  test('a directory that cannot be made is a miss, never an error', () async {
    final blocked = File('${root.path}/file');
    blocked.writeAsStringSync('in the way');
    final broken = LightingAssetCache(() async => Directory(blocked.path));
    await broken.write(rig, bytesOf(5));
    expect(await broken.read(rig), isNull);
  });

  group('eviction', () {
    Future<void> age(String path, int minutesAgo) async {
      final file = File(
        '${dir().path}${Platform.pathSeparator}'
        '${LightingAssetCache.fileNameFor(path)}',
      );
      await file.setLastModified(
        DateTime.now().subtract(Duration(minutes: minutesAgo)),
      );
    }

    test('keeps the cache under its size, least recently used first', () async {
      cache = LightingAssetCache(() async => root, maxBytes: 250);
      await cache.write('a/one.glb', bytesOf(100));
      await age('a/one.glb', 30);
      await cache.write('a/two.glb', bytesOf(100));
      await age('a/two.glb', 20);
      // Read, so used more lately than two.
      await cache.read('a/one.glb');
      await cache.write('a/three.glb', bytesOf(100));
      expect(await cache.read('a/two.glb'), isNull);
      expect(await cache.read('a/one.glb'), isNotNull);
      expect(await cache.read('a/three.glb'), isNotNull);
    });

    test('never the file just written, however large', () async {
      cache = LightingAssetCache(() async => root, maxBytes: 250);
      await cache.write('a/one.glb', bytesOf(100));
      await age('a/one.glb', 5);
      await cache.write('a/big.glb', bytesOf(400));
      expect(await cache.read('a/big.glb'), hasLength(400));
      expect(await cache.read('a/one.glb'), isNull);
    });

    test('sweeps up an old temporary file, not a new one', () async {
      await dir().create(recursive: true);
      final old = File('${dir().path}/.tmp-old')..writeAsBytesSync([1]);
      await old.setLastModified(
        DateTime.now().subtract(const Duration(hours: 2)),
      );
      final fresh = File('${dir().path}/.tmp-fresh')..writeAsBytesSync([1]);
      await cache.write(rig, bytesOf(5));
      expect(old.existsSync(), isFalse);
      expect(fresh.existsSync(), isTrue);
    });
  });
}
