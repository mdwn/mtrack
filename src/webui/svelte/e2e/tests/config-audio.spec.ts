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

  test("device options show the channel count when it is known", async ({
    page,
  }) => {
    const option = page.locator(
      "#audio-device-list option[value='USB Interface']",
    );
    await expect(option).toHaveText("USB Interface (18ch, ALSA)");
  });

  // An ALSA plug node reports cpal's clamped maximum regardless of the
  // hardware behind it, so the picker must not present that as a capability.
  // It stays in the list: on some interfaces it is the only node that works.
  test("plug nodes are labelled rather than given a fictional channel count", async ({
    page,
  }) => {
    const option = page.locator(
      "#audio-device-list option[value='alsa:plughw:CARD=WING']",
    );
    await expect(option).toHaveText("alsa:plughw:CARD=WING (virtual, ALSA)");
    await expect(option).not.toContainText("64");
  });

  // Setting a rig up happens in this editor, not in a terminal, so the check
  // that a device will actually stream has to be reachable from here.
  test("testing a device reports that it streams", async ({ page }) => {
    await page.locator("#audio-test").click();
    const result = page.locator("#audio-test-result");
    await expect(result).toHaveAttribute("data-outcome", "streaming");
    await expect(result).toContainText("streaming");
  });

  // The failure that motivated the probe: the name resolves and the format
  // does not. Reporting only "it failed" would send nobody anywhere.
  test("a device that will not open shows the reason", async ({ page }) => {
    await page.locator("#audio-device").fill("alsa:plughw:CARD=WING");
    await page.locator("#audio-test").click();
    const result = page.locator("#audio-test-result");
    await expect(result).toHaveAttribute("data-outcome", "could_not_open");
    await expect(result).toContainText("Unsupported bit depth");
  });

  // The device the player already holds would refuse a second open. Reporting
  // it as broken would be the worst answer available.
  test("the device already in use is reported, not reopened", async ({
    page,
  }) => {
    await page.locator("#audio-device").fill("Default Audio Device");
    await page.locator("#audio-test").click();
    const result = page.locator("#audio-test-result");
    await expect(result).toHaveAttribute("data-outcome", "in_use");
    await expect(result).toContainText("playing through right now");
  });

  // A result describes the settings it was run with. Left standing after an
  // edit it becomes a claim about something nobody asked for.
  test("changing a setting clears a stale test result", async ({ page }) => {
    await page.locator("#audio-test").click();
    await expect(page.locator("#audio-test-result")).toBeVisible();

    await page.locator("#audio-sample-rate").selectOption("48000");
    await expect(page.locator("#audio-test-result")).toHaveCount(0);
  });

  // Restoring the settings a result was produced for brings it back, which is
  // what proves the result is tied to the settings rather than merely wiped by
  // the change handler. An implementation that clears on edit passes the test
  // above and fails this one.
  test("a test result returns when its settings are restored", async ({
    page,
  }) => {
    await page.locator("#audio-test").click();
    await expect(page.locator("#audio-test-result")).toBeVisible();

    await page.locator("#audio-sample-rate").selectOption("48000");
    await expect(page.locator("#audio-test-result")).toHaveCount(0);

    await page.locator("#audio-sample-rate").selectOption("");
    await expect(page.locator("#audio-test-result")).toHaveAttribute(
      "data-outcome",
      "streaming",
    );
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
