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
import 'dart:math';
import 'dart:typed_data';

import 'package:protobuf/protobuf.dart';

import '../../gen/player/v1/player.pb.dart' as player;
import '../protocol/player_channel.dart';
import '../protocol/player_exception.dart';
import '../protocol/transport.dart';
import 'demo_player.dart';
import 'demo_stage.dart';

/// The fastest the demo's stage stream sends frames, in frames a second, and
/// what a stream that asks for the player's own rate gets.
const int demoStageRate = 15;

/// A [PlayerChannel] to a [DemoPlayer] inside the app: the third carrier,
/// beside Bluetooth and Wi-Fi. It decodes each call's method and request
/// bytes, has the demo player answer, and encodes the answer, so everything
/// above it speaks to the demo as it speaks to a real player. Nothing it does
/// touches a radio, the network or the disk.
///
/// A method it has no answer for fails with `UNIMPLEMENTED`, as a real
/// player's does.
class DemoChannel implements PlayerChannel {
  /// A channel to a fresh demo player on [clock] (a monotonic clock; the
  /// default is a stopwatch). The channel owns the player and disposes of it.
  DemoChannel({Duration Function()? clock})
    : _player = DemoPlayer(clock: clock) {
    _unary = {
      'Play': _call(player.PlayRequest.fromBuffer, (_) => _player.play()),
      'PlayFrom': _call(
        player.PlayFromRequest.fromBuffer,
        (r) => _player.playFrom(r.startTime.toDart()),
      ),
      'PlaySongFrom': _call(
        player.PlaySongFromRequest.fromBuffer,
        (r) => _player.playSongFrom(r.songName, r.startTime.toDart()),
      ),
      'Seek': _call(
        player.SeekRequest.fromBuffer,
        (r) => _player.seek(r.position.toDart()),
      ),
      'SeekToSection': _call(
        player.SeekToSectionRequest.fromBuffer,
        (r) => _player.seekToSection(r.sectionName),
      ),
      'Previous': _call(
        player.PreviousRequest.fromBuffer,
        (_) => _player.previous(),
      ),
      'Next': _call(player.NextRequest.fromBuffer, (_) => _player.next()),
      'NavigateToSong': _call(
        player.NavigateToSongRequest.fromBuffer,
        (r) => _player.navigateToSong(r.position, r.songName),
      ),
      'Stop': _call(player.StopRequest.fromBuffer, (_) => _player.stop()),
      'SwitchToPlaylist': _call(
        player.SwitchToPlaylistRequest.fromBuffer,
        (r) => _player.switchToPlaylist(r.playlistName),
      ),
      'Status': _call(
        player.StatusRequest.fromBuffer,
        (_) => _player.fullStatus(),
      ),
      'StopSamples': _call(
        player.StopSamplesRequest.fromBuffer,
        (_) => _player.stopSamples(),
      ),
      'LoopSection': _call(
        player.LoopSectionRequest.fromBuffer,
        (r) => _player.loopSection(r.sectionName),
      ),
      'StopSectionLoop': _call(
        player.StopSectionLoopRequest.fromBuffer,
        (_) => _player.stopSectionLoop(),
      ),
      'SectionAck': _call(
        player.SectionAckRequest.fromBuffer,
        (_) => _player.sectionAck(),
      ),
      'SetTrackGain': _call(
        player.SetTrackGainRequest.fromBuffer,
        (r) => _player.setTrackGain(r.track, r.gainDb),
      ),
      'SetTrackMute': _call(
        player.SetTrackMuteRequest.fromBuffer,
        (r) => _player.setTrackMute(r.track, r.muted),
      ),
      'GetTrackGains': _call(
        player.GetTrackGainsRequest.fromBuffer,
        (_) => _player.getTrackGains(),
      ),
      'SetAmbientMode': _call(
        player.SetAmbientModeRequest.fromBuffer,
        (r) => _player.setAmbientMode(r.mode),
      ),
      'CancelAutoPlay': _call(
        player.CancelAutoPlayRequest.fromBuffer,
        (_) => _player.cancelAutoPlay(),
      ),
      'ListPlaylists': _call(
        player.ListPlaylistsRequest.fromBuffer,
        (r) => _player.listPlaylists(r.detailed),
      ),
      'GetPlaylist': _call(
        player.GetPlaylistRequest.fromBuffer,
        (r) => _player.getPlaylist(
          r.name,
          withLibrary: r.withLibrary,
          withPalettes: r.withPalettes,
        ),
      ),
      'SavePlaylist': _call(
        player.SavePlaylistRequest.fromBuffer,
        _player.savePlaylist,
      ),
      'ListLightShows': _call(
        player.ListLightShowsRequest.fromBuffer,
        (_) => _player.listLightShows(),
      ),
      'ListSongs': _call(
        player.ListSongsRequest.fromBuffer,
        (r) => _player.listSongs(r.start, r.detailed),
      ),
      'GetSong': _call(
        player.GetSongRequest.fromBuffer,
        (r) => _player.getSong(r.name),
      ),
      'UpdateSongSettings': _call(
        player.UpdateSongSettingsRequest.fromBuffer,
        _player.updateSongSettings,
      ),
      'DeletePlaylist': _call(
        player.DeletePlaylistRequest.fromBuffer,
        (r) => _player.deletePlaylist(r.name),
      ),
      'SetLock': _call(
        player.SetLockRequest.fromBuffer,
        (r) => _player.setLock(r.locked),
      ),
      'GetNetworkInfo': _call(
        player.GetNetworkInfoRequest.fromBuffer,
        (_) => _player.networkInfo(),
      ),
    };
    _streams = {
      'WatchStage': _watchStage,
      'GetLightingAsset': _lightingAsset,
      'WatchStatus': _watchStatus,
    };
  }

