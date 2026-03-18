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

test.describe("Effects and Logs Cards", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/");
    // Wait for WS data
    await expect(page.locator(".playback-song")).toContainText(
      "Test Song Alpha",
    );
  });

  test("effects card shows active effects from WebSocket", async ({ page }) => {
    // Mock server sends FIXTURE_STATE with active_effects: ["color_wash", "strobe_pulse"]
    const effectsList = page.locator(".effects-list li");
    await expect(effectsList).toHaveCount(2);
    await expect(effectsList.nth(0)).toContainText("color_wash");
    await expect(effectsList.nth(1)).toContainText("strobe_pulse");
  });

  test("logs card shows log lines from WebSocket", async ({ page }) => {
    // Mock server sends LOG_LINES with 2 log entries
    const logLines = page.locator(".log-line");
    await expect(logLines.first()).toBeVisible();
    await expect(logLines).toHaveCount(2);
  });

  test("log lines show level and message", async ({ page }) => {
    const firstLog = page.locator(".log-line").first();
    await expect(firstLog).toContainText("INFO");
    await expect(firstLog).toContainText("Player started");
  });

  test("log lines show target", async ({ page }) => {
    const firstLog = page.locator(".log-line").first();
    await expect(firstLog).toContainText("mtrack::player");
  });
});
