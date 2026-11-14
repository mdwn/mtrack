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
import { LightingService } from "../../src/gen/player/v1/player_pb";
import { card } from "../support/locators";
import { mockRpc } from "../support/rpc";

// What a user typed is never thrown away without a word. Saves are caught
// here with page.route, so the shared mock is untouched.

test("a fixture type's blank or repeated channel blocks the save", async ({
  page,
}) => {
  const rpc = await mockRpc(page);
  await page.goto("/#/lighting/fixtures");
  await card(page, "par").click();
  const rows = page.getByTestId("ft-channel-row");
  await expect(rows).toHaveCount(4);
  await rows.nth(1).getByLabel("Channel name").fill("");
  await rows.nth(3).getByLabel("Channel name").fill("red");
  await page
    .locator(".editor-form")
    .getByRole("button", { name: "Save" })
    .click();
  await expect(page.locator(".save-msg")).toHaveText(
    "Not saved: 3 channels need attention.",
  );
  await expect(page.getByTestId("ft-channel-error")).toHaveText([
    "Another channel has this name.",
    "Give this channel a name.",
    "Another channel has this name.",
  ]);
  await expect(rows.nth(0).getByLabel("Channel name")).toBeFocused();
  await page.waitForTimeout(300);
  expect(rpc.calls(LightingService.method.saveFixtureType)).toHaveLength(0);
});

test("a tag with no allowed character stays in the box, marked", async ({
  page,
}) => {
  await page.goto("/#/lighting/venues");
  await page.getByRole("button", { name: "New Venue" }).click();
  await page.getByRole("button", { name: "Add Fixture" }).click();
  const input = page.locator(".tag-text-input").first();
  await input.fill("!!!");
  await input.press("Enter");
  await expect(input).toHaveValue("!!!");
  await expect(input).toHaveAttribute("aria-invalid", "true");
  await expect(page.getByTestId("tag-invalid")).toHaveText(
    "Tags use letters, digits, - and _ only.",
  );
  await input.fill("front");
  await input.press("Enter");
  await expect(page.locator(".tag-chip")).toContainText("front");
  await expect(page.getByTestId("tag-invalid")).toHaveCount(0);
});
