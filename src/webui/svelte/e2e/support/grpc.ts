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
// to succeed.
import type { Page } from "@playwright/test";

/** How a stubbed call answers.
 *  - `trailers-only`: an empty body, its status in the headers.
 *  - `framed`: a well-formed unary reply, an empty message frame then a
 *    trailers frame. A bare empty body makes the client's response parsing
 *    reject, which silently aborts call chains like pause's stop-then-seek:
 *    a spec that follows such a chain needs this one. */
export type GrpcReply = "trailers-only" | "framed";

function framedReply(): Buffer {
  const trailers = Buffer.from("grpc-status: 0\r\n");
  return Buffer.concat([
    Buffer.from([0x00, 0, 0, 0, 0]),
    Buffer.from([0x80, 0, 0, 0, trailers.length]),
    trailers,
  ]);
}

/** Answers every PlayerService call but the stage stream with success (see {@link GrpcReply}),
 *  calling `onCall` first, if given. */
export async function stubPlayerService(
  page: Page,
  options: { reply?: GrpcReply; onCall?: () => void } = {},
): Promise<void> {
  const reply = options.reply ?? "trailers-only";
  await page.route("**/player.v1.PlayerService/**", async (route) => {
    // The stage stream is the mock server's (support/stage.ts), not a call
    // to stub.
    if (route.request().url().endsWith("/WatchStage")) {
      await route.fallback();
      return;
    }
    options.onCall?.();
    await route.fulfill(
      reply === "framed"
        ? {
            status: 200,
            headers: { "content-type": "application/grpc-web+proto" },
            body: framedReply(),
          }
        : {
            status: 200,
            headers: {
              "content-type": "application/grpc-web+proto",
              "grpc-status": "0",
            },
            body: Buffer.alloc(0),
          },
    );
  });
}
