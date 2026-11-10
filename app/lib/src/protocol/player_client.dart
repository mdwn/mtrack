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
import 'package:protobuf/well_known_types/google/protobuf/duration.pb.dart'
    as pb;

import '../../gen/ble/v1/ble.pb.dart' as ble;
import '../../gen/player/v1/player.pb.dart' as player;
import 'frame_codec.dart';
import 'player_exception.dart';
import 'transport.dart';

/// Calls the player's `PlayerService` over a [PlayerTransport] and follows its
/// status.
///
/// The client owns the protocol: it numbers requests, frames and chunks them,
/// matches responses to calls by id, and times calls out. The transport owns
/// the radio.
class PlayerClient {
  /// A client over [transport].
  ///
  /// [random] supplies the first request id; it defaults to a cryptographically
  /// secure source, because every phone's responses reach every phone and a
  /// guessable id would make another phone's answers look like ours.
  PlayerClient(this._transport, {Random? random})
    : _nextId = (random ?? Random.secure()).nextInt(1 << 32) {
    _subscriptions
      ..add(_transport.state.listen(_onState))
      ..add(_transport.responseChunks.listen(_onResponseChunk))
      ..add(_transport.statusChunks.listen(_onStatusChunk));
  }

  /// How long a call waits for its answer unless the caller says otherwise.
  static const Duration defaultTimeout = Duration(seconds: 5);

  final PlayerTransport _transport;
  final List<StreamSubscription<Object?>> _subscriptions = [];
  final Map<int, _Pending> _pending = {};
  final Reassembler _responses = Reassembler();
  final Reassembler _statuses = Reassembler();
  final StreamController<ble.Status> _statusController =
      StreamController<ble.Status>.broadcast();

  int _nextId;

  /// The tail of the write queue. A call writes only once the one before it
  /// has written every chunk, so two requests never interleave on the wire.
  Future<void> _writeQueue = Future<void>.value();

  bool _disposed = false;

  /// The player's status as it is pushed: decoded `ble.v1.Status` messages.
  ///
  /// A frame that does not decode is dropped. Continuation chunks that arrive
  /// before the first start, as they can when subscribing in the middle of a
  /// frame, are ignored.
  Stream<ble.Status> get statusStream => _statusController.stream;

  /// Reads the `status` characteristic once and decodes it.
  Future<ble.Status> readStatus() async {
    final bytes = await _transport.readStatus();
    return ble.Status.fromBuffer(bytes);
  }

  /// Calls [method] (a bare `PlayerService` method name such as `Play`) with
  /// [request] and returns what [decode] makes of the response payload.
  ///
  /// Throws [PlayerException] when the player answers with a non-zero code,
  /// when no answer comes within [timeout], or when the link is down or goes
  /// down first.
  Future<R> call<R>(
    String method,
    GeneratedMessage request,
    R Function(List<int>) decode, {
    Duration timeout = defaultTimeout,
  }) async {
    if (_disposed || _transport.currentState != TransportState.ready) {
      throw const PlayerException.unavailable();
    }
    final id = _nextId;
    _nextId = (_nextId + 1) & 0xffffffff;

    final frame = ble.Request(
      id: id,
      method: method,
      payload: request.writeToBuffer(),
    ).writeToBuffer();
    final List<Uint8List> pieces;
    try {
      pieces = chunks(frame, _transport.maxChunk);
    } on FrameError catch (e) {
      throw PlayerException(GrpcCode.invalidArgument, e.message);
    }

    final completer = Completer<ble.Response>();
    final pending = _Pending(completer);
    _pending[id] = pending;
    pending.timer = Timer(timeout, () {
      if (_pending.remove(id) != null) {
        completer.completeError(PlayerException.timeout(method));
      }
    });

    unawaited(_send(id, pieces));

    final ble.Response response;
    try {
      response = await completer.future;
    } finally {
      pending.timer?.cancel();
    }
    if (response.code != GrpcCode.ok) {
      throw PlayerException(response.code, response.message);
    }
    return decode(response.payload);
  }

  /// Writes a call's chunks in order once the queue reaches it. A failed write
  /// fails the call; a call that is no longer pending (timed out, or the link
  /// dropped) stops writing.
  Future<void> _send(int id, List<Uint8List> pieces) async {
    final previous = _writeQueue;
    final turn = Completer<void>();
    _writeQueue = turn.future;
    try {
      await previous;
      for (final piece in pieces) {
        if (!_pending.containsKey(id)) {
          return;
        }
        await _transport.write(piece);
      }
    } on Object catch (e) {
      final pending = _pending.remove(id);
      if (pending != null) {
        pending.completer.completeError(
          e is PlayerException ? e : PlayerException.unavailable('$e'),
        );
      }
    } finally {
      turn.complete();
    }
  }

  void _onState(TransportState state) {
    _responses.reset();
    _statuses.reset();
    if (state != TransportState.ready) {
      _failAll(const PlayerException.unavailable('connection lost'));
    }
  }

  void _failAll(PlayerException error) {
    final waiting = _pending.values.toList();
    _pending.clear();
    for (final pending in waiting) {
      pending.timer?.cancel();
      pending.completer.completeError(error);
    }
  }

  void _onResponseChunk(Uint8List chunk) {
    final Uint8List? frame;
    try {
      frame = _responses.push(chunk);
    } on FrameError {
      return;
    }
    if (frame == null) {
      return;
    }
    final ble.Response response;
    try {
      response = ble.Response.fromBuffer(frame);
    } on InvalidProtocolBufferException {
      return;
    }
    // Another phone's answer, or one for a call that has timed out.
    final pending = _pending.remove(response.id);
    if (pending == null) {
      return;
    }
    pending.timer?.cancel();
    pending.completer.complete(response);
  }

