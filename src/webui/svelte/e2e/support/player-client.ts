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

// The player's gRPC service, called from a spec's own process (not a page)
// over gRPC-Web on the web server's port: what a spec that sets up or reads
// a real server does where it once called the REST API.
import { createClient } from "@connectrpc/connect";
import { createGrpcWebTransport } from "@connectrpc/connect-web";
import { PlayerService } from "../../src/gen/player/v1/player_pb";

/** A client of the player whose web server is at `baseUrl`
 *  (`http://127.0.0.1:8080`). */
export function playerClientAt(baseUrl: string) {
  return createClient(PlayerService, createGrpcWebTransport({ baseUrl }));
}
