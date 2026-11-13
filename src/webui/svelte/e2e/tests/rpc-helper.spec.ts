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

// The gRPC answer helper (support/rpc.ts), on calls the app really makes:
// the transport buttons.
import { test, expect } from "@playwright/test";
import { Code } from "@connectrpc/connect";
import { PlayerService } from "../../src/gen/player/v1/player_pb";
import { mockRpc } from "../support/rpc";

test.describe("Answering gRPC calls from a spec", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/");
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );
  });

  test("records the request a button sends, by descriptor or by name", async ({
    page,
  }) => {
    const rpc = await mockRpc(page);
    rpc.unary(PlayerService.method.play, {});

    await page.getByRole("button", { name: "Play" }).click();

    await expect
      .poll(() => rpc.calls(PlayerService.method.play).length)
      .toBe(1);
    expect(rpc.calls("PlayerService/Play")).toHaveLength(1);
    expect(rpc.calls(PlayerService.method.stop)).toHaveLength(0);
    expect(rpc.allCalls.map((c) => c.method)).toEqual(["Play"]);
    await expect(page.getByRole("alert")).toHaveCount(0);
  });

  test("answers with a function of the request", async ({ page }) => {
    const rpc = await mockRpc(page);
    const seen: unknown[] = [];
    rpc.unary("player.v1.PlayerService/Play", (request) => {
      seen.push(request);
      return {};
    });

    await page.getByRole("button", { name: "Play" }).click();

    await expect.poll(() => seen.length).toBe(1);
    await expect(page.getByRole("alert")).toHaveCount(0);
  });

  test("an error answer shows the page's failure", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.error(
      PlayerService.method.play,
      Code.FailedPrecondition,
      "Player is locked. Unlock to make changes.",
    );

    await page.getByRole("button", { name: "Play" }).click();

    await expect(page.getByRole("alert")).toContainText("Play failed");
    expect(rpc.calls(PlayerService.method.play)).toHaveLength(1);
  });

  test("a call nothing answers falls through, and streams are left alone", async ({
    page,
  }) => {
    const rpc = await mockRpc(page);
    rpc.unary(PlayerService.method.stop, {});

    // The stage stream is open on the page (the mock server answers it), and
    // is not a call the helper takes.
    await page.goto("/#/lighting");
    await expect(page.locator(".app-nav, nav").first()).toBeVisible();
    expect(rpc.allCalls.map((c) => c.method)).not.toContain("WatchStage");
  });

  test("a method that does not exist is refused by name", async ({ page }) => {
    const rpc = await mockRpc(page);
    expect(() => rpc.unary("PlayerService/NoSuchCall", {})).toThrow(
      /no method/,
    );
    expect(() =>
      rpc.unary(PlayerService.method.watchStage as never, {}),
    ).toThrow(/stream/);
  });
});
