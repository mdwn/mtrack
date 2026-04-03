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

test.describe("Global Max Sample Voices", () => {
  test("shows max sample voices field on config page", async ({ page }) => {
    await page.goto("/#/config");
    await expect(page.locator("#max-sample-voices-list")).toBeVisible();
  });

  test("field has placeholder of 32", async ({ page }) => {
    await page.goto("/#/config");
    const input = page.locator("#max-sample-voices-list");
    await expect(input).toHaveAttribute("placeholder", "32");
  });

  test("changing value marks samples as dirty", async ({ page }) => {
    await page.goto("/#/config");
    const input = page.locator("#max-sample-voices-list");
    await input.fill("64");
    await input.dispatchEvent("change");
    // The save button should become enabled
    const saveBtn = page
      .locator(".samples-top-section")
      .getByRole("button", { name: /Save Samples/i });
    await expect(saveBtn).toBeEnabled();
  });
});
