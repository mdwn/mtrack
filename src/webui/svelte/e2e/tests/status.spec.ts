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
import { StatusPageModel } from "../pages/status.page.js";

test.describe("Status Page", () => {
  let status: StatusPageModel;

  test.beforeEach(async ({ page }) => {
    status = new StatusPageModel(page);
    await status.goto();
  });

  test("shows Status heading", async () => {
    await expect(status.heading).toBeVisible();
  });

  test("shows refresh button", async () => {
    await expect(status.refreshButton).toBeVisible();
  });

  test("shows three cards", async () => {
    await expect(status.cards).toHaveCount(3);
  });

  test("shows build info card with version", async () => {
    await expect(status.buildInfoCard).toBeVisible();
    await expect(status.buildInfoCard).toContainText("0.1.0-test");
  });

  test("shows git hash", async () => {
    await expect(status.buildInfoCard).toContainText("deadbeef");
  });

  test("shows build time", async () => {
    await expect(status.buildInfoCard).toContainText("2026-01-01");
  });

  test("shows hardware card", async () => {
    await expect(status.hardwareCard).toBeVisible();
  });

  test("shows hostname", async () => {
    await expect(status.hardwareCard).toContainText("test-host");
  });

  test("shows audio subsystem as connected", async () => {
    const audioRow = status.subsystemRow("Audio");
    await expect(audioRow).toBeVisible();
    await expect(audioRow).toContainText("Connected");
  });

  test("shows audio device name", async () => {
    const audioRow = status.subsystemRow("Audio");
    await expect(audioRow).toContainText("Default Audio Device");
  });

  test("shows MIDI subsystem as not connected", async () => {
    const midiRow = status.subsystemRow("MIDI");
    await expect(midiRow).toBeVisible();
    await expect(midiRow).toContainText("Not Connected");
  });

  test("shows DMX subsystem as not connected", async () => {
    const dmxRow = status.subsystemRow("DMX");
    await expect(dmxRow).toBeVisible();
    await expect(dmxRow).toContainText("Not Connected");
  });

  test("shows Trigger subsystem as not connected", async () => {
    const triggerRow = status.subsystemRow("Trigger");
    await expect(triggerRow).toBeVisible();
    await expect(triggerRow).toContainText("Not Connected");
  });

  test("shows controllers card", async () => {
    await expect(status.controllersCard).toBeVisible();
  });

  test("shows OSC controller as running", async () => {
    await expect(status.controllersCard).toContainText("OSC");
    await expect(status.controllersCard).toContainText("Running");
    await expect(status.controllersCard).toContainText("0.0.0.0:9000");
  });

  test("shows restart button for controllers", async () => {
    await expect(status.restartButton).toBeVisible();
  });

  test("refresh button fetches status", async () => {
    await status.refreshButton.click();
    // After refresh, data should still be displayed
    await expect(status.buildInfoCard).toContainText("0.1.0-test");
  });

  test("restart controllers calls API", async ({ page }) => {
    let restartCalled = false;
    await page.route("**/api/controllers/restart", async (route) => {
      restartCalled = true;
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          status: "restarted",
          controllers: [
            {
              kind: "osc",
              status: "running",
              detail: "0.0.0.0:9000",
              error: null,
            },
          ],
        }),
      });
    });

    await status.restartButton.click();
    expect(restartCalled).toBe(true);
  });

  test("shows error banner on API failure", async ({ page }) => {
    // Navigate away from status first, then set up the intercept,
    // then navigate back so the component remounts and refetches.
    await page.goto("/#/");
    await page.route("**/api/status", async (route) => {
      await route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({ error: "Internal server error" }),
      });
    });

    await page.goto("/#/status");
    const freshStatus = new StatusPageModel(page);
    await expect(freshStatus.errorBanner).toBeVisible();
    await expect(freshStatus.errorBanner).toContainText(
      "Failed to load status",
    );
  });
});
