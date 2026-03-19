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

test.describe("Profile Editor", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    // Click the test-host profile to enter detail view
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
  });

  test("shows hostname field", async ({ page }) => {
    const hostnameInput = page.locator("#profile-hostname");
    await expect(hostnameInput).toBeVisible();
    await expect(hostnameInput).toHaveValue("test-host");
  });

  test("shows tab bar with sections", async ({ page }) => {
    await expect(page.locator(".tab-bar")).toBeVisible();
    await expect(page.locator(".tab", { hasText: "Audio" })).toBeVisible();
    await expect(page.locator(".tab", { hasText: "MIDI" })).toBeVisible();
    await expect(page.locator(".tab", { hasText: "Lighting" })).toBeVisible();
    await expect(page.locator(".tab", { hasText: "Triggers" })).toBeVisible();
    await expect(
      page.locator(".tab", { hasText: "Controllers" }),
    ).toBeVisible();
  });

  test("audio tab is active by default", async ({ page }) => {
    await expect(page.locator(".tab.active")).toContainText("Audio");
  });

  test("audio tab shows enabled state", async ({ page }) => {
    // The profile has audio configured, so audio section should be enabled
    await expect(page.locator(".tab-dot").first()).toBeVisible();
  });

  test("switching to MIDI tab works", async ({ page }) => {
    await page.locator(".tab", { hasText: "MIDI" }).click();
    await expect(page.locator(".tab.active")).toContainText("MIDI");
  });

  test("MIDI tab shows enable toggle when disabled", async ({ page }) => {
    await page.locator(".tab", { hasText: "MIDI" }).click();
    // MIDI is not configured in the mock, so it should show the enable checkbox
    const enableToggle = page.locator(".enable-toggle");
    await expect(enableToggle).toBeVisible();
  });

  test("switching to Controllers tab works", async ({ page }) => {
    await page.locator(".tab", { hasText: "Controllers" }).click();
    await expect(page.locator(".tab.active")).toContainText("Controllers");
  });

  test("save button starts disabled (no changes)", async ({ page }) => {
    const saveBtn = page.getByRole("button", { name: "Save" });
    await expect(saveBtn).toBeDisabled();
  });

  test("changing hostname enables save immediately", async ({ page }) => {
    const hostnameInput = page.locator("#profile-hostname");
    await hostnameInput.fill("new-hostname");
    // Uses oninput so save should enable without needing blur
    const saveBtn = page.getByRole("button", { name: "Save" });
    await expect(saveBtn).toBeEnabled();
  });

  test("delete button is visible for existing profiles", async ({ page }) => {
    await expect(page.getByRole("button", { name: "Delete" })).toBeVisible();
  });

  test("detail toolbar shows profile name", async ({ page }) => {
    await expect(page.locator(".detail-title")).toContainText("test-host");
  });

  test("audio section shows device dropdown", async ({ page }) => {
    // The audio section should show a device selector with mock devices
    const deviceSelect = page.locator("select").first();
    await expect(deviceSelect).toBeVisible();
  });

  test("hostname change is detected on every keystroke", async ({ page }) => {
    const hostnameInput = page.locator("#profile-hostname");
    // Type a single character
    await hostnameInput.pressSequentially("x");
    // Save should be enabled immediately (oninput, not onchange)
    await expect(page.getByRole("button", { name: "Save" })).toBeEnabled();
  });

  test("audio device dropdown shows mock devices", async ({ page }) => {
    const deviceSelect = page.locator("select").first();
    const options = deviceSelect.locator("option");
    // Should include at least the mock devices
    const count = await options.count();
    expect(count).toBeGreaterThanOrEqual(2);
  });
});