  final DemoPlayer _player;
  late final Map<String, Uint8List Function(Uint8List)> _unary;
  late final Map<String, Stream<Uint8List> Function(Uint8List)> _streams;
  final StreamController<TransportState> _stateController =
      StreamController<TransportState>.broadcast();
  // The streams that are open, each with what ends the work it does.
  final Map<StreamController<Uint8List>, void Function()> _open = {};
  TransportState _state = TransportState.ready;
  int _inFlight = 0;

  /// The demo player behind the channel, for tests.
  DemoPlayer get demoPlayer => _player;

  /// Decodes a request with [parse], has [handle] answer it, and encodes the
  /// answer.
  static Uint8List Function(Uint8List) _call<Req extends GeneratedMessage>(
    Req Function(List<int>) parse,
    GeneratedMessage Function(Req) handle,
  ) =>
      (request) => handle(parse(request)).writeToBuffer();

  @override
  Carrier get carrier => Carrier.demo;

  @override
  TransportState get currentState => _state;

  @override
  Stream<TransportState> get state => _stateController.stream;

  @override
  bool get hasStreams => true;

  @override
  int get callsInFlight => _inFlight;

  @override
  Stream<player.RemoteStatus> get statusStream => _player.statuses;

  @override
  Future<player.RemoteStatus> readStatus() async {
    if (_state != TransportState.ready) {
      throw const PlayerException.unavailable();
    }
    return _player.status();
  }

  @override
  Future<Uint8List> call(
    String method,
    Uint8List request, {
    Duration timeout = PlayerChannel.defaultTimeout,
  }) async {
    if (_state != TransportState.ready) {
      throw const PlayerException.unavailable();
    }
    _inFlight++;
    try {
      // Always a turn later, as an answer from a player is.
      await Future<void>.value();
      if (_state != TransportState.ready) {
        throw const PlayerException.unavailable();
      }
      final handler = _unary[method];
      if (handler == null) {
        throw PlayerException(
          GrpcCode.unimplemented,
          _streams.containsKey(method)
              ? 'method $method is a stream'
              : 'unknown method $method',
        );
      }
      try {
        return handler(request);
      } on InvalidProtocolBufferException catch (e) {
        throw PlayerException(GrpcCode.invalidArgument, 'bad request: $e');
      }
    } finally {
      _inFlight--;
    }
  }

