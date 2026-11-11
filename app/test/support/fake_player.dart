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
import 'dart:typed_data';

import 'package:mtrack_remote/gen/ble/v1/ble.pb.dart' as ble;
import 'package:mtrack_remote/src/protocol/frame_codec.dart';
import 'package:mtrack_remote/src/protocol/transport.dart';
import 'package:protobuf/protobuf.dart';

/// What the fake player does with one request.
class FakeReply {
  /// Answers with [message] as the payload.
  FakeReply.ok(GeneratedMessage message, {this.delay = Duration.zero})
    : code = 0,
      text = '',
      payload = message.writeToBuffer(),
      drop = false;

  /// Answers with an empty payload.
  FakeReply.empty({this.delay = Duration.zero})
    : code = 0,
      text = '',
      payload = const [],
      drop = false;

  /// Answers with a gRPC error.
  FakeReply.error(this.code, this.text, {this.delay = Duration.zero})
    : payload = const [],
      drop = false;

  /// Never answers.
  FakeReply.drop()
    : code = 0,
      text = '',
      payload = const [],
      delay = Duration.zero,
      drop = true;

  final int code;
  final String text;
  final List<int> payload;

  /// How long after the request completes before the response is sent.
  final Duration delay;

  /// True to send nothing.
  final bool drop;
}

/// An in-memory [PlayerTransport] that behaves like the player.
///
/// It reassembles what the client writes, answers allowlisted methods from
/// [handlers] (an empty success when a method has none), refuses every other
/// method with `PERMISSION_DENIED` as the real player does, and sends
/// responses and status frames back as chunks cut to [maxChunk].
///
/// It opens the streams in [streamMethods] as the real player does: at most
/// [maxStreams] at once, an id that is open refused on `response`, a cancel
/// answered with `CANCELLED` on `stream`. What a stream says is the test's to
/// send, with [streamMessage] and [endStream]. With [hasStreams] false it is
/// a player from before streams: no `stream` characteristic, and a stream
/// method refused like any other it does not know.
class FakePlayer implements PlayerTransport {
  /// A fake that starts connected, cutting to [maxChunk].
  FakePlayer({this.maxChunk = 185});

  /// The methods a phone may call.
  static const Set<String> allowlist = {
    'Play',
    'PlayFrom',
    'PlaySongFrom',
    'Stop',
    'Next',
    'Previous',
    'NavigateToSong',
    'Seek',
    'SeekToSection',
    'SwitchToPlaylist',
    'Status',
    'LoopSection',
    'StopSectionLoop',
    'SectionAck',
    'StopSamples',
    'CancelAutoPlay',
    'ListPlaylists',
    'GetPlaylist',
    'GetTrackGains',
    'SetTrackGain',
    'SetTrackMute',
    'SetAmbientMode',
    'SavePlaylist',
    'DeletePlaylist',
    'SetLock',
  };

  /// The stream methods a phone may open.
  static const Set<String> streamMethods = {'WatchStage'};

  /// How many streams a phone may have open.
  static const int maxStreams = 2;

  /// False for a player from before streams.
  @override
  bool hasStreams = true;

  /// Called when a stream has been opened, with its request.
  void Function(ble.Request request)? onStream;

  /// The ids of the streams that are open, in the order they were opened.
  final List<int> openStreams = [];

  /// The ids cancelled, in order, whether or not a stream was open under
  /// them.
  final List<int> cancels = [];

  /// When true a cancel is taken but its last response is not sent, until
  /// the test sends it with [endStream].
  bool holdCancels = false;

  /// How a method is answered, by bare method name.
  final Map<String, FakeReply Function(ble.Request)> handlers = {};

  /// Every request the fake has fully received, in order.
  final List<ble.Request> received = [];

  /// Every chunk written, in order.
  final List<Uint8List> writes = [];

  /// How many times a start chunk arrived while a frame was in progress: the
  /// mark of two requests interleaving on the wire.
  int interleavedStarts = 0;

  /// When true, every [write] fails though the link says it is ready: what
  /// a phone sees writing to a characteristic the player no longer has.
  bool refuseWrites = false;

  /// How long each [write] takes to be acknowledged.
  Duration writeDelay = Duration.zero;

  /// The bytes the next [readStatus] returns.
  Uint8List statusBytes = Uint8List(0);

  /// When set, each status read and each request is noted in it, as
  /// `readStatus` and `call:Method`, so a test can check their order against
  /// other events.
  List<String>? log;

  @override
  int maxChunk;

  TransportState _state = TransportState.ready;
  final StreamController<TransportState> _stateController =
      StreamController<TransportState>.broadcast();
  final StreamController<Uint8List> _responses =
      StreamController<Uint8List>.broadcast();
  final StreamController<Uint8List> _statuses =
      StreamController<Uint8List>.broadcast();
  final StreamController<Uint8List> _streams =
      StreamController<Uint8List>.broadcast();
  final Reassembler _incoming = Reassembler();
  bool _midFrame = false;

  /// True while something listens for response notifications.
  bool get hasResponseListener => _responses.hasListener;

  /// True while something listens for status notifications.
  bool get hasStatusListener => _statuses.hasListener;

