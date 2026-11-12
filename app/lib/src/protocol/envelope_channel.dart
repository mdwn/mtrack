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
import 'dart:collection';
import 'dart:math';
import 'dart:typed_data';

import 'package:protobuf/protobuf.dart';

import '../../gen/ble/v1/ble.pb.dart' as ble;
import '../../gen/player/v1/player.pb.dart' as player;
import 'frame_codec.dart';
import 'player_channel.dart';
import 'player_exception.dart';
import 'transport.dart';

/// Carries the player's `PlayerService` calls over a [PlayerTransport], inside
/// the Bluetooth envelope: the one way to reach a player by radio.
///
/// The channel owns the protocol: it numbers requests, frames and chunks them,
/// matches responses to calls by id, and times calls out. The transport owns
/// the radio.
///
/// A server stream ([openStream]) is a call with many answers, which arrive
/// on the `stream` characteristic and are reassembled apart from the unary
/// answers on `response`: chunks of the two interleave on the air.
class EnvelopeChannel implements PlayerChannel {
  /// A channel over [transport].
  ///
  /// [random] supplies the first request id; it defaults to a cryptographically
  /// secure source, because every phone's responses reach every phone and a
  /// guessable id would make another phone's answers look like ours.
  EnvelopeChannel(this._transport, {Random? random})
    : _nextId = (random ?? Random.secure()).nextInt(1 << 32) {
    _subscriptions
      ..add(_transport.state.listen(_onState))
      ..add(_transport.responseChunks.listen(_onResponseChunk))
      ..add(_transport.statusChunks.listen(_onStatusChunk))
      ..add(_transport.streamChunks.listen(_onStreamChunk));
  }

  /// How long a cancelled stream's id is kept for the player's last response
  /// before it is given up. The player finishes a message it has begun before
  /// it answers a cancel, and the largest message takes about four seconds at
  /// the pace the player sends streams.
  static const Duration cancelTimeout = Duration(seconds: 10);

  /// The most a stream holds for a listener that has paused, in bytes of
  /// messages (with a few bytes counted for each, so that empty ones add up
  /// too). Past it the stream ends with `RESOURCE_EXHAUSTED`: see
  /// [openStream].
  static const int maxHeldStreamBytes = 256 * 1024;

  final PlayerTransport _transport;
  final List<StreamSubscription<Object?>> _subscriptions = [];
  final Map<int, _Pending> _pending = {};
  final Reassembler _responses = Reassembler();
  final Reassembler _statuses = Reassembler();
  final Map<int, _OpenStream> _streams = {};
  final Reassembler _streamFrames = Reassembler();
  final StreamController<player.RemoteStatus> _statusController =
      StreamController<player.RemoteStatus>.broadcast();

  int _nextId;

  /// The tail of the write queue. A call writes only once the one before it
  /// has written every chunk, so two requests never interleave on the wire.
  Future<void> _writeQueue = Future<void>.value();

  bool _disposed = false;

  @override
  Carrier get carrier => Carrier.bluetooth;

  @override
  TransportState get currentState => _transport.currentState;

  @override
  Stream<TransportState> get state => _transport.state;

  @override
  bool get hasStreams => _transport.hasStreams;

  @override
  int get callsInFlight => _pending.length;

  /// The player's status as it is pushed: decoded `RemoteStatus` messages.
  ///
  /// A frame that does not decode is dropped. Continuation chunks that arrive
  /// before the first start, as they can when subscribing in the middle of a
  /// frame, are ignored.
  @override
  Stream<player.RemoteStatus> get statusStream => _statusController.stream;

  @override
  Future<player.RemoteStatus> readStatus() async {
    final bytes = await _transport.readStatus();
    return player.RemoteStatus.fromBuffer(bytes);
  }

