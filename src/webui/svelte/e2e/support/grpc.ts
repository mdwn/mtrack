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

// Stubbing the player's gRPC-web service for specs that only need its calls
// to succeed. A spec that needs an answer, or the requests, uses ./rpc.ts.
import type { Page } from "@playwright/test";
import { mockRpc, type GrpcReply } from "./rpc";

export type { GrpcReply };

/** Answers every PlayerService call but the streams (`WatchStage`) with
 *  success (see {@link GrpcReply}), calling `onCall` first, if given. */
export async function stubPlayerService(
  page: Page,
  options: { reply?: GrpcReply; onCall?: () => void } = {},
): Promise<void> {
  await mockRpc(page, {
    otherwise: options.reply ?? "trailers-only",
    onCall: () => options.onCall?.(),
  });
}
