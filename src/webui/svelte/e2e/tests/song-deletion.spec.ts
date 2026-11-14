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

import { test, expect } from "@playwright/test";
import { Code } from "@connectrpc/connect";
import { ConfigService } from "../../src/gen/player/v1/player_pb";
import { mockRpc } from "../support/rpc";

test.describe("Song Deletion", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs");
    await expect(
      page.locator(".song-row", { hasText: "Test Song Alpha" }),
    ).toBeVisible();
  });

  test("song row shows delete button on hover", async ({ page }) => {
    const songRow = page.locator(".song-row", { hasText: "Test Song Alpha" });
    await songRow.hover();
    await expect(songRow.locator(".song-row__delete")).toBeVisible();
  });

  test("delete calls API", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.unary(ConfigService.method.deleteSong, (req) => ({ name: req.name }));

    const songRow = page.locator(".song-row", { hasText: "Test Song Alpha" });
    await songRow.hover();
    await songRow.locator(".song-row__delete").click();

    // Confirm via custom dialog
    const dialog = page.locator('[role="dialog"]');
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Confirm" }).click();

    await expect
      .poll(() => rpc.calls(ConfigService.method.deleteSong).map((c) => c.name))
      .toEqual(["Test Song Alpha"]);
  });

  test("cancelling delete dialog does not call API", async ({ page }) => {
    const rpc = await mockRpc(page);

    const songRow = page.locator(".song-row", { hasText: "Test Song Alpha" });
    await songRow.hover();
    await songRow.locator(".song-row__delete").click();

    // Cancel via custom dialog
    const dialog = page.locator('[role="dialog"]');
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toBeHidden();

    expect(rpc.calls(ConfigService.method.deleteSong)).toHaveLength(0);
  });

  test("a delete the player refuses is shown", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.error(
      ConfigService.method.deleteSong,
      Code.FailedPrecondition,
      "Cannot delete a song that is currently playing",
    );

    const songRow = page.locator(".song-row", { hasText: "Test Song Alpha" });
    await songRow.hover();
    await songRow.locator(".song-row__delete").click();
    await page
      .locator('[role="dialog"]')
      .getByRole("button", { name: "Confirm" })
      .click();

    await expect(page.locator('[role="dialog"]')).toContainText(
      "Cannot delete a song that is currently playing",
    );
  });
});