  /// Calls [method] (a bare `PlayerService` method name such as `Play`) with
  /// the serialized [request] and returns the response payload.
  ///
  /// Throws [PlayerException] when the player answers with a non-zero code,
  /// when no answer comes within [timeout], or when the link is down or goes
  /// down first.
  @override
  Future<Uint8List> call(
    String method,
    Uint8List request, {
    Duration timeout = PlayerChannel.defaultTimeout,
  }) async {
    if (_disposed || _transport.currentState != TransportState.ready) {
      throw const PlayerException.unavailable();
    }
    final id = _takeId();

    final frame = ble.Request(
      id: id,
      method: method,
      payload: request,
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

    unawaited(
      _send(
        pieces,
        wanted: () => _pending.containsKey(id),
        failed: (error) => _pending.remove(id)?.completer.completeError(error),
      ),
    );

    final ble.Response response;
    try {
      response = await completer.future;
    } finally {
      pending.timer?.cancel();
    }
    if (response.code != GrpcCode.ok) {
      throw PlayerException(response.code, response.message);
    }
    final payload = response.payload;
    return payload is Uint8List ? payload : Uint8List.fromList(payload);
  }

  /// For tests only: sets the id the next request takes, so one can bring
  /// the counter round to an id in use without sending four thousand million
  /// requests. (Not annotated as such: this layer imports nothing that has
  /// the annotation.)
  set debugNextId(int id) => _nextId = id & 0xffffffff;

  /// The next request id, passing over any a call or a stream is open under:
  /// a stream keeps its id until the player's last response for it.
  int _takeId() {
    while (_pending.containsKey(_nextId) || _streams.containsKey(_nextId)) {
      _nextId = (_nextId + 1) & 0xffffffff;
    }
    final id = _nextId;
    _nextId = (_nextId + 1) & 0xffffffff;
    return id;
  }

  /// Runs [body] once everything queued before it has written, so two
  /// requests never interleave on the wire.
  Future<void> _inTurn(Future<void> Function() body) async {
    final previous = _writeQueue;
    final turn = Completer<void>();
    _writeQueue = turn.future;
    try {
      await previous;
      await body();
    } finally {
      turn.complete();
    }
  }

  /// Writes a request's chunks in order once the queue reaches it. [wanted]
  /// is asked before each chunk: a call that is no longer pending (timed out,
  /// or the link dropped) stops writing. A failed write is reported to
  /// [failed].
  Future<void> _send(
    List<Uint8List> pieces, {
    required bool Function() wanted,
    required void Function(PlayerException error) failed,
  }) => _inTurn(() async {
    try {
      for (final piece in pieces) {
        if (!wanted()) {
          return;
        }
        await _transport.write(piece);
      }
    } on Object catch (e) {
      failed(e is PlayerException ? e : PlayerException.unsent('$e'));
    }
  });

  // Streams.

  /// Opens the server stream [method] (a bare `PlayerService` method name,
  /// such as `WatchStage`) with the serialized request [payload]. Messages
  /// arrive as their serialized payloads, in order; nothing here decodes
  /// them.
  ///
  /// The request is sent when the stream is listened to, and the stream is
  /// single-subscription. It closes normally when the player ends it with
  /// code 0, and with a [PlayerException] first for any other code, for a
  /// refusal (an old player's `PERMISSION_DENIED`, `ALREADY_EXISTS`), for a
  /// request that could not be written (`unsent`) and for a lost link
  /// (`UNAVAILABLE`, as a unary call gets). Against a player with no `stream`
  /// characteristic nothing is sent and the error is
  /// [PlayerException.noStreams].
  ///
  /// Cancelling the subscription sends the cancel. The future `cancel()`
  /// returns completes when the id is free again: when the player's last
  /// response arrives, the link goes, or [cancelTimeout] passes without
  /// either. Until then the id is not used for anything else, and the stream
  /// still counts toward the two a phone may have open.
  ///
  /// A stream has no timeout: a player with nothing to say is silent. A link
  /// that died without the phone noticing is the caller's to find, by calling
  /// something.
  ///
  /// Messages are delivered synchronously as their last chunk arrives, and
  /// nothing is queued for a listener that is listening. A listener that
  /// pauses has what arrives held for it, up to [maxHeldStreamBytes]; past
  /// that what was held is dropped, the stream is cancelled at the player
  /// and the listener gets `RESOURCE_EXHAUSTED` (and the end, once the
  /// player has let go of the stream), because nothing here can tell which
  /// messages could be dropped. So memory is bounded whatever the listener
  /// does, and a listener that wants only the newest message does the
  /// coalescing itself.
  @override
  Stream<Uint8List> openStream(String method, Uint8List payload) {
    _OpenStream? open;
    late final StreamController<Uint8List> controller;
    controller = StreamController<Uint8List>(
      sync: true,
      onListen: () => open = _startStream(method, payload, controller),
      onResume: () => open?.drain(),
      onCancel: () {
        final stream = open;
        return stream == null ? null : _cancelStream(stream);
      },
    );
    return controller.stream;
  }

  /// Asks for the stream. Returns null, and ends [controller] with the
  /// reason, when it cannot be asked for.
  _OpenStream? _startStream(
    String method,
    Uint8List payload,
    StreamController<Uint8List> controller,
  ) {
    PlayerException? refusal;
    var id = 0;
    var pieces = const <Uint8List>[];
    if (_disposed || _transport.currentState != TransportState.ready) {
      refusal = const PlayerException.unavailable();
    } else if (!_transport.hasStreams) {
      refusal = const PlayerException.noStreams();
    } else {
      id = _takeId();
      final frame = ble.Request(
        id: id,
        method: method,
        payload: payload,
      ).writeToBuffer();
      try {
        pieces = chunks(frame, _transport.maxChunk);
      } on FrameError catch (e) {
        refusal = PlayerException(GrpcCode.invalidArgument, e.message);
      }
    }
    if (refusal != null) {
      final error = refusal;
      // Not from inside the listen call: the listener is not set up yet.
      scheduleMicrotask(() {
        controller.addError(error);
        unawaited(controller.close());
      });
      return null;
    }
    final open = _OpenStream(id, controller);
    _streams[id] = open;
    unawaited(
      _send(
        pieces,
        wanted: () {
          // Over already (the link dropped), or cancelled before any of it
          // was written, which leaves nothing to cancel. Once some of it is
          // written all of it is, so the cancel that follows finds a stream.
          if (open.ended || (open.cancelled && !open.sent)) {
            return false;
          }
          open.sent = true;
          return true;
        },
        failed: (error) => _endStream(open, error: error),
      ),
    );
    return open;
  }

  /// The listener is gone: tells the player, and keeps the id until the
  /// player's last response. The future completes when the id is free.
  Future<void> _cancelStream(_OpenStream open) {
    open.listening = false;
    if (!open.ended && !open.cancelled) {
      open.cancelled = true;
      open.drop();
      unawaited(_sendCancel(open));
    }
    return open.closed.future;
  }

  Future<void> _sendCancel(_OpenStream open) => _inTurn(() async {
    if (open.ended) {
      return;
    }
    if (!open.sent) {
      // The request never left, so the player has nothing open.
      _endStream(open);
      return;
    }
    try {
      final frame = ble.Request(id: open.id, cancel: true).writeToBuffer();
      for (final piece in chunks(frame, _transport.maxChunk)) {
        await _transport.write(piece);
      }
    } on Object {
      // The link took nothing: whatever the player has open goes with the
      // link, and the next call finds it gone.
      _endStream(open);
      return;
    }
    if (!open.ended) {
      open.cancelTimer = Timer(cancelTimeout, () => _endStream(open));
    }
  });

  /// Stream [open] is over: its id is free. A listener that is still there
  /// gets what was held for it, then [error] if there is one, then the end;
  /// one that was given up on has had its error, and gets the end.
  void _endStream(_OpenStream open, {PlayerException? error}) {
    if (open.ended) {
      return;
    }
    open.ended = true;
    if (identical(_streams[open.id], open)) {
      _streams.remove(open.id);
    }
    open.cancelTimer?.cancel();
    if (open.listening) {
      if (!open.cancelled) {
        open.flush();
        if (error != null) {
          open.controller.addError(error);
        }
      }
      unawaited(open.controller.close());
    }
    open.closed.complete();
  }

  /// The listener of [open] paused and too much has arrived since: the
  /// stream is given up, and the player told to stop sending it. The
  /// listener's stream ends when the player has let go, as a cancel's does.
  void _overflow(_OpenStream open) {
    open.drop();
    open.controller.addError(
      const PlayerException(
        GrpcCode.resourceExhausted,
        'the listener fell too far behind the stream',
      ),
    );
    open.cancelled = true;
    unawaited(_sendCancel(open));
  }

  void _endAllStreams(PlayerException error) {
    for (final open in _streams.values.toList()) {
      _endStream(open, error: error);
    }
  }

  void _onStreamChunk(Uint8List chunk) {
    final Uint8List? frame;
    try {
      frame = _streamFrames.push(chunk);
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
    // Another phone's stream, or one of ours that is over.
    final open = _streams[response.id];
    if (open == null) {
      return;
    }
    if (!response.more) {
      _endStream(
        open,
        error: response.code == GrpcCode.ok
            ? null
            : PlayerException(response.code, response.message),
      );
      return;
    }
    if (open.cancelled) {
      // Sent before the player heard the cancel.
      return;
    }
    final payload = response.payload;
    open.deliver(payload is Uint8List ? payload : Uint8List.fromList(payload));
    if (open.heldBytes > maxHeldStreamBytes) {
      _overflow(open);
    }
  }

  void _onState(TransportState state) {
    _responses.reset();
    _statuses.reset();
    _streamFrames.reset();
    if (state != TransportState.ready) {
      _failAll(const PlayerException.unavailable('connection lost'));
      _endAllStreams(const PlayerException.unavailable('connection lost'));
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
      // Or the refusal of a stream, which is the one thing about a stream
      // that comes here: an old player's PERMISSION_DENIED, or
      // ALREADY_EXISTS.
      final open = _streams[response.id];
      if (open != null && response.code != GrpcCode.ok) {
        _endStream(
          open,
          error: PlayerException(response.code, response.message),
        );
      }
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
      _statusController.add(player.RemoteStatus.fromBuffer(frame));
    } on InvalidProtocolBufferException {
      return;
    }
  }

  /// Stops listening to the transport, fails every call still waiting and
  /// ends every stream. The transport itself is left open; close it
  /// separately.
  @override
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
    _endAllStreams(const PlayerException.unavailable('closed'));
    await _statusController.close();
  }
}

/// A stream the player has open for us, or is being asked for, or has been
/// told to stop: from its request until its id is free again.
class _OpenStream {
  _OpenStream(this.id, this.controller);

