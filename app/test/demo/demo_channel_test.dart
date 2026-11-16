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
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/demo/demo_channel.dart';
import 'package:mtrack_remote/src/protocol/player_channel.dart';
import 'package:mtrack_remote/src/protocol/player_client.dart';
import 'package:mtrack_remote/src/protocol/player_exception.dart';
import 'package:mtrack_remote/src/protocol/transport.dart';

/// The methods of `PlayerClient` that the demo is allowed to leave
/// unanswered, because they cannot make sense in a demo. There are none: the
/// demo answers everything the app can ask, and a call added to the client
/// that is not answered fails the walk below. A method added here needs the
/// reason it cannot be answered beside it.
const Set<String> unansweredInTheDemo = {};

/// A channel that records every method it is asked for and how the demo
/// answered, and passes everything on to the demo.
class RecordingChannel implements PlayerChannel {
  RecordingChannel(this.inner);

  final DemoChannel inner;

  /// Every method asked for, with the code of its answer (0 for success).
  final Map<String, Set<int>> answers = {};

  void _note(String method, int code) =>
      (answers[method] ??= <int>{}).add(code);

  @override
  Carrier get carrier => inner.carrier;

  @override
  TransportState get currentState => inner.currentState;

  @override
  Stream<TransportState> get state => inner.state;

  @override
  bool get hasStreams => inner.hasStreams;

  @override
  int get callsInFlight => inner.callsInFlight;

  @override
  Stream<pp.RemoteStatus> get statusStream => inner.statusStream;

  @override
  Future<pp.RemoteStatus> readStatus() => inner.readStatus();

  @override
  Future<Uint8List> call(
    String method,
    Uint8List request, {
    Duration timeout = PlayerChannel.defaultTimeout,
  }) async {
    try {
      final response = await inner.call(method, request, timeout: timeout);
      _note(method, 0);
      return response;
    } on PlayerException catch (e) {
      _note(method, e.code);
      rethrow;
    }
  }

  @override
  Stream<Uint8List> openStream(String method, Uint8List request) {
    final stream = inner.openStream(method, request);
    return stream.transform(
      StreamTransformer.fromHandlers(
        handleData: (data, sink) {
          _note(method, 0);
          sink.add(data);
        },
        handleError: (error, stack, sink) {
          _note(method, error is PlayerException ? error.code : -1);
          sink.addError(error, stack);
        },
      ),
    );
  }

  @override
  Future<void> dispose() => inner.dispose();
}

/// The methods `PlayerClient` calls, as its source names them.
Set<String> methodsOfTheClient() {
  final source = File('lib/src/protocol/player_client.dart').readAsStringSync();
  return {
    for (final match in RegExp(
      r"(?:call|openStream)(?:<\w+>)?\(\s*'([A-Za-z]+)'",
    ).allMatches(source))
      match.group(1)!,
  };
}

/// Waits for [future] in fake time: its value, or its error thrown.
T settled<T>(FakeAsync async, Future<T> future) {
  T? value;
  Object? error;
  var done = false;
  future.then(
    (v) {
      value = v;
      done = true;
    },
    onError: (Object e) {
      error = e;
      done = true;
    },
  );
  async.flushMicrotasks();
  expect(done, isTrue, reason: 'the call did not finish');
  if (error != null) {
    throw error!;
  }
  return value as T;
}

Matcher failsWith(int code) =>
    throwsA(isA<PlayerException>().having((e) => e.code, 'code', code));

