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

test.describe("MIDI to DMX Editor", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await page.locator(".tab", { hasText: "MIDI" }).click();
    await page.getByRole("button", { name: "Enable MIDI" }).click();
  });

  test("shows MIDI to DMX section with add button", async ({ page }) => {
    await expect(page.getByText("MIDI to DMX")).toBeVisible();
    const addBtn = page
      .locator(".midi-to-dmx-section")
      .getByRole("button", { name: "Add" });
    await expect(addBtn).toBeVisible();
  });

  test("adding mapping shows channel and universe fields", async ({ page }) => {
    await page
      .locator(".midi-to-dmx-section")
      .getByRole("button", { name: "Add" })
      .click();
    await expect(page.locator(".mapping-card")).toHaveCount(1);
    await expect(page.locator('[id^="mtd-ch-"]')).toBeVisible();
    await expect(page.locator('[id^="mtd-univ-"]')).toBeVisible();
  });

  test("removing mapping removes card", async ({ page }) => {
    await page
      .locator(".midi-to-dmx-section")
      .getByRole("button", { name: "Add" })
      .click();
    await expect(page.locator(".mapping-card")).toHaveCount(1);
    await page
      .locator(".mapping-card")
      .getByRole("button", { name: "Remove" })
      .click();
    await expect(page.locator(".mapping-card")).toHaveCount(0);
  });

  test("adding transformer shows type selector", async ({ page }) => {
    await page
      .locator(".midi-to-dmx-section")
      .getByRole("button", { name: "Add" })
      .click();
    await page
      .locator(".transformers-area")
      .getByRole("button", { name: "Add" })
      .click();
    await expect(page.locator(".transformer-row")).toHaveCount(1);
    await expect(page.locator(".transformer-type")).toBeVisible();
  });

  test("note mapper shows input note and output notes fields", async ({
    page,
  }) => {
    await page
      .locator(".midi-to-dmx-section")
      .getByRole("button", { name: "Add" })
      .click();
    await page
      .locator(".transformers-area")
      .getByRole("button", { name: "Add" })
      .click();
    // Default is note_mapper
    await expect(page.locator('[id$="-note"]')).toBeVisible();
    await expect(page.locator('[id$="-notes"]')).toBeVisible();
  });

  test("switching to CC mapper shows controller fields", async ({ page }) => {
    await page
      .locator(".midi-to-dmx-section")
      .getByRole("button", { name: "Add" })
      .click();
    await page
      .locator(".transformers-area")
      .getByRole("button", { name: "Add" })
      .click();
    await page
      .locator(".transformer-type")
      .selectOption("control_change_mapper");
    await expect(page.locator('[id$="-cc"]')).toBeVisible();
    await expect(page.locator('[id$="-ccs"]')).toBeVisible();
  });

  test("removing transformer removes row", async ({ page }) => {
    await page
      .locator(".midi-to-dmx-section")
      .getByRole("button", { name: "Add" })
      .click();
    await page
      .locator(".transformers-area")
      .getByRole("button", { name: "Add" })
      .click();
    await expect(page.locator(".transformer-row")).toHaveCount(1);
    await page.locator(".transformer-row .btn-danger").click();
    await expect(page.locator(".transformer-row")).toHaveCount(0);
  });
});
