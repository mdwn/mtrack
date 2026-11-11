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

/// Where the link to a player stands.
enum TransportState {
  /// No link. Nothing can be written.
  disconnected,

  /// A link is being set up: connecting, discovering the service, subscribing.
  connecting,

  /// The link is up, notifications are subscribed and [PlayerTransport.maxChunk]
  /// is valid.
  ready,
}

/// The seam between the protocol and the radio.
///
/// An implementation moves opaque chunks (see `frame_codec.dart`) to and from
/// the player's GATT characteristics and reports the link's state. It knows
/// nothing of frames, requests or messages, so everything that can be decided
/// without a radio is decided above it.
abstract class PlayerTransport {
  /// The state now. Callers that start after the link came up read this
  /// instead of waiting for a change.
  TransportState get currentState;

  /// Every change of state, in order. A broadcast stream.
  Stream<TransportState> get state;

  /// The largest chunk to write: the negotiated ATT MTU minus the 3 bytes ATT
  /// keeps. Valid while the state is [TransportState.ready].
  int get maxChunk;

  /// Writes one chunk to the `request` characteristic, with response, and
  /// completes when the player has acknowledged it. Throws when the link is
  /// not up or the write fails.
  Future<void> write(Uint8List chunk);

  /// Notifications on the `response` characteristic, one chunk each. A
  /// broadcast stream.
  Stream<Uint8List> get responseChunks;

  /// Notifications on the `status` characteristic, one chunk each. A broadcast
  /// stream.
  Stream<Uint8List> get statusChunks;

  /// Whether the player has a `stream` characteristic and the transport is
  /// subscribed to it. False against a player from before streams, which is
  /// connected to and called exactly as one with them is. Valid while the
  /// state is [TransportState.ready].
  bool get hasStreams;

  /// Notifications on the `stream` characteristic, one chunk each: the
  /// messages of server streams and their ends. A broadcast stream, silent
  /// when [hasStreams] is false.
  Stream<Uint8List> get streamChunks;

  /// Reads the `status` characteristic: a bare encoded `Status`, not chunked.
  Future<Uint8List> readStatus();

  /// Drops the link and releases the transport. The state ends at
  /// [TransportState.disconnected].
  Future<void> close();
}
