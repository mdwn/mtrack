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
import '../app/ports.dart';
import '../protocol/grpc_channel.dart';
import '../protocol/player_channel.dart';

/// Connects to players over the network with gRPC.
class GrpcWifiLink implements WifiLink {
  /// A link that opens [GrpcChannel]s.
  const GrpcWifiLink();

  @override
  Future<PlayerChannel> connect(
    String host,
    int port, {
    Duration timeout = GrpcChannel.connectTimeout,
  }) => GrpcChannel.connect(host, port, timeout: timeout);
}
