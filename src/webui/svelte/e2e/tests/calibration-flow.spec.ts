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

test.describe("Calibration Flow", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await page.locator(".tab", { hasText: "Triggers" }).click();

    // Enable triggers and add an audio input
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    await checkbox.check();
    await page.getByRole("button", { name: "+ Audio" }).click();
    await expect(page.locator(".input-card")).toBeVisible();
  });

  test("calibration wizard shows setup fields", async ({ page }) => {
    await page.getByRole("button", { name: "Calibrate" }).click();
    await expect(page.locator(".cal-wizard")).toBeVisible();

    // Scroll wizard into view and check fields
    await page.locator(".cal-wizard").scrollIntoViewIfNeeded();
    await expect(page.locator(".cal-wizard")).toContainText(/channel/i);
    await expect(page.locator(".cal-wizard")).toContainText(/duration/i);
  });

  test("calibration wizard shows Start and Cancel buttons", async ({
    page,
  }) => {
    await page.getByRole("button", { name: "Calibrate" }).click();
    await page.locator(".cal-wizard").scrollIntoViewIfNeeded();

    const startBtn = page
      .locator(".cal-wizard")
      .getByRole("button", { name: "Start" });
    const cancelBtn = page
      .locator(".cal-wizard")
      .getByRole("button", { name: "Cancel" });
    await expect(startBtn).toBeVisible();
    await expect(cancelBtn).toBeVisible();
  });

  test("calibration wizard shows hint when device is empty", async ({
    page,
  }) => {
    await page.getByRole("button", { name: "Calibrate" }).click();
    await page.locator(".cal-wizard").scrollIntoViewIfNeeded();

    // Start button should be disabled and hint should be visible
    const startBtn = page
      .locator(".cal-wizard")
      .getByRole("button", { name: "Start" });
    await expect(startBtn).toBeDisabled();
    await expect(page.locator(".cal-hint")).toBeVisible();
    await expect(page.locator(".cal-hint")).toContainText(/select a device/i);
  });

  test("calibration hint disappears when device is filled", async ({
    page,
  }) => {
    await page.getByRole("button", { name: "Calibrate" }).click();
    await page.locator(".cal-wizard").scrollIntoViewIfNeeded();
    await expect(page.locator(".cal-hint")).toBeVisible();

    // Fill in the device
    const deviceInput = page.locator("#cal-device");
    await deviceInput.evaluate((el) => el.scrollIntoView({ block: "center" }));
    await deviceInput.fill("Default Audio Device");

    // Hint should disappear and Start should be enabled
    await expect(page.locator(".cal-hint")).not.toBeVisible();
    await expect(
      page.locator(".cal-wizard").getByRole("button", { name: "Start" }),
    ).toBeEnabled();
  });

  test("calibration cancel closes wizard", async ({ page }) => {
    await page.getByRole("button", { name: "Calibrate" }).click();
    await page.locator(".cal-wizard").scrollIntoViewIfNeeded();

    const cancelBtn = page
      .locator(".cal-wizard")
      .getByRole("button", { name: "Cancel" });
    await cancelBtn.scrollIntoViewIfNeeded();
    await cancelBtn.click();

    await expect(page.locator(".cal-wizard")).not.toBeVisible();
  });
});
