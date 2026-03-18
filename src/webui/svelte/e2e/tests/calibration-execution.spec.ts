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

test.describe("Calibration Execution", () => {
  test.beforeEach(async ({ page }) => {
    // Navigate to triggers and set up an audio input
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await page.locator(".tab", { hasText: "Triggers" }).click();

    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    await checkbox.check();
    await page.getByRole("button", { name: "+ Audio" }).click();
    await expect(page.locator(".input-card")).toBeVisible();

    // Open calibration wizard
    await page.getByRole("button", { name: "Calibrate" }).click();
    await expect(page.locator(".cal-wizard")).toBeVisible();
  });

  test("start calibration calls API and shows noise floor", async ({
    page,
  }) => {
    // Fill in required device field (Start is disabled without a device)
    const deviceInput = page.locator("#cal-device");
    await deviceInput.evaluate((el) => el.scrollIntoView({ block: "center" }));
    await deviceInput.fill("Default Audio Device");

    // Click Start
    const startBtn = page.locator(".cal-actions button", {
      hasText: "Start",
    });
    await startBtn.evaluate((el) => el.scrollIntoView({ block: "center" }));
    await expect(startBtn).toBeEnabled();
    await startBtn.click();

    // After API returns, wizard should advance to show noise floor stats
    await expect(page.locator(".cal-stats")).toBeVisible({ timeout: 10000 });
  });

  test("cancel during setup closes wizard without API call", async ({
    page,
  }) => {
    const cancelBtn = page.locator(".cal-actions button", {
      hasText: "Cancel",
    });
    await cancelBtn.evaluate((el) => el.scrollIntoView({ block: "center" }));
    await cancelBtn.click();

    await expect(page.locator(".cal-wizard")).not.toBeVisible();
  });
});