  void _onStatusChunk(Uint8List chunk) {
    final Uint8List? frame;
    try {
      frame = _statuses.push(chunk);
    } on FrameError {
      return;
    }
    if (frame == null) {
      return;
    }
    try {
      _statusController.add(ble.Status.fromBuffer(frame));
    } on InvalidProtocolBufferException {
      return;
    }
  }

  /// Stops listening to the transport and fails every call still waiting. The
  /// transport itself is left open; close it separately.
  Future<void> dispose() async {
    if (_disposed) {
      return;
    }
    _disposed = true;
    // A subscription stops delivering the moment it is cancelled; its future
    // only reports that the stream's own clean-up is done, which nothing here
    // waits for.
    for (final subscription in _subscriptions) {
      unawaited(subscription.cancel());
    }
    _subscriptions.clear();
    _failAll(const PlayerException.unavailable('closed'));
    await _statusController.close();
  }

  // Typed calls, one per method the player allows over Bluetooth.

  /// Plays the current song.
  Future<player.PlayResponse> play() =>
      call('Play', player.PlayRequest(), player.PlayResponse.fromBuffer);

  /// Plays the current song from [start].
  Future<player.PlayResponse> playFrom(Duration start) => call(
    'PlayFrom',
    player.PlayFromRequest(startTime: pb.Duration.fromDart(start)),
    player.PlayResponse.fromBuffer,
  );

  /// Plays the named song from [start].
  Future<player.PlayResponse> playSongFrom(String song, Duration start) => call(
    'PlaySongFrom',
    player.PlaySongFromRequest(
      songName: song,
      startTime: pb.Duration.fromDart(start),
    ),
    player.PlayResponse.fromBuffer,
  );

  /// Stops the song that is playing.
  Future<player.StopResponse> stop() =>
      call('Stop', player.StopRequest(), player.StopResponse.fromBuffer);

  /// Moves the playlist to the next song.
  Future<player.NextResponse> next() =>
      call('Next', player.NextRequest(), player.NextResponse.fromBuffer);

  /// Moves the playlist to the previous song.
  Future<player.PreviousResponse> previous() => call(
    'Previous',
    player.PreviousRequest(),
    player.PreviousResponse.fromBuffer,
  );

  /// Makes the playlist entry at [position] current without playing it.
  /// [songName] is the song the caller believes is there: the player refuses
  /// with FAILED_PRECONDITION when the entry holds another.
  Future<player.NavigateToSongResponse> navigateToSong(
    int position,
    String songName,
  ) => call(
    'NavigateToSong',
    player.NavigateToSongRequest(position: position, songName: songName),
    player.NavigateToSongResponse.fromBuffer,
  );

  /// Jumps to [position] in the current song.
  Future<player.SeekResponse> seek(Duration position) => call(
    'Seek',
    player.SeekRequest(position: pb.Duration.fromDart(position)),
    player.SeekResponse.fromBuffer,
  );

  /// Jumps to the start of the named section of the current song.
  Future<player.SeekResponse> seekToSection(String section) => call(
    'SeekToSection',
    player.SeekToSectionRequest(sectionName: section),
    player.SeekResponse.fromBuffer,
  );

  /// Switches the player to the named playlist.
  Future<player.SwitchToPlaylistResponse> switchToPlaylist(String name) => call(
    'SwitchToPlaylist',
    player.SwitchToPlaylistRequest(playlistName: name),
    player.SwitchToPlaylistResponse.fromBuffer,
  );

  /// The player's full status, with the song's beat grid and sections.
  Future<player.StatusResponse> status() =>
      call('Status', player.StatusRequest(), player.StatusResponse.fromBuffer);

  /// Loops the named section of the current song.
  Future<player.LoopSectionResponse> loopSection(String section) => call(
    'LoopSection',
    player.LoopSectionRequest(sectionName: section),
    player.LoopSectionResponse.fromBuffer,
  );

  /// Ends the section loop at the end of the current pass.
  Future<player.StopSectionLoopResponse> stopSectionLoop() => call(
    'StopSectionLoop',
    player.StopSectionLoopRequest(),
    player.StopSectionLoopResponse.fromBuffer,
  );

  /// Acknowledges a section that is waiting for it.
  Future<player.SectionAckResponse> sectionAck() => call(
    'SectionAck',
    player.SectionAckRequest(),
    player.SectionAckResponse.fromBuffer,
  );

  /// Stops all triggered samples that are playing.
  Future<player.StopSamplesResponse> stopSamples() => call(
    'StopSamples',
    player.StopSamplesRequest(),
    player.StopSamplesResponse.fromBuffer,
  );

  /// Cancels the song that is waiting to start by itself.
  Future<player.CancelAutoPlayResponse> cancelAutoPlay() => call(
    'CancelAutoPlay',
    player.CancelAutoPlayRequest(),
    player.CancelAutoPlayResponse.fromBuffer,
  );

  /// The names of the playlists and which is active.
  Future<player.ListPlaylistsResponse> listPlaylists() => call(
    'ListPlaylists',
    player.ListPlaylistsRequest(),
    player.ListPlaylistsResponse.fromBuffer,
  );

  /// The entries of the named playlist, or of the active one when [name] is
  /// null.
  Future<player.GetPlaylistResponse> getPlaylist([String? name]) => call(
    'GetPlaylist',
    player.GetPlaylistRequest(name: name),
    player.GetPlaylistResponse.fromBuffer,
  );
}

/// A call waiting for its response.
class _Pending {
  _Pending(this.completer);

  final Completer<ble.Response> completer;
  Timer? timer;
}
