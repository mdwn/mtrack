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

import { test, expect, type Page } from "@playwright/test";

interface SubsystemStatus {
  status: "connected" | "initializing" | "not_connected" | "not_configured";
  name: string | null;
}

interface ControllerStatus {
  kind: string;
  status: string;
  detail: string | null;
  error: string | null;
}

function statusFixture(overrides: {
  audio?: SubsystemStatus;
  midi?: SubsystemStatus;
  dmx?: SubsystemStatus;
  trigger?: SubsystemStatus;
  controllers?: ControllerStatus[];
  init_done?: boolean;
}) {
  return {
    build: {
      version: "0.1.0-test",
      git_hash: "deadbeef",
      build_time: "2026-01-01T00:00:00Z",
    },
    hardware: {
      init_done: overrides.init_done ?? true,
      hostname: "test-host",
      profile: "test-host",
      audio: overrides.audio ?? {
        status: "connected",
        name: "Default Audio Device",
      },
      midi: overrides.midi ?? { status: "not_connected", name: null },
      dmx: overrides.dmx ?? { status: "not_connected", name: null },
      trigger: overrides.trigger ?? { status: "not_connected", name: null },
    },
    controllers: overrides.controllers ?? [
      { kind: "osc", status: "running", detail: "0.0.0.0:9000", error: null },
    ],
  };
}

async function mockStatus(page: Page, body: unknown) {
  await page.route("**/api/status", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(body),
    });
  });
}

test.describe("Topnav health dot", () => {
  test("green when all required subsystems connected", async ({ page }) => {
    await mockStatus(page, statusFixture({}));
    await page.goto("/#/");

    const dot = page.locator(".topnav__conn");
    await expect(dot).toBeVisible();
    await expect(dot).not.toHaveClass(/topnav__conn--off/);
    await expect(dot).not.toHaveClass(/topnav__conn--warn/);
    await expect(dot).not.toHaveClass(/topnav__conn--error/);
  });

  test("red when audio is not_connected", async ({ page }) => {
    await mockStatus(
      page,
      statusFixture({
        audio: { status: "not_connected", name: "Default Audio Device" },
      }),
    );
    await page.goto("/#/");
    await expect(page.locator(".topnav__conn")).toHaveClass(
      /topnav__conn--error/,
    );
  });

  test("red when a configured MIDI device is not_connected", async ({
    page,
  }) => {
    await mockStatus(
      page,
      statusFixture({
        midi: { status: "not_connected", name: "Some MIDI Device" },
      }),
    );
    await page.goto("/#/");
    await expect(page.locator(".topnav__conn")).toHaveClass(
      /topnav__conn--error/,
    );
  });

  test("amber while a subsystem is initializing", async ({ page }) => {
    await mockStatus(
      page,
      statusFixture({
        audio: { status: "initializing", name: "Default Audio Device" },
      }),
    );
    await page.goto("/#/");
    await expect(page.locator(".topnav__conn")).toHaveClass(
      /topnav__conn--warn/,
    );
  });

  test("amber when a controller is in error", async ({ page }) => {
    await mockStatus(
      page,
      statusFixture({
        controllers: [
          {
            kind: "osc",
            status: "error",
            detail: "0.0.0.0:9000",
            error: "bind failed",
          },
        ],
      }),
    );
    await page.goto("/#/");
    await expect(page.locator(".topnav__conn")).toHaveClass(
      /topnav__conn--warn/,
    );
  });

  test("does NOT mark unconfigured MIDI/DMX as red", async ({ page }) => {
    // The default mock has midi/dmx as `not_connected` with `name: null`,
    // i.e. "not configured". That should remain green.
    await mockStatus(page, statusFixture({}));
    await page.goto("/#/");
    await expect(page.locator(".topnav__conn")).not.toHaveClass(
      /topnav__conn--error/,
    );
  });
});

test.describe("Logs row tinting", () => {
  // Helper to push a log line via the mock server's WS injection endpoint.
  async function pushLog(
    page: Page,
    level: "INFO" | "WARN" | "ERROR" | "DEBUG" | "TRACE",
    message: string,
  ) {
    await page.request.post("http://127.0.0.1:3111/test/send-ws", {
      data: {
        type: "logs",
        lines: [{ level, target: "mtrack::test", message }],
      },
    });
  }

  test("ERROR rows get error tint and left edge stripe", async ({ page }) => {
    await page.goto("/#/");
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );

    await pushLog(page, "ERROR", "ALSA underrun on default device");

    const errorRow = page.locator(".logs-card__line--ERROR").last();
    await expect(errorRow).toBeVisible();
    await expect(errorRow).toContainText("ALSA underrun on default device");
    // The CSS rule paints the row with rgba(232, 75, 75, 0.1). Computed
    // colors in Playwright come back as `rgb(...)` / `rgba(...)`, so we
    // assert the "is the styling actually applied" via getComputedStyle.
    const bg = await errorRow.evaluate(
      (el) => getComputedStyle(el).backgroundColor,
    );
    expect(bg).toMatch(/rgba?\(232,\s*75,\s*75/);
  });

  test("WARN rows get amber tint", async ({ page }) => {
    await page.goto("/#/");
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );

    await pushLog(page, "WARN", "frame dropped");

    const warnRow = page.locator(".logs-card__line--WARN").last();
    await expect(warnRow).toBeVisible();
    const bg = await warnRow.evaluate(
      (el) => getComputedStyle(el).backgroundColor,
    );
    expect(bg).toMatch(/rgba?\(242,\s*181,\s*68/);
  });

  test("INFO rows have no row-level tint", async ({ page }) => {
    await page.goto("/#/");
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );

    await pushLog(page, "INFO", "ordinary playback event");

    const infoRow = page
      .locator(".logs-card__line--INFO")
      .filter({ hasText: "ordinary playback event" })
      .last();
    await expect(infoRow).toBeVisible();
    const bg = await infoRow.evaluate(
      (el) => getComputedStyle(el).backgroundColor,
    );
    // The background is transparent / inherited from the feed, not pink/amber.
    expect(bg).not.toMatch(/rgba?\(232,\s*75,\s*75/);
    expect(bg).not.toMatch(/rgba?\(242,\s*181,\s*68/);
  });
});
