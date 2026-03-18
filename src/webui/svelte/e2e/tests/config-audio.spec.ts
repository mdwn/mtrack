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

test.describe("Profile Editor - Audio Section", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    // Audio tab should be active by default
    await expect(page.locator(".tab.active")).toContainText("Audio");
  });

  test("shows Enable Audio checkbox checked", async ({ page }) => {
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    await expect(checkbox).toBeChecked();
  });

  test("shows device input", async ({ page }) => {
    await expect(page.locator("#audio-device")).toBeVisible();
  });

  test("device input has current value", async ({ page }) => {
    await expect(page.locator("#audio-device")).toHaveValue("default");
  });

  test("shows sample rate select", async ({ page }) => {
    await expect(page.locator("#audio-sample-rate")).toBeVisible();
  });

  test("shows sample format select", async ({ page }) => {
    await expect(page.locator("#audio-sample-format")).toBeVisible();
  });

  test("shows buffer size input", async ({ page }) => {
    await expect(page.locator("#audio-buffer-size")).toBeVisible();
  });

  test("shows Refresh button for devices", async ({ page }) => {
    await expect(page.getByRole("button", { name: "Refresh" })).toBeVisible();
  });

  test("shows track mappings section", async ({ page }) => {
    await expect(page.getByText(/track mappings/i)).toBeVisible();
  });

  test("shows Add button for track mappings", async ({ page }) => {
    await expect(page.getByRole("button", { name: "Add" })).toBeVisible();
  });

  test("adding a track mapping creates a row", async ({ page }) => {
    await page.getByRole("button", { name: "Add" }).click();
    await expect(page.locator(".mapping-row")).toBeVisible();
  });

  test("shows resampler select", async ({ page }) => {
    await expect(page.locator("#audio-resampler")).toBeVisible();
  });

  test("shows playback delay input", async ({ page }) => {
    await expect(page.locator("#audio-delay")).toBeVisible();
  });
});
