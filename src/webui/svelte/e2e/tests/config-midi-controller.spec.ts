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

test.describe("MIDI Controller", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await page.locator(".tab", { hasText: "Controllers" }).click();
    await page.getByRole("button", { name: "Enable Controllers" }).click();
  });

  test("Add MIDI button is visible", async ({ page }) => {
    await expect(page.getByRole("button", { name: "Add MIDI" })).toBeVisible();
  });

  test("adding MIDI controller shows event mappings", async ({ page }) => {
    await page.getByRole("button", { name: "Add MIDI" }).click();
    await expect(page.locator(".controller-kind")).toContainText(/midi/i);
    // Should show all 6 required action labels
    await expect(page.getByText("Play", { exact: true })).toBeVisible();
    await expect(page.getByText("Previous Song")).toBeVisible();
    await expect(page.getByText("Next Song")).toBeVisible();
    await expect(page.getByText("Stop", { exact: true })).toBeVisible();
    await expect(page.getByText("All Songs", { exact: true })).toBeVisible();
    await expect(page.getByText("Playlist", { exact: true })).toBeVisible();
  });

  test("required actions have MIDI event editors", async ({ page }) => {
    await page.getByRole("button", { name: "Add MIDI" }).click();
    // Each required action should have a type dropdown
    await expect(page.locator('[id^="midi-"][id$="-play-type"]')).toBeVisible();
    await expect(page.locator('[id^="midi-"][id$="-prev-type"]')).toBeVisible();
    await expect(page.locator('[id^="midi-"][id$="-next-type"]')).toBeVisible();
    await expect(page.locator('[id^="midi-"][id$="-stop-type"]')).toBeVisible();
  });

  test("optional section_ack action is disabled by default", async ({
    page,
  }) => {
    await page.getByRole("button", { name: "Add MIDI" }).click();
    // section_ack checkbox should exist but not be checked
    const sectionAckAction = page.locator(".midi-action", {
      hasText: "Section Ack",
    });
    await expect(sectionAckAction).toBeVisible();
    const checkbox = sectionAckAction.locator('input[type="checkbox"]');
    await expect(checkbox).not.toBeChecked();
    // No type dropdown should be visible for unchecked optional action
    await expect(
      sectionAckAction.locator('[id$="-section_ack-type"]'),
    ).not.toBeVisible();
  });

  test("enabling optional section_ack shows event editor", async ({ page }) => {
    await page.getByRole("button", { name: "Add MIDI" }).click();
    const sectionAckAction = page.locator(".midi-action", {
      hasText: "Section Ack",
    });
    await sectionAckAction.locator('input[type="checkbox"]').check();
    await expect(
      sectionAckAction.locator('[id$="-section_ack-type"]'),
    ).toBeVisible();
  });

  test("changing event type updates visible fields", async ({ page }) => {
    await page.getByRole("button", { name: "Add MIDI" }).click();
    const playType = page.locator('[id^="midi-"][id$="-play-type"]');
    // Default is note_on, should show key and velocity
    await expect(page.locator('[id^="midi-"][id$="-play-key"]')).toBeVisible();
    await expect(
      page.locator('[id^="midi-"][id$="-play-velocity"]'),
    ).toBeVisible();

    // Switch to program_change
    await playType.selectOption("program_change");
    await expect(
      page.locator('[id^="midi-"][id$="-play-program"]'),
    ).toBeVisible();
    await expect(
      page.locator('[id^="midi-"][id$="-play-key"]'),
    ).not.toBeVisible();
  });

  test("Morningstar toggle still works", async ({ page }) => {
    await page.getByRole("button", { name: "Add MIDI" }).click();
    const morningstarCheckbox = page.getByLabel(/Morningstar/i);
    await expect(morningstarCheckbox).toBeVisible();
    await morningstarCheckbox.check();
    await expect(page.locator(".morningstar-fields")).toBeVisible();
  });

  test("removing MIDI controller removes card", async ({ page }) => {
    await page.getByRole("button", { name: "Add MIDI" }).click();
    await expect(page.locator(".controller-card")).toHaveCount(1);
    await page
      .locator(".controller-card")
      .getByRole("button", { name: "Remove" })
      .click();
    await expect(page.locator(".controller-card")).toHaveCount(0);
  });
});
