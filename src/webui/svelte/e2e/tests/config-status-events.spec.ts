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

test.describe("Profile Editor - Status Events", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await page.locator(".tab", { hasText: "Status Events" }).click();
    await expect(page.locator(".tab.active")).toContainText("Status Events");
  });

  test("shows enable button when not configured", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /Enable Status Events/i }),
    ).toBeVisible();
  });

  test("enabling shows three event groups", async ({ page }) => {
    await page.getByRole("button", { name: /Enable Status Events/i }).click();
    await expect(page.getByText("Off Events")).toBeVisible();
    await expect(page.getByText("Idling Events")).toBeVisible();
    await expect(page.getByText("Playing Events")).toBeVisible();
  });

  test("add button creates a MIDI event editor", async ({ page }) => {
    await page.getByRole("button", { name: /Enable Status Events/i }).click();
    const offGroup = page.locator(".event-group").first();
    await offGroup.getByRole("button", { name: "Add" }).click();
    await expect(offGroup.locator(".event-row")).toHaveCount(1);
    await expect(
      offGroup.locator('[id^="status-off_events-0-type"]'),
    ).toBeVisible();
  });

  test("remove button deletes event from group", async ({ page }) => {
    await page.getByRole("button", { name: /Enable Status Events/i }).click();
    const offGroup = page.locator(".event-group").first();
    await offGroup.getByRole("button", { name: "Add" }).click();
    await expect(offGroup.locator(".event-row")).toHaveCount(1);
    await offGroup.locator(".remove-btn").click();
    await expect(offGroup.locator(".event-row")).toHaveCount(0);
  });
});
