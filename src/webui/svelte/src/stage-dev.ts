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

// The stage-only page on a desktop, with no phone: something to play the
// host's part. Dev server only (`npm run dev`); stage-main.ts loads this
// behind `import.meta.env.DEV`, so the built page does not carry it.
//
//   /stage.html?dev=replay   the recorded stream the tests use
//                            (e2e/fixtures/stage-stream.json), round and
//                            round, two updates a second (`&rate=` changes it)
//   /stage.html?dev=grpc     a running player's WatchStage, over gRPC-Web
//                            through the dev server's proxy
//                            (VITE_PROXY_TARGET, default 127.0.0.1:8080)
//
// Both go through `window.mtrackStage.update`, base64 and all, as the phone
// app's do.

import { toBinary } from "@bufbuild/protobuf";
import { createClient } from "@connectrpc/connect";
import { createGrpcWebTransport } from "@connectrpc/connect-web";
import { PlayerService, StageUpdateSchema } from "./gen/player/v1/player_pb";

const sleep = (ms: number) => new Promise((done) => setTimeout(done, ms));

function base64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

async function replay(rate: number): Promise<never> {
  const { default: recording } =
    await import("../e2e/fixtures/stage-stream.json");
  for (;;) {
    for (const step of recording.steps) {
      window.mtrackStage.update(step.update);
      await sleep(1000 / rate);
    }
    // A new stream: its layouts are numbered from 1 again.
    window.mtrackStage.reset();
    await sleep(1000 / rate);
  }
}

async function watch(): Promise<never> {
  const client = createClient(
    PlayerService,
    createGrpcWebTransport({ baseUrl: location.origin }),
  );
  for (;;) {
    try {
      for await (const update of client.watchStage({ maxRate: 10 })) {
        window.mtrackStage.update(base64(toBinary(StageUpdateSchema, update)));
      }
    } catch (error) {
      console.warn("stage stream ended", error);
    }
    window.mtrackStage.reset();
    await sleep(2000);
  }
}

/** Plays the host: `mode` is the page's `?dev=`. */
export function startDev(mode: string): void {
  const params = new URLSearchParams(location.search);
  if (mode === "replay") void replay(Number(params.get("rate")) || 2);
  else if (mode === "grpc") void watch();
  else console.warn(`stage: unknown ?dev=${mode} (replay or grpc)`);
}
