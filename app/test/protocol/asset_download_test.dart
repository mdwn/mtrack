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
import 'dart:math';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as player;
import 'package:mtrack_remote/src/protocol/asset_download.dart';
import 'package:mtrack_remote/src/protocol/player_client.dart';
import 'package:mtrack_remote/src/protocol/player_exception.dart';

import '../support/fake_player.dart';

/// A random source whose `nextInt` always returns [value].
class FixedRandom implements Random {
  FixedRandom(this.value);

  final int value;

  @override
  bool nextBool() => false;

  @override
  double nextDouble() => 0;

  @override
  int nextInt(int max) => value;
}

/// [n] bytes that do not repeat, so gzip cannot make them small.
Uint8List noise(int n) {
  var x = 12345;
  return Uint8List.fromList(
    List<int>.generate(n, (_) {
      x = (x * 1103515245 + 12345) & 0x7fffffff;
      return x >> 16 & 0xff;
    }),
  );
}

void main() {
  late FakePlayer fake;
  late PlayerClient client;

  setUp(() {
    fake = FakePlayer();
    client = PlayerClient(fake, random: FixedRandom(1000));
  });

  tearDown(() async {
    await client.dispose();
  });

  Future<void> pump() => Future<void>.delayed(Duration.zero);

  /// The code [download] failed with.
  Future<int> failure(AssetDownload download) async {
    try {
      await download.bytes;
    } on PlayerException catch (e) {
      return e.code;
    }
    fail('the download succeeded');
  }

  test('asks for the path, gzipped, as a stream', () async {
    final download = client.getLightingAsset('abc/rig-1-v2.json');
    await pump();
    final request = fake.received.single;
    expect(request.method, 'GetLightingAsset');
    final asked = player.GetLightingAssetRequest.fromBuffer(request.payload);
    expect(asked.path, 'abc/rig-1-v2.json');
    expect(asked.gzip, isTrue);
    expect(fake.openStreams, [request.id]);
    await download.cancel();
  });

  test('joins the chunks in order, and tells their progress', () async {
    final progress = <(int, int)>[];
    final download = client.getLightingAsset(
      'abc/models/Base.glb',
      onProgress: (received, total) => progress.add((received, total)),
    );
    await pump();
    final data = noise(1000);
    fake.sendAsset(1000, data, chunk: 300);
    expect(await download.bytes, data);
    expect(progress, [(300, 1000), (600, 1000), (900, 1000), (1000, 1000)]);
    await download.freed;
    expect(fake.openStreams, isEmpty);
    expect(fake.cancels, isEmpty);
  });

  test('unpacks what was sent gzipped', () async {
    final download = client.getLightingAsset('abc/rig-1-v2.json');
    await pump();
    final json = Uint8List.fromList(List.filled(5000, '{"a":1}'.codeUnitAt(1)));
    fake.sendAsset(1000, json, gzip: true, chunk: 50);
    expect(await download.bytes, json);
    expect(download.total, lessThan(200), reason: 'counted as sent');
  });

  test('an empty file is an empty asset', () async {
    final download = client.getLightingAsset('abc/empty.json');
    await pump();
    fake.sendAsset(1000, const []);
    expect(await download.bytes, isEmpty);
  });

  test('one larger than a stream holds for a listener arrives whole', () async {
    final download = client.getLightingAsset('scenery/abc/models/Hall.glb');
    await pump();
    final data = noise(PlayerClient.maxHeldStreamBytes + 50 * 1024);
    fake.sendAsset(1000, data, chunk: 4000);
    expect(await download.bytes, data);
  });

  test('the player not having it is NOT_FOUND', () async {
    final download = client.getLightingAsset('abc/gone.json');
    await pump();
    fake.endStream(1000, code: GrpcCode.notFound, text: 'no such asset');
    expect(await failure(download), GrpcCode.notFound);
    await download.freed;
  });

  test('an older player refuses it as a method it does not allow', () async {
    fake.streamMethods.remove('GetLightingAsset');
    final download = client.getLightingAsset('abc/rig.json');
    final error = await download.bytes.then<Object?>(
      (_) => null,
      onError: (Object e) => e,
    );
    expect((error! as PlayerException).isStreamTooOld, isTrue);
    await download.freed;
  });

  test('the link going mid-file is UNAVAILABLE', () async {
    final download = client.getLightingAsset('abc/models/Head.glb');
    await pump();
    fake.sendAsset(1000, noise(1000), chunk: 300, total: 2000, end: false);
    fake.disconnect();
    expect(await failure(download), GrpcCode.unavailable);
    await download.freed;
  });

  test('a stream that ends short of its total is DATA_LOSS', () async {
    final download = client.getLightingAsset('abc/models/Head.glb');
    await pump();
    fake.sendAsset(1000, noise(1000), total: 2000);
    expect(await failure(download), GrpcCode.dataLoss);
  });

  test('one that runs past its total is DATA_LOSS, and cancelled', () async {
    final download = client.getLightingAsset('abc/models/Head.glb');
    await pump();
    fake.sendAsset(1000, noise(1000), chunk: 600, total: 700, end: false);
    expect(await failure(download), GrpcCode.dataLoss);
    await pump();
    expect(fake.cancels, [1000]);
    await download.freed;
  });

  test('gzip that does not unpack is DATA_LOSS', () async {
    final download = client.getLightingAsset('abc/rig.json');
    await pump();
    final broken = GZipCodec().encode(noise(500)).sublist(0, 100);
    fake.sendAsset(1000, broken);
    expect(await failure(download), GrpcCode.dataLoss);
  });

  group('the size cap', () {
    test('is 64 MiB, the player\'s own', () {
      expect(maxLightingAssetBytes, 64 * 1024 * 1024);
    });

    test('refuses a total over it at once, and cancels', () async {
      final download = client.getLightingAsset('abc/huge.glb');
      await pump();
      fake.sendAsset(
        1000,
        noise(10),
        total: maxLightingAssetBytes + 1,
        end: false,
      );
      expect(await failure(download), GrpcCode.resourceExhausted);
      await pump();
      expect(fake.cancels, [1000]);
      await download.freed;
    });

    test('stops what is sent past it', () async {
      final download = AssetDownload(
        client.openStream('GetLightingAsset', Uint8List(0)),
        maxBytes: 1000,
      );
      await pump();
      fake.sendAsset(1000, noise(1500), chunk: 400, total: 0, end: false);
      expect(await failure(download), GrpcCode.resourceExhausted);
      await download.freed;
    });

    test('stops gzip that unpacks past it, however small', () async {
      final bomb = GZipCodec().encode(Uint8List(5 * 1024 * 1024));
      expect(bomb.length, lessThan(10 * 1024));
      expect(
        () => unpackAsset(Uint8List.fromList(bomb), maxBytes: 1024 * 1024),
        throwsA(
          isA<PlayerException>().having(
            (e) => e.code,
            'code',
            GrpcCode.resourceExhausted,
          ),
        ),
      );
    });
  });

  test('a file not sent gzipped is taken as it is', () {
    final glb = Uint8List.fromList('glTF....'.codeUnits);
    expect(unpackAsset(glb), same(glb));
  });

  group('cancel', () {
    test('fails it as CANCELLED and tells the player', () async {
      final download = client.getLightingAsset('abc/models/Head.glb');
      await pump();
      fake.sendAsset(1000, noise(100), total: 2000, end: false);
      await pump();
      var freed = false;
      final cancelled = download.cancel().then((_) => freed = true);
      expect(await failure(download), GrpcCode.cancelled);
      await pump();
      expect(fake.cancels, [1000]);
      await cancelled;
      expect(freed, isTrue);
      expect(fake.openStreams, isEmpty);
    });

    test('holds the stream until the player lets go of it', () async {
      fake.holdCancels = true;
      final download = client.getLightingAsset('abc/models/Head.glb');
      await pump();
      var freed = false;
      unawaited(download.cancel().then((_) => freed = true));
      await pump();
      await pump();
      expect(freed, isFalse);
      fake.endStream(1000, code: GrpcCode.cancelled, text: 'cancelled');
      await pump();
      expect(freed, isTrue);
    });

    test('after it is over sends nothing', () async {
      final download = client.getLightingAsset('abc/rig.json');
      await pump();
      fake.sendAsset(1000, noise(10));
      await download.bytes;
      await download.cancel();
      expect(fake.cancels, isEmpty);
    });

    test('happens when the client is disposed of', () async {
      final download = client.getLightingAsset('abc/rig.json');
      await pump();
      await client.dispose();
      expect(await failure(download), GrpcCode.unavailable);
      await download.freed;
    });
  });
}
