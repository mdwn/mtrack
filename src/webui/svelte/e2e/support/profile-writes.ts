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

// The inline profiles a page wrote, read off a `mockRpc`: what a spec
// reads of a profile save.
//
//   const rpc = await mockRpc(page);
//   await page.getByRole("button", { name: "Save" }).click();
//   const { profile } = await profileWrite(rpc);
import { expect } from "@playwright/test";
import { PlayerService } from "../../src/gen/player/v1/player_pb";
import type { RpcMock } from "./rpc";

/** One `UpdateProfile` call, its profile parsed. */
export interface ProfileWrite {
  index: number;
  expectedChecksum: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- a profile as a spec digs into it
  profile: any;
}

/** Waits for the `n`th (0-based) `UpdateProfile` the page makes, and
 *  answers it. The call is answered by `rpc` if a spec registered an
 *  answer, else by the mock server. */
export async function profileWrite(rpc: RpcMock, n = 0): Promise<ProfileWrite> {
  await expect
    .poll(() => rpc.calls(PlayerService.method.updateProfile).length, {
      timeout: 10000,
    })
    .toBeGreaterThan(n);
  const call = rpc.calls(PlayerService.method.updateProfile)[n];
  return {
    index: call.index,
    expectedChecksum: call.expectedChecksum,
    profile: JSON.parse(call.profileJson),
  };
}
