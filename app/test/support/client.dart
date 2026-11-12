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
import 'dart:math';

import 'package:mtrack_remote/src/protocol/envelope_channel.dart';
import 'package:mtrack_remote/src/protocol/player_client.dart';
import 'package:mtrack_remote/src/protocol/transport.dart';

final Expando<EnvelopeChannel> _channels = Expando<EnvelopeChannel>();

/// A client over the Bluetooth envelope on [transport].
PlayerClient clientOver(PlayerTransport transport, {Random? random}) {
  final channel = EnvelopeChannel(transport, random: random);
  final client = PlayerClient(channel);
  _channels[client] = channel;
  return client;
}

/// Sets the id [client]'s next request takes.
void setNextId(PlayerClient client, int id) =>
    _channels[client]!.debugNextId = id;
