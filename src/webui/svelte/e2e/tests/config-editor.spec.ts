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
import { ConfigEditorPage } from "../pages/config-editor.page.js";

test.describe("Config Editor", () => {
  let config: ConfigEditorPage;

  test.beforeEach(async ({ page }) => {
    config = new ConfigEditorPage(page);
    await config.goto();
  });

  test("shows Hardware Profiles heading", async () => {
    await expect(config.heading).toBeVisible();
  });

  test("shows Add Profile button", async () => {
    await expect(config.addProfileButton).toBeVisible();
  });

  test("lists inline profiles from config", async () => {
    await expect(config.profileByName("test-host")).toBeVisible();
  });

  test("profile row shows hostname", async () => {
    const row = config.profileByName("test-host");
    await expect(row.locator(".row-hostname")).toContainText("test-host");
  });

  test("profile row shows subsystem badge", async () => {
    const row = config.profileByName("test-host");
    await expect(row.locator(".badge")).toHaveCount(1); // audio only
  });

  test("shows audio badge", async () => {
    const row = config.profileByName("test-host");
    await expect(row.locator(".badge-audio")).toBeVisible();
  });

  test("clicking a profile navigates to detail", async ({ page }) => {
    await config.profileByName("test-host").click();
    await expect(page).toHaveURL(/.*#\/config\/test-host/);
  });

  test("profile detail shows back button", async () => {
    await config.profileByName("test-host").click();
    await expect(config.backButton).toBeVisible();
  });

  test("back button returns to list view", async ({ page }) => {
    await config.profileByName("test-host").click();
    await expect(config.backButton).toBeVisible();
    await config.backButton.click();
    await expect(config.heading).toBeVisible();
    await expect(page).toHaveURL(/.*#\/config$/);
  });

  test("shows Samples section", async () => {
    await expect(config.samplesHeading).toBeVisible();
  });
});
