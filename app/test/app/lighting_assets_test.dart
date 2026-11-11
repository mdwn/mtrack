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
import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/src/app/asset_cache.dart';
import 'package:mtrack_remote/src/app/lighting_assets.dart';
import 'package:mtrack_remote/src/protocol/player_client.dart';

import '../support/fake_player.dart';

Uint8List bytesOf(String text) => Uint8List.fromList(text.codeUnits);

void main() {
  late FakePlayer fake;
  late PlayerClient client;
  late Directory root;
  late LightingAssetCache cache;
  late LightingAssets assets;
  var ready = true;

  setUp(() async {
    ready = true;
    fake = FakePlayer();
    client = PlayerClient(fake);
    root = await Directory.systemTemp.createTemp('lighting_assets_test');
    cache = LightingAssetCache(() async => root);
    assets = LightingAssets(
      client: () => client,
      isReady: () => ready,
      cache: cache,
    )..linkChanged(ready: true);
  });

  tearDown(() async {
    assets.dispose();
    await client.dispose();
    await root.delete(recursive: true);
  });

  /// Lets the cache's disk and the queue catch up: until [done] holds, or
  /// a few turns when there is nothing to wait for.
  Future<void> pump([bool Function()? done]) async {
    final deadline = DateTime.now().add(const Duration(seconds: 5));
    while (done != null && !done() && DateTime.now().isBefore(deadline)) {
      await Future<void>.delayed(const Duration(milliseconds: 2));
    }
    for (var i = 0; i < 5; i++) {
      await Future<void>.delayed(Duration.zero);
    }
  }

  /// What [future] failed with, as an [AssetUnavailable]'s reason.
  Future<AssetFailure?> reason(Future<Object?> future) async {
    try {
      await future;
    } on AssetUnavailable catch (e) {
      return e.reason;
    }
    return null;
  }

  test('a miss is fetched and kept; a hit asks the player nothing', () async {
    fake.assets = {'a/rig.json': bytesOf('{"nodes":[]}')};
    expect(await assets.get('a/rig.json'), bytesOf('{"nodes":[]}'));
    expect(fake.assetsAsked, ['a/rig.json']);
    expect(await cache.read('a/rig.json'), bytesOf('{"nodes":[]}'));

    expect(await assets.get('a/rig.json'), bytesOf('{"nodes":[]}'));
    expect(fake.assetsAsked, ['a/rig.json'], reason: 'from the cache');
  });

  test('without a cache, every ask is a fetch', () async {
    assets.dispose();
    assets = LightingAssets(client: () => client, isReady: () => ready)
      ..linkChanged(ready: true);
    fake.assets = {'a/rig.json': bytesOf('{}')};
    await assets.get('a/rig.json');
    await assets.get('a/rig.json');
    expect(fake.assetsAsked, ['a/rig.json', 'a/rig.json']);
  });

  test('one at a time, the next once the player has let go', () async {
    final one = assets.get('a/one.glb');
    final two = assets.get('a/two.glb');
    await pump(() => fake.openStreams.isNotEmpty);
    expect(fake.assetsAsked, ['a/one.glb']);
    final first = fake.openStreams.single;
    fake.sendAsset(first, bytesOf('one'), end: false);
    await pump();
    expect(fake.assetsAsked, ['a/one.glb'], reason: 'one still open');
    fake.endStream(first);
    expect(await one, bytesOf('one'));
    await pump(() => fake.openStreams.isNotEmpty);
    expect(fake.assetsAsked, ['a/one.glb', 'a/two.glb']);
    fake.sendAsset(fake.openStreams.single, bytesOf('two'));
    expect(await two, bytesOf('two'));
  });

  test('two asks for one asset share its fetch', () async {
    fake.assets = {'a/one.glb': bytesOf('one')};
    final results = await Future.wait([
      assets.get('a/one.glb'),
      assets.get('a/one.glb'),
    ]);
    expect(results, [bytesOf('one'), bytesOf('one')]);
    expect(fake.assetsAsked, ['a/one.glb']);
  });

  test('a path that is not an asset\'s is not asked for', () async {
    expect(await assets.get('../etc/passwd.json'), isNull);
    expect(await assets.get('a/rig.exe'), isNull);
    expect(fake.received, isEmpty);
  });

  test('one the player does not have is null, and counted', () async {
    fake.assets = {};
    expect(await assets.get('a/gone.glb'), isNull);
    expect(assets.failures, 1);
    expect(assets.onlyMissing, isTrue);
    assets.clearFailures();
    expect(assets.failures, 0);
  });

  test('says how the fetching goes', () async {
    final told = <String>[];
    assets.addListener(
      () => told.add(
        '${assets.loading} ${assets.done}/${assets.count} ${assets.fraction}',
      ),
    );
    final one = assets.get('a/one.glb');
    final two = assets.get('a/two.glb');
    await pump(() => fake.openStreams.isNotEmpty && assets.count == 2);
    expect(assets.loading, isTrue);
    expect(assets.count, 2);
    expect(assets.fraction, isNull, reason: 'its size not yet known');
    fake.sendAsset(
      fake.openStreams.single,
      List.filled(100, 1),
      chunk: 50,
      end: false,
    );
    await pump();
    expect(assets.fraction, 0.5);
    fake.endStream(fake.openStreams.single);
    await one;
    await pump(() => fake.openStreams.isNotEmpty);
    expect(assets.done, 1);
    fake.sendAsset(fake.openStreams.single, List.filled(10, 1));
    await two;
    expect(assets.loading, isFalse);
    expect(assets.count, 0);
    expect(told, contains('true 0/2 0.25'));
  });

  group('could not be had', () {
    test('from an older player, which is not asked again', () async {
      fake.streamMethods.remove('GetLightingAsset');
      expect(await reason(assets.get('a/one.glb')), AssetFailure.tooOld);
      expect(assets.tooOld, isTrue);
      expect(assets.onlyMissing, isFalse);
      final asked = fake.received.length;
      expect(await reason(assets.get('a/two.glb')), AssetFailure.tooOld);
      expect(fake.received.length, asked);

      // A cached asset is still served.
      await cache.write('a/kept.glb', bytesOf('kept'));
      expect(await assets.get('a/kept.glb'), bytesOf('kept'));

      // The player may have been updated while the link was down.
      assets
        ..linkChanged(ready: false)
        ..linkChanged(ready: true);
      expect(assets.tooOld, isFalse);
    });

    test('while the link is down', () async {
      ready = false;
      expect(await reason(assets.get('a/one.glb')), AssetFailure.unavailable);
      expect(fake.received, isEmpty);
      expect(assets.failures, 1);
    });

    test('when the link goes mid-file, nor what waited', () async {
      final one = reason(assets.get('a/one.glb'));
      final two = reason(assets.get('a/two.glb'));
      await pump(() => fake.openStreams.isNotEmpty);
      fake.sendAsset(
        fake.openStreams.single,
        List.filled(100, 1),
        total: 1000,
        end: false,
      );
      await pump();
      ready = false;
      assets.linkChanged(ready: false);
      fake.disconnect();
      expect(await one, AssetFailure.unavailable);
      expect(await two, AssetFailure.unavailable);
      expect(fake.assetsAsked, ['a/one.glb']);
      expect(assets.failures, 2);
      expect(assets.loading, isFalse);
      expect(await cache.read('a/one.glb'), isNull, reason: 'nothing partial');
    });

    test('when nothing comes for a while', () {
      fakeAsync((async) {
        // Its own player, client and no cache: a disk does not run on fake
        // time.
        final fake = FakePlayer();
        final client = PlayerClient(fake);
        final assets = LightingAssets(client: () => client, isReady: () => true)
          ..linkChanged(ready: true);
        AssetFailure? failed;
        unawaited(reason(assets.get('a/one.glb')).then((r) => failed = r));
        async.flushMicrotasks();
        fake.sendAsset(
          fake.openStreams.single,
          List.filled(100, 1),
          total: 1000,
          end: false,
        );
        async.elapse(assetIdleTimeout - const Duration(seconds: 1));
        expect(failed, isNull);
        async.elapse(const Duration(seconds: 2));
        async.flushMicrotasks();
        expect(failed, AssetFailure.failed);
        expect(fake.cancels, hasLength(1));
        assets.dispose();
        unawaited(client.dispose());
        async.flushMicrotasks();
      });
    });
  });

  test('forgetting the player forgets what was learned of it', () async {
    fake.streamMethods.remove('GetLightingAsset');
    await reason(assets.get('a/one.glb'));
    expect(assets.tooOld, isTrue);
    assets.clear();
    expect(assets.tooOld, isFalse);
    expect(assets.failures, 0);
  });
}
