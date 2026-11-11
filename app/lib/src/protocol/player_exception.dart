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
/// The gRPC status codes the app tells apart. The player carries them in the
/// `code` of a `Response`; 0 is OK.
abstract final class GrpcCode {
  /// The call succeeded.
  static const int ok = 0;

  /// The stream was cancelled, by the phone.
  static const int cancelled = 1;

  /// The request was malformed.
  static const int invalidArgument = 3;

  /// The player did not answer in time.
  static const int deadlineExceeded = 4;

  /// The named song, playlist, section or lighting asset does not exist.
  static const int notFound = 5;

  /// The playlist to create already exists.
  static const int alreadyExists = 6;

  /// The call is not allowed over Bluetooth, or the player is locked.
  static const int permissionDenied = 7;

  /// The response did not fit in a frame, the phone has as many streams open
  /// as it may, or a stream's listener fell too far behind it.
  static const int resourceExhausted = 8;

  /// The player is in a state that refuses the call.
  static const int failedPrecondition = 9;

  /// The player has nothing to carry a stream on: it is from before streams.
  /// Never sent by a player; the app's own, for a stream it did not ask for.
  static const int unimplemented = 12;

  /// There is no link to the player.
  static const int unavailable = 14;

  /// A lighting asset arrived short or damaged. The app's own: the player
  /// sends what it has.
  static const int dataLoss = 15;
}

/// A call that did not succeed: the player refused it, or no answer came.
class PlayerException implements Exception {
  /// An exception with the gRPC [code] and a [message] for the user.
  const PlayerException(this.code, this.message, {this.unsent = false});

  /// The call got no answer in time.
  const PlayerException.timeout(String method)
    : this(GrpcCode.deadlineExceeded, 'timed out waiting for $method');

  /// The link was down, or went down before the answer came.
  const PlayerException.unavailable([String message = 'not connected'])
    : this(GrpcCode.unavailable, message);

  /// The request could not be written: the link took nothing, though it
  /// was thought to be up.
  const PlayerException.unsent(String message)
    : this(GrpcCode.unavailable, message, unsent: true);

  /// The player has no `stream` characteristic, so no stream was asked for.
  const PlayerException.noStreams()
    : this(GrpcCode.unimplemented, 'the player is too old for streams');

  /// The gRPC status code.
  final int code;

  /// True when the request never left the phone, as against a player that
  /// answered with a refusal.
  final bool unsent;

  /// The player's description, or ours for a timeout or a lost link.
  final String message;

  /// True when the player is locked or the call is not allowed.
  bool get isPermissionDenied => code == GrpcCode.permissionDenied;

  /// True when the call was not answered in time.
  bool get isTimeout => code == GrpcCode.deadlineExceeded;

  /// True when there was no link.
  bool get isUnavailable => code == GrpcCode.unavailable;

  /// True when a stream was not opened because the player is from before
  /// streams: it has no `stream` characteristic, or it refused the method as
  /// one it does not allow.
  bool get isStreamTooOld =>
      code == GrpcCode.unimplemented || code == GrpcCode.permissionDenied;

  @override
  String toString() => 'PlayerException($code): $message';
}