  @override
  Stream<Uint8List> openStream(String method, Uint8List request) {
    final open = _streams[method];
    if (open == null) {
      return _failing(
        PlayerException(
          GrpcCode.unimplemented,
          _unary.containsKey(method)
              ? 'method $method is not a stream'
              : 'unknown method $method',
        ),
      );
    }
    if (_state != TransportState.ready) {
      return _failing(const PlayerException.unavailable());
    }
    return open(request);
  }

  // A stream that ends with [error] as soon as it is listened to.
  Stream<Uint8List> _failing(PlayerException error) {
    late final StreamController<Uint8List> controller;
    controller = StreamController<Uint8List>(
      onListen: () {
        controller.addError(error);
        unawaited(controller.close());
      },
      // A cancel that answers with a future of its own: the call is over.
      onCancel: () => Future<void>.value(),
    );
    return controller.stream;
  }

  /// A stream that [start] fills, once listened to, and that is cancelled
  /// by [stop] when its listener cancels or the channel is disposed of.
  Stream<Uint8List> _stream(
    void Function(StreamController<Uint8List>) start,
    void Function() stop,
  ) {
    late final StreamController<Uint8List> controller;
    controller = StreamController<Uint8List>(
      onListen: () {
        _open[controller] = stop;
        start(controller);
      },
      onCancel: () {
        _open.remove(controller)?.call();
        return Future<void>.value();
      },
    );
    return controller.stream;
  }

  /// The stage: the layout, then a frame whenever the look has changed.
  Stream<Uint8List> _watchStage(Uint8List request) {
    final asked = player.WatchStageRequest.fromBuffer(request).maxRate;
    final rate = asked == 0 ? demoStageRate : min(asked, demoStageRate);
    Timer? ticker;
    Uint8List? last;
    return _stream(
      (controller) {
        void frame() {
          if (controller.isPaused) {
            return;
          }
          final bytes = demoFrameUpdate(
            _player.look,
            _player.seconds,
          ).writeToBuffer();
          if (last != null && _same(last!, bytes)) {
            return;
          }
          last = bytes;
          controller.add(bytes);
        }

        controller.add(demoLayoutUpdate().writeToBuffer());
        frame();
        ticker = Timer.periodic(
          Duration(milliseconds: 1000 ~/ rate),
          (_) => frame(),
        );
      },
      () {
        ticker?.cancel();
        ticker = null;
      },
    );
  }

  static bool _same(Uint8List a, Uint8List b) {
    if (a.length != b.length) {
      return false;
    }
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) {
        return false;
      }
    }
    return true;
  }

  /// The player's status as it is pushed.
  Stream<Uint8List> _watchStatus(Uint8List request) {
    StreamSubscription<player.RemoteStatus>? subscription;
    return _stream(
      (controller) {
        controller.add(_player.status().writeToBuffer());
        subscription = _player.statuses.listen(
          (status) => controller.add(status.writeToBuffer()),
        );
      },
      () {
        unawaited(subscription?.cancel());
        subscription = null;
      },
    );
  }

  /// A lighting asset: the demo's rig has no 3D models and its venue no
  /// scenery, so no file is in the asset store, and the answer to every path
  /// is the one a real player gives for a file it does not have.
  Stream<Uint8List> _lightingAsset(Uint8List request) {
    player.GetLightingAssetRequest.fromBuffer(request);
    return _failing(
      const PlayerException(GrpcCode.notFound, 'Asset not found'),
    );
  }

  @override
  Future<void> dispose() async {
    if (_state == TransportState.disconnected) {
      return;
    }
    _state = TransportState.disconnected;
    if (!_stateController.isClosed) {
      _stateController.add(TransportState.disconnected);
    }
    final open = Map.of(_open);
    _open.clear();
    for (final MapEntry(key: controller, value: stop) in open.entries) {
      stop();
      if (!controller.isClosed) {
        controller.addError(const PlayerException.unavailable('closed'));
        unawaited(controller.close());
      }
    }
    _player.dispose();
    await _stateController.close();
  }
}
