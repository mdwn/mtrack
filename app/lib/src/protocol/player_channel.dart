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
import 'dart:typed_data';

import '../../gen/player/v1/player.pb.dart' as player;
import 'transport.dart';

/// Which radio a channel runs on.
enum Carrier {
  /// Bluetooth Low Energy, inside the envelope (see `envelope_channel.dart`).
  bluetooth,

  /// The player's gRPC port, over the network the phone and the player share.
  wifi,
}

/// The seam between the app and the player: "call this method with these
/// request bytes and get response bytes", and "open this server stream".
///
/// Everything above it speaks `PlayerService` in serialized messages and
/// neither knows nor cares which [Carrier] is under it. Bluetooth implements
/// it with the envelope (`EnvelopeChannel`), Wi-Fi with real gRPC calls
/// (`GrpcChannel`). Both fail with a `PlayerException`, carrying the same gRPC
/// status codes, so a screen says the same thing whichever was used.
abstract class PlayerChannel {
  /// How long a call waits for its answer unless the caller says otherwise.
  static const Duration defaultTimeout = Duration(seconds: 5);

  /// Which radio this runs on.
  Carrier get carrier;

  /// The state of the link now.
  TransportState get currentState;

  /// Every change of the link's state, in order. A broadcast stream. Leaving
  /// [TransportState.ready] is final for a channel: the app makes a new one.
  Stream<TransportState> get state;

  /// Whether the player can carry a server stream at all: false for a player
  /// from before streams (Bluetooth only). Valid while the link is up.
  bool get hasStreams;

  /// How many unary calls are waiting for their answer now.
  int get callsInFlight;

  /// Calls [method] (a bare `PlayerService` method name such as `Play`) with
  /// the serialized [request] and returns the serialized response.
  ///
  /// Throws `PlayerException` when the player answers with a non-zero code,
  /// when no answer comes within [timeout], or when the link is down or goes
  /// down first.
  Future<Uint8List> call(
    String method,
    Uint8List request, {
    Duration timeout = defaultTimeout,
  });

  /// Opens the server stream [method] with the serialized [request]. Messages
  /// arrive as serialized payloads, in order. The request is sent when the
  /// stream is listened to, and the stream is single-subscription. It closes
  /// when the player ends it with code 0, and with a `PlayerException` first
  /// for any other end. Cancelling the subscription cancels the call; the
  /// future `cancel()` returns completes when the call is over.
  Stream<Uint8List> openStream(String method, Uint8List request);

  /// The player's status as it is pushed: on change, and at least once a
  /// second while playing for the clock. A broadcast stream.
  Stream<player.RemoteStatus> get statusStream;

  /// The player's status now: the one the connection started with, or the
  /// latest.
  Future<player.RemoteStatus> readStatus();

  /// Stops listening, fails every call still waiting and ends every stream.
  /// Whatever the channel was given to carry it on (a Bluetooth transport) is
  /// left to its owner; a channel that opened its own connection closes it.
  Future<void> dispose();
}