  /// What a held message counts for beyond its bytes.
  static const int _overhead = 16;

  final int id;
  final StreamController<Uint8List> controller;

  /// Completes when the id is free.
  final Completer<void> closed = Completer<void>();

  /// Messages that arrived while the listener was paused, oldest first.
  final Queue<Uint8List> _held = ListQueue<Uint8List>();

  /// What [_held] counts for.
  int heldBytes = 0;

  /// True once any of the request has been written.
  bool sent = false;

  /// True once the listener is gone, or has been given up on: no more
  /// messages go to it, and the player has been or will be told.
  bool cancelled = false;

  /// True until the listener cancels, or its stream has ended.
  bool listening = true;

  /// True once the id is free.
  bool ended = false;

  /// Runs out when the player does not answer a cancel.
  Timer? cancelTimer;

  /// Hands [message] to the listener, or holds it while the listener is
  /// paused, behind what is held already.
  void deliver(Uint8List message) {
    if (_held.isEmpty && !controller.isPaused) {
      controller.add(message);
      return;
    }
    _held.add(message);
    heldBytes += message.length + _overhead;
  }

  /// The listener resumed: hands over what was held, for as long as it
  /// stays resumed.
  void drain() {
    while (_held.isNotEmpty && !controller.isPaused) {
      final message = _held.removeFirst();
      heldBytes -= message.length + _overhead;
      controller.add(message);
    }
  }

  /// Hands over everything held, paused or not, ahead of the stream's end.
  void flush() {
    _held.forEach(controller.add);
    drop();
  }

  /// Forgets what was held.
  void drop() {
    _held.clear();
    heldBytes = 0;
  }
}

/// A call waiting for its response.
class _Pending {
  _Pending(this.completer);

  final Completer<ble.Response> completer;
  Timer? timer;
}