  /// True while something listens for stream notifications.
  bool get hasStreamListener => _streams.hasListener;

  @override
  TransportState get currentState => _state;

  @override
  Stream<TransportState> get state => _stateController.stream;

  @override
  Stream<Uint8List> get responseChunks => _responses.stream;

  @override
  Stream<Uint8List> get statusChunks => _statuses.stream;

  @override
  Stream<Uint8List> get streamChunks => _streams.stream;

  /// Moves the link to [next]. Leaving ready drops the frame being received
  /// and every open stream, with nothing said.
  void setState(TransportState next) {
    _state = next;
    if (next != TransportState.ready) {
      _incoming.reset();
      _midFrame = false;
      openStreams.clear();
    }
    _stateController.add(next);
  }

  /// Drops the link.
  void disconnect() => setState(TransportState.disconnected);

  @override
  Future<void> write(Uint8List chunk) async {
    if (_state != TransportState.ready) {
      throw StateError('write while not ready');
    }
    if (refuseWrites) {
      throw StateError('write refused');
    }
    if (writeDelay > Duration.zero) {
      await Future<void>.delayed(writeDelay);
      if (_state != TransportState.ready) {
        throw StateError('link lost during write');
      }
    }
    writes.add(chunk);
    if (chunk.isNotEmpty && chunk[0] == 0x01 && _midFrame) {
      interleavedStarts++;
    }
    final frame = _incoming.push(chunk);
    _midFrame = frame == null;
    if (frame != null) {
      _handle(ble.Request.fromBuffer(frame));
    }
  }

  void _handle(ble.Request request) {
    received.add(request);
    if (hasStreams && request.cancel) {
      log?.add('cancel');
      cancels.add(request.id);
      // Nothing answers a cancel itself, and one for no open stream is
      // ignored.
      if (openStreams.contains(request.id) && !holdCancels) {
        endStream(request.id, code: 1, text: 'cancelled');
      }
      return;
    }
    log?.add('call:${request.method}');
    if (hasStreams && streamMethods.contains(request.method)) {
      if (openStreams.contains(request.id)) {
        sendResponse(
          ble.Response(
            id: request.id,
            code: 6,
            message: 'stream ${request.id} is already open',
          ),
        );
      } else if (openStreams.length >= maxStreams) {
        sendStream(
          ble.Response(
            id: request.id,
            code: 8,
            message: '$maxStreams streams are open already',
          ),
        );
      } else {
        openStreams.add(request.id);
        onStream?.call(request);
      }
      return;
    }
    final FakeReply reply;
    if (!allowlist.contains(request.method)) {
      reply = FakeReply.error(7, 'method ${request.method} is not allowed');
    } else {
      final handler = handlers[request.method];
      reply = handler == null ? FakeReply.empty() : handler(request);
    }
    if (reply.drop) {
      return;
    }
    final response = ble.Response(
      id: request.id,
      code: reply.code,
      message: reply.text,
      payload: reply.payload,
    );
    if (reply.delay == Duration.zero) {
      sendResponse(response);
    } else {
      Timer(reply.delay, () => sendResponse(response));
    }
  }

  /// Sends [response] on the response characteristic, chunked. Use it for a
  /// response the fake would not send itself: another phone's, or a late one.
  void sendResponse(ble.Response response, {int? chunkSize}) {
    sendResponseChunks(chunks(response.writeToBuffer(), chunkSize ?? maxChunk));
  }

  /// Sends raw chunks on the response characteristic.
  void sendResponseChunks(Iterable<Uint8List> pieces) {
    pieces.forEach(_responses.add);
  }

  /// Sends [response] on the stream characteristic, chunked.
  void sendStream(ble.Response response, {int? chunkSize}) {
    sendStreamChunks(chunks(response.writeToBuffer(), chunkSize ?? maxChunk));
  }

  /// Sends raw chunks on the stream characteristic.
  void sendStreamChunks(Iterable<Uint8List> pieces) {
    pieces.forEach(_streams.add);
  }

  /// Sends [payload] as a message of stream [id].
  void streamMessage(int id, List<int> payload, {int? chunkSize}) {
    sendStream(
      ble.Response(id: id, more: true, payload: payload),
      chunkSize: chunkSize,
    );
  }

  /// Ends stream [id] with its last response.
  void endStream(int id, {int code = 0, String text = ''}) {
    openStreams.remove(id);
    sendStream(ble.Response(id: id, code: code, message: text));
  }

  /// Sends [status] on the status characteristic, chunked.
  void emitStatus(ble.Status status, {int? chunkSize}) {
    emitStatusFrame(status.writeToBuffer(), chunkSize: chunkSize);
  }

  /// Sends [frame] on the status characteristic, chunked.
  void emitStatusFrame(Uint8List frame, {int? chunkSize}) {
    emitStatusChunks(chunks(frame, chunkSize ?? maxChunk));
  }

  /// Sends raw chunks on the status characteristic.
  void emitStatusChunks(Iterable<Uint8List> pieces) {
    pieces.forEach(_statuses.add);
  }

  @override
  Future<Uint8List> readStatus() async {
    log?.add('readStatus');
    return statusBytes;
  }

  @override
  Future<void> close() async {
    setState(TransportState.disconnected);
  }
}
