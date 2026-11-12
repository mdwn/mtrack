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
import '../protocol/player_channel.dart';
import '../protocol/player_exception.dart';
import 'player_link.dart';
import 'ports.dart';

/// What to say when there is no connection to send a command on.
const String notConnectedMessage = 'Not connected to a player.';

/// What to say when the player does not answer.
const String noAnswerMessage = 'The player did not answer.';

/// A command's failure in a short sentence for a person.
String describeFailure(Object error) {
  if (error is PlayerException) {
    if (error.isPermissionDenied) {
      return 'The player is locked, or does not allow this from a phone.';
    }
    if (error.isTimeout || error.isUnavailable) {
      return noAnswerMessage;
    }
    // The player's own words say why it refused: a song cannot be changed
    // while another plays, a section does not exist, and so on.
    if (error.message.isNotEmpty) {
      return error.message;
    }
  }
  return 'Something went wrong talking to the player.';
}

/// Why a first connection to a player did not work, in words for a person.
String describeConnectFailure(Object error, AdapterState adapter) {
  switch (adapter) {
    case AdapterState.off:
      return 'Bluetooth is off. Turn it on and try again.';
    case AdapterState.permissionDenied:
      return 'mtrack is not allowed to use Bluetooth.';
    case AdapterState.unsupported:
      return 'This phone does not have Bluetooth Low Energy.';
    case AdapterState.ready:
      return 'Could not reach that player. Move closer and try again.';
  }
}

/// Why a first connection to a player known by [target]'s address did not
/// work, in words for a person.
String describeWifiFailure(Object error, RememberedPlayer target) {
  if (error is PlayerException && error.code == GrpcCode.unimplemented) {
    return 'That player is too old to connect over Wi-Fi. Update it, or '
        'connect to it by Bluetooth.';
  }
  final where = target.hosts.isEmpty ? 'that address' : target.hosts.first;
  return 'Nothing answered at $where. Check that your phone is on the '
      "player's network, and that the player's gRPC controller is turned "
      'on.';
}

/// The name of a radio for a person: `Wi-Fi` or `Bluetooth`.
String carrierName(Carrier carrier) =>
    carrier == Carrier.wifi ? 'Wi-Fi' : 'Bluetooth';
