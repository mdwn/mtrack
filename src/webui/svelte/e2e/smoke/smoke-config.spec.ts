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
import { waitForWebSocket } from "./helpers.js";

test.describe("Smoke: Config", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await waitForWebSocket(page);
  });

  test("config page loads and shows profile", async ({ page }) => {
    // The smoke-generated profile "01-smoke" should appear.
    await expect(page.getByText("01-smoke")).toBeVisible({ timeout: 10000 });
  });

  test("profile shows AUDIO badge", async ({ page }) => {
    // The profile should show an AUDIO badge.
    await expect(page.getByText("AUDIO")).toBeVisible({ timeout: 10000 });
  });

  test("clicking profile opens profile editor", async ({ page }) => {
    // Click the profile row.
    await page.getByText("01-smoke").click();

    // Should show profile editor with tabs.
    await expect(page.locator(".tab").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("profile editor shows audio section", async ({ page }) => {
    await page.getByText("01-smoke").click();

    // Look for audio tab.
    const audioTab = page.locator(".tab", { hasText: "Audio" });
    await expect(audioTab).toBeVisible({ timeout: 10000 });
    await audioTab.click();

    // Should show audio device configuration.
    await expect(page.getByText(/device/i).first()).toBeVisible();
  });
});

test.describe("Smoke: Status Page", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/status");
    await waitForWebSocket(page);
  });

  test("status page shows build version", async ({ page }) => {
    await expect(page.getByText(/version/i)).toBeVisible({ timeout: 10000 });
  });

  test("status page shows hardware status", async ({ page }) => {
    await expect(page.getByText(/audio/i).first()).toBeVisible();
  });

  test("status page shows controller status", async ({ page }) => {
    await expect(page.getByText(/grpc/i)).toBeVisible({ timeout: 10000 });
  });
});
