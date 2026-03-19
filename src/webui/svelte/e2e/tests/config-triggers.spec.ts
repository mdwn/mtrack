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

test.describe("Profile Editor - Triggers Section", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    // Click the Triggers tab, then enable the section
    await page.locator(".tab", { hasText: "Triggers" }).click();
    await expect(page.locator(".tab.active")).toContainText("Triggers");
    await page.getByRole("button", { name: "Enable Triggers" }).click();
  });

  test("triggers tab shows input controls", async ({ page }) => {
    // Should show Add Audio and Add MIDI buttons
    await expect(page.getByRole("button", { name: "+ Audio" })).toBeVisible();
    await expect(page.getByRole("button", { name: "+ MIDI" })).toBeVisible();
  });

  test("adding audio input shows input card", async ({ page }) => {
    await page.getByRole("button", { name: "+ Audio" }).click();
    await expect(page.locator(".input-card")).toBeVisible();
    // Should show the AUDIO header
    await expect(page.locator(".input-kind")).toContainText(/audio/i);
  });

  test("adding MIDI input shows MIDI fields", async ({ page }) => {
    await page.getByRole("button", { name: "+ MIDI" }).click();
    await expect(page.locator(".input-card.midi")).toBeVisible();
    // Should show MIDI-specific fields
    await expect(page.locator('[id^="trigger-midi-type-"]')).toBeVisible();
    await expect(page.locator('[id^="trigger-midi-ch-"]')).toBeVisible();
  });

  test("audio input shows Calibrate button", async ({ page }) => {
    await page.getByRole("button", { name: "+ Audio" }).click();
    await expect(page.getByRole("button", { name: "Calibrate" })).toBeVisible();
  });

  test("audio input shows More/Less toggle for advanced settings", async ({
    page,
  }) => {
    await page.getByRole("button", { name: "+ Audio" }).click();
    await expect(
      page.getByRole("button", { name: "More", exact: true }),
    ).toBeVisible();
  });

  test("More button reveals advanced settings", async ({ page }) => {
    await page.getByRole("button", { name: "+ Audio" }).click();
    await page.getByRole("button", { name: "More", exact: true }).click();

    // Advanced fields should be visible
    await expect(page.locator('[id^="trigger-retrig-"]')).toBeVisible();
    await expect(page.locator('[id^="trigger-scan-"]')).toBeVisible();
    await expect(page.locator('[id^="trigger-vel-"]')).toBeVisible();

    // Button should now say "Less"
    await expect(
      page.getByRole("button", { name: "Less", exact: true }),
    ).toBeVisible();
  });

  test("removing input removes card", async ({ page }) => {
    await page.getByRole("button", { name: "+ Audio" }).click();
    await expect(page.locator(".input-card")).toHaveCount(1);

    // Click the X remove button (red button in the input header)
    await page
      .locator(".input-card .input-header-controls .btn-small")
      .last()
      .click();
    await expect(page.locator(".input-card")).toHaveCount(0);
  });

  test("calibration wizard opens on Calibrate click", async ({ page }) => {
    await page.getByRole("button", { name: "+ Audio" }).click();
    await page.getByRole("button", { name: "Calibrate" }).click();

    // Should show calibration wizard with Start button
    await expect(page.locator(".cal-wizard")).toBeVisible();
    await expect(page.getByRole("button", { name: "Start" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Cancel" })).toBeVisible();
  });

  test("calibration wizard shows device and channel inputs", async ({
    page,
  }) => {
    await page.getByRole("button", { name: "+ Audio" }).click();
    await page.getByRole("button", { name: "Calibrate" }).click();

    await expect(page.locator("#cal-device")).toBeVisible();
    await expect(page.locator("#cal-channel")).toBeVisible();
    await expect(page.locator("#cal-duration")).toBeVisible();
  });

  test("calibration cancel closes wizard", async ({ page }) => {
    await page.getByRole("button", { name: "+ Audio" }).click();
    await page.getByRole("button", { name: "Calibrate" }).click();
    await expect(page.locator(".cal-wizard")).toBeVisible();

    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.locator(".cal-wizard")).not.toBeVisible();
  });
});
