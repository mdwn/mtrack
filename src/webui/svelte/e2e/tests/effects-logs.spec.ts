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

  test("log level filter pills are visible", async ({ page }) => {
    const pills = page.locator(".log-level-pill");
    await expect(pills).toHaveCount(5);
  });

  test("INFO, WARN, ERROR pills are active by default", async ({ page }) => {
    await expect(page.locator(".log-level-pill.level-INFO")).toHaveClass(
      /active/,
    );
    await expect(page.locator(".log-level-pill.level-WARN")).toHaveClass(
      /active/,
    );
    await expect(page.locator(".log-level-pill.level-ERROR")).toHaveClass(
      /active/,
    );
  });

  test("TRACE and DEBUG pills are inactive by default", async ({ page }) => {
    await expect(page.locator(".log-level-pill.level-TRACE")).not.toHaveClass(
      /active/,
    );
    await expect(page.locator(".log-level-pill.level-DEBUG")).not.toHaveClass(
      /active/,
    );
  });

  test("clicking a pill toggles its state", async ({ page }) => {
    const infoPill = page.locator(".log-level-pill.level-INFO");
    await expect(infoPill).toHaveClass(/active/);
    await infoPill.click();
    await expect(infoPill).not.toHaveClass(/active/);
    await expect(page.locator(".log-line")).toHaveCount(0);
  });

  test("clicking a disabled pill shows its logs", async ({ page }) => {
    await expect(page.locator(".log-line")).toHaveCount(2);
    const debugPill = page.locator(".log-level-pill.level-DEBUG");
    await expect(debugPill).not.toHaveClass(/active/);
    await debugPill.click();
    await expect(debugPill).toHaveClass(/active/);
    await expect(page.locator(".log-line")).toHaveCount(3);
    await expect(page.locator(".log-line.level-DEBUG")).toContainText(
      "DMX frame sent",
    );
  });
});