void main() {
  test('answers every call PlayerClient can make', () async {
    final channel = RecordingChannel(DemoChannel());
    final client = PlayerClient(channel);
    addTearDown(client.dispose);

    await client.readStatus();
    await client.status();
    await client.getNetworkInfo();
    await client.listPlaylists();
    await client.getPlaylist();
    await client.getPlaylist('all_songs');
    await client.getPlaylistWithPalettes('Short set');
    await client.listSongs();
    await client.listAllSongs();
    await client.getSong('Harbor Lights');
    await client.listLightShows();
    await client.getTrackGains();
    await client.setTrackGain('click', -2);
    await client.setTrackMute('click', false);
    await client.setAmbientMode('auto');
    await client.setLock(false);
    await client.navigateToSong(0, 'Harbor Lights');
    await client.seek(const Duration(seconds: 10));
    await client.seekToSection('Verse');
    await client.playFrom(const Duration(seconds: 5));
    await client.sectionAck();
    await client.stopSectionLoop();
    await client.loopSection('Intro');
    await client.stopSectionLoop();
    await client.stopSamples();
    await client.next();
    await client.previous();
    await client.stop();
    await client.switchToPlaylist('Rehearsal list');
    await client.play();
    await client.stop();
    await client.playSongFrom('Sundial', Duration.zero);
    await client.stop();
    await client.cancelAutoPlay();
    await client.savePlaylist('Walk', ['Sundial'], create: true);
    await client.deletePlaylist('Walk');
    await client.updateSongSettings('Sundial', looping: pp.SongLooping());
    await client.switchToPlaylist('Short set');

    final stage = await client.watchStage().first;
    expect(pp.StageUpdate.fromBuffer(stage).hasLayout(), isTrue);

    // The 3D models: the demo has none, as a real player with none says.
    await expectLater(
      client.getLightingAsset('rigs/par.glb').bytes,
      failsWith(GrpcCode.notFound),
    );

    // Every one of those worked: the demo refused none of them but the
    // asset it does not have.
    final refused = {
      for (final entry in channel.answers.entries)
        if (entry.value.any((code) => code != 0)) entry.key: entry.value,
    };
    expect(refused.keys, ['GetLightingAsset']);
    expect(refused['GetLightingAsset'], {GrpcCode.notFound});

    // And every call the client has is among those walked.
    final named = methodsOfTheClient();
    expect(named.length, greaterThanOrEqualTo(30), reason: '$named');
    final missed = named.difference(channel.answers.keys.toSet());
    expect(
      missed,
      isEmpty,
      reason: 'PlayerClient has calls this walk does not make: add them',
    );
    for (final method in named) {
      if (unansweredInTheDemo.contains(method)) {
        continue;
      }
      expect(
        channel.answers[method],
        isNot(contains(GrpcCode.unimplemented)),
        reason: '$method is not answered by the demo',
      );
    }
  });

  test('answers a method it does not know as a real player does', () async {
    final channel = DemoChannel();
    addTearDown(channel.dispose);
    await expectLater(
      channel.call('NoSuchMethod', Uint8List(0)),
      failsWith(GrpcCode.unimplemented),
    );
    // A stream is not a unary call, and a call is not a stream.
    await expectLater(
      channel.call('WatchStage', Uint8List(0)),
      failsWith(GrpcCode.unimplemented),
    );
    await expectLater(
      channel.openStream('Play', Uint8List(0)).first,
      failsWith(GrpcCode.unimplemented),
    );
    await expectLater(
      channel.openStream('NoSuchStream', Uint8List(0)).toList(),
      failsWith(GrpcCode.unimplemented),
    );
  });

  test(
    'refuses what is not a request, and anything once disposed of',
    () async {
      final channel = DemoChannel();
      await expectLater(
        channel.call('SetLock', Uint8List.fromList([0xff, 0xff, 0xff])),
        failsWith(GrpcCode.invalidArgument),
      );
      expect(channel.carrier, Carrier.demo);
      expect(channel.currentState, TransportState.ready);
      expect(channel.hasStreams, isTrue);
      final states = <TransportState>[];
      channel.state.listen(states.add);
      await channel.dispose();
      expect(channel.currentState, TransportState.disconnected);
      await Future<void>.delayed(Duration.zero);
      expect(states, [TransportState.disconnected]);
      await expectLater(
        channel.call('Status', Uint8List(0)),
        failsWith(GrpcCode.unavailable),
      );
      await expectLater(
        channel.openStream('WatchStage', Uint8List(0)).first,
        failsWith(GrpcCode.unavailable),
      );
      // Disposing again is nothing.
      await channel.dispose();
    },
  );

  group('the stage', () {
    /// Opens the stage on a demo channel in fake time, with the messages it
    /// sends collected.
    void stage(
      void Function(
        FakeAsync async,
        DemoChannel channel,
        PlayerClient client,
        List<pp.StageUpdate> heard,
      )
      body, {
      int maxRate = 10,
    }) {
      fakeAsync((async) {
        final channel = DemoChannel(clock: () => async.elapsed);
        final client = PlayerClient(channel);
        final heard = <pp.StageUpdate>[];
        final subscription = client
            .watchStage(maxRate: maxRate)
            .listen((bytes) => heard.add(pp.StageUpdate.fromBuffer(bytes)));
        async.flushMicrotasks();
        body(async, channel, client, heard);
        unawaited(subscription.cancel());
        unawaited(client.dispose());
        async.flushMicrotasks();
        expect(async.pendingTimers, isEmpty);
      });
    }

    test('sends a layout first, of a small rig with no 3D models', () {
      stage((async, channel, client, heard) {
        final layout = heard.first.layout;
        expect(heard.first.hasLayout(), isTrue);
        expect(layout.id, 1);
        expect(layout.fixtures.length, inInclusiveRange(8, 12));
        final names = layout.fixtures.map((f) => f.name).toList();
        expect(names, [...names]..sort(), reason: 'sorted by name');
        expect(names.toSet(), hasLength(names.length));
        expect(
          layout.fixtures.where((f) => f.tags.contains('truss')),
          isNotEmpty,
        );
        expect(
          layout.fixtures.where((f) => f.tags.contains('back')),
          isNotEmpty,
        );
        for (final fixture in layout.fixtures) {
          expect(fixture.type, isNotEmpty);
          expect(fixture.hasPosition(), isTrue);
          expect(fixture.channels, [...fixture.channels]..sort());
          expect(fixture.channels, containsAll(['red', 'green', 'blue']));
          expect(fixture.hasRig(), isFalse, reason: 'no 3D model to ask for');
        }
        expect(layout.venue.name, isNotEmpty);
        expect(layout.venue.hasScenery(), isFalse);
        expect(layout.layers, isNotEmpty);
      });
    });

    test('follows with a frame read against the layout', () {
      stage((async, channel, client, heard) {
        final layout = heard.first.layout;
        final frame = heard[1].frame;
        expect(heard[1].hasFrame(), isTrue);
        expect(frame.layout, layout.id);
        expect(
          frame.channels.length,
          layout.fixtures.fold<int>(0, (n, f) => n + f.channels.length),
        );
      });
    });

    test('shows the ambient look when stopped, and nothing moves', () {
      stage((async, channel, client, heard) {
        expect(heard[1].frame.activeEffects, ['ambient']);
        expect(heard[1].frame.channels.any((b) => b > 0), isTrue);
        final count = heard.length;
        async.elapse(const Duration(seconds: 3));
        expect(heard, hasLength(count), reason: 'a still stage is quiet');
      });
    });

    test('follows the ambient mode when stopped', () {
      stage((async, channel, client, heard) {
        settled(async, client.setAmbientMode('off'));
        async.elapse(const Duration(milliseconds: 300));
        expect(heard.last.frame.activeEffects, isEmpty);
        expect(heard.last.frame.channels.every((b) => b == 0), isTrue);
        settled(async, client.setAmbientMode('on'));
        async.elapse(const Duration(milliseconds: 300));
        expect(heard.last.frame.activeEffects, ['ambient']);
      });
    });

    test('chases colours while a song plays, at the rate asked for', () {
      stage((async, channel, client, heard) {
        settled(async, client.play());
        final before = heard.length;
        async.elapse(const Duration(seconds: 1));
        final frames = heard.skip(before).toList();
        expect(frames.length, inInclusiveRange(9, 11));
        expect(frames.last.frame.activeEffects, ['colour chase']);
        expect(
          frames.map((f) => f.frame.channels.join(',')).toSet().length,
          greaterThan(5),
          reason: 'the look moves',
        );
        // Over a long while the hue has gone all the way round.
        final reds = <int>{};
        for (var i = 0; i < 300; i++) {
          async.elapse(const Duration(milliseconds: 100));
          reds.add(heard.last.frame.channels[3]);
        }
        expect(reds.length, greaterThan(10));
        settled(async, client.stop());
        async.elapse(const Duration(seconds: 1));
        expect(heard.last.frame.activeEffects, ['ambient']);
      });
    });

    test('sends the player\'s own rate for 0, and no more than its best', () {
      stage((async, channel, client, heard) {
        settled(async, client.play());
        final before = heard.length;
        async.elapse(const Duration(seconds: 1));
        expect(
          heard.length - before,
          inInclusiveRange(demoStageRate - 1, demoStageRate + 1),
        );
      }, maxRate: 0);
      stage((async, channel, client, heard) {
        settled(async, client.play());
        final before = heard.length;
        async.elapse(const Duration(seconds: 1));
        expect(
          heard.length - before,
          inInclusiveRange(demoStageRate - 1, demoStageRate + 1),
        );
      }, maxRate: 1000);
    });

    test('ends with the channel, and leaves no timer', () {
      fakeAsync((async) {
        final channel = DemoChannel(clock: () => async.elapsed);
        Object? ended = 'not yet';
        channel
            .openStream('WatchStage', pp.WatchStageRequest().writeToBuffer())
            .listen((_) {}, onError: (Object e) => ended = e, onDone: () {});
        async.flushMicrotasks();
        unawaited(channel.dispose());
        async.flushMicrotasks();
        expect(ended, isA<PlayerException>());
        expect(async.pendingTimers, isEmpty);
      });
    });
  });

  group('the status', () {
    test('is pushed to the channel\'s listeners as the player changes', () {
      fakeAsync((async) {
        final channel = DemoChannel(clock: () => async.elapsed);
        final client = PlayerClient(channel);
        final heard = <pp.RemoteStatus>[];
        client.statusStream.listen(heard.add);
        settled(async, client.play());
        async.elapse(const Duration(seconds: 3));
        expect(heard.length, greaterThanOrEqualTo(4));
        expect(heard.last.playing, isTrue);
        expect(heard.last.elapsed.toDart(), const Duration(seconds: 3));
        expect(settled(async, client.readStatus()).playing, isTrue);
        unawaited(client.dispose());
        async.flushMicrotasks();
        expect(async.pendingTimers, isEmpty);
      });
    });

    test('is streamed by WatchStatus, first at once', () {
      fakeAsync((async) {
        final channel = DemoChannel(clock: () => async.elapsed);
        final heard = <pp.RemoteStatus>[];
        final subscription = channel
            .openStream('WatchStatus', pp.WatchStatusRequest().writeToBuffer())
            .listen((b) => heard.add(pp.RemoteStatus.fromBuffer(b)));
        async.flushMicrotasks();
        expect(heard, hasLength(1));
        settled(async, PlayerClient(channel).setLock(true));
        async.flushMicrotasks();
        expect(heard.last.locked, isTrue);
        unawaited(subscription.cancel());
        unawaited(channel.dispose());
        async.flushMicrotasks();
        expect(async.pendingTimers, isEmpty);
      });
    });
  });
}
