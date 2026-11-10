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

  /// The request was malformed.
  static const int invalidArgument = 3;

  /// The player did not answer in time.
  static const int deadlineExceeded = 4;

  /// The named song, playlist or section does not exist.
  static const int notFound = 5;

  /// The call is not allowed over Bluetooth, or the player is locked.
  static const int permissionDenied = 7;

  /// The response did not fit in a frame.
  static const int resourceExhausted = 8;

  /// The player is in a state that refuses the call.
  static const int failedPrecondition = 9;

  /// There is no link to the player.
  static const int unavailable = 14;
}

/// A call that did not succeed: the player refused it, or no answer came.
class PlayerException implements Exception {
  /// An exception with the gRPC [code] and a [message] for the user.
  const PlayerException(this.code, this.message);

  /// The call got no answer in time.
  const PlayerException.timeout(String method)
    : this(GrpcCode.deadlineExceeded, 'timed out waiting for $method');

  /// The link was down, or went down before the answer came.
  const PlayerException.unavailable([String message = 'not connected'])
    : this(GrpcCode.unavailable, message);

  /// The gRPC status code.
  final int code;

  /// The player's description, or ours for a timeout or a lost link.
  final String message;

  /// True when the player is locked or the call is not allowed.
  bool get isPermissionDenied => code == GrpcCode.permissionDenied;

  /// True when the call was not answered in time.
  bool get isTimeout => code == GrpcCode.deadlineExceeded;

  /// True when there was no link.
  bool get isUnavailable => code == GrpcCode.unavailable;

  @override
  String toString() => 'PlayerException($code): $message';
}
