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

test.describe("Song Detail", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
  });

  test("shows back link", async ({ page }) => {
    await expect(page.locator(".back-link")).toBeVisible();
    await expect(page.locator(".back-link")).toContainText("All Songs");
  });

  test("shows song name", async ({ page }) => {
    await expect(page.locator(".song-title, h2")).toContainText(
      "Test Song Alpha",
    );
  });

  test("shows tab bar with all tabs", async ({ page }) => {
    await expect(page.locator(".tab-bar")).toBeVisible();
    await expect(page.locator(".tab", { hasText: "Tracks" })).toBeVisible();
    await expect(page.locator(".tab", { hasText: "MIDI" })).toBeVisible();
    await expect(page.locator(".tab", { hasText: "Samples" })).toBeVisible();
    await expect(page.locator(".tab", { hasText: "Lighting" })).toBeVisible();
    await expect(page.locator(".tab", { hasText: "Config" })).toBeVisible();
  });

  test("tracks tab is active by default", async ({ page }) => {
    await expect(page.locator(".tab.active")).toContainText("Tracks");
  });

  test("clicking MIDI tab changes active tab", async ({ page }) => {
    await page.locator(".tab", { hasText: "MIDI" }).click();
    await expect(page.locator(".tab.active")).toContainText("MIDI");
    await expect(page).toHaveURL(/.*#\/songs\/Test%20Song%20Alpha\/midi/);
  });

  test("clicking Lighting tab changes active tab", async ({ page }) => {
    await page.locator(".tab", { hasText: "Lighting" }).click();
    await expect(page.locator(".tab.active")).toContainText("Lighting");
    await expect(page).toHaveURL(/.*#\/songs\/Test%20Song%20Alpha\/lighting/);
  });

  test("clicking Config tab shows config editor", async ({ page }) => {
    await page.locator(".tab", { hasText: "Config" }).click();
    await expect(page.locator(".tab.active")).toContainText("Config");
    await expect(page.locator(".config-editor")).toBeVisible();
  });

  test("config tab shows YAML content", async ({ page }) => {
    await page.locator(".tab", { hasText: "Config" }).click();
    const editor = page.locator(".config-editor");
    await expect(editor).toBeVisible();
    const value = await editor.inputValue();
    expect(value).toContain("name:");
  });

  test("back link navigates to song list", async ({ page }) => {
    await page.locator(".back-link").click();
    await expect(page).toHaveURL(/.*#\/songs$/);
  });

  test("shows song metadata", async ({ page }) => {
    // Should show duration and track info somewhere in the detail header
    await expect(page.getByText("3:00")).toBeVisible();
  });

  test("shows MIDI badge for song with MIDI", async ({ page }) => {
    await expect(page.locator(".badge.midi")).toBeVisible();
  });

  test("shows lighting badge for song with lighting", async ({ page }) => {
    await expect(page.locator(".badge.lighting, .badge.light")).toBeVisible();
  });

  test("clicking Samples tab shows samples content", async ({ page }) => {
    await page.locator(".tab", { hasText: "Samples" }).click();
    await expect(page.locator(".tab.active")).toContainText("Samples");
    await expect(page).toHaveURL(/.*#\/songs\/Test%20Song%20Alpha\/samples/);
  });

  test("Samples tab shows empty state", async ({ page }) => {
    await page.locator(".tab", { hasText: "Samples" }).click();
    await expect(page.getByText(/per-song sample overrides/i)).toBeVisible();
  });

  test("Samples tab shows Add Sample button", async ({ page }) => {
    await page.locator(".tab", { hasText: "Samples" }).click();
    await expect(
      page.getByRole("button", { name: "Add Sample" }),
    ).toBeVisible();
  });
});

test.describe("Song Detail - Loop Playback", () => {
  test("does not show LOOP badge for non-looping song", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await expect(page.locator(".badge.loop")).not.toBeVisible();
  });

  test("shows LOOP badge for looping song", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Beta");
    await expect(page.locator(".badge.loop")).toBeVisible();
  });

  test("loop checkbox is unchecked for non-looping song", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    const checkbox = page.locator("#loop-playback");
    await expect(checkbox).toBeVisible();
    await expect(checkbox).not.toBeChecked();
  });

  test("loop checkbox is checked for looping song", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Beta");
    const checkbox = page.locator("#loop-playback");
    await expect(checkbox).toBeVisible();
    await expect(checkbox).toBeChecked();
  });

  test("toggling loop checkbox marks config as dirty", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await page.locator("#loop-playback").check();
    await expect(page.locator(".unsaved")).toBeVisible();
  });
});

test.describe("Song Detail - MIDI Event Editor", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    await page.locator(".tab", { hasText: "MIDI" }).click();
  });

  test("shows Song Select Event section", async ({ page }) => {
    await expect(page.getByText(/Song Select Event/i)).toBeVisible();
  });

  test("shows Add Event button when no event configured", async ({ page }) => {
    await expect(page.getByRole("button", { name: "Add Event" })).toBeVisible();
  });

  test("adding event shows type dropdown and fields", async ({ page }) => {
    await page.getByRole("button", { name: "Add Event" }).click();
    await expect(page.locator("#midi-event-type")).toBeVisible();
    await expect(page.locator("#midi-event-channel")).toBeVisible();
    // Default type is program_change, so program field should be visible
    await expect(page.locator("#midi-event-program")).toBeVisible();
  });

  test("changing type to note_on shows key and velocity fields", async ({
    page,
  }) => {
    await page.getByRole("button", { name: "Add Event" }).click();
    await page.locator("#midi-event-type").selectOption("note_on");
    await expect(page.locator("#midi-event-key")).toBeVisible();
    await expect(page.locator("#midi-event-velocity")).toBeVisible();
  });

  test("changing type to control_change shows controller and value fields", async ({
    page,
  }) => {
    await page.getByRole("button", { name: "Add Event" }).click();
    await page.locator("#midi-event-type").selectOption("control_change");
    await expect(page.locator("#midi-event-controller")).toBeVisible();
    await expect(page.locator("#midi-event-value")).toBeVisible();
  });

  test("Remove button clears the event", async ({ page }) => {
    await page.getByRole("button", { name: "Add Event" }).click();
    await expect(page.locator("#midi-event-type")).toBeVisible();
    await page.getByRole("button", { name: "Remove", exact: true }).click();
    await expect(page.locator("#midi-event-type")).not.toBeVisible();
    await expect(page.getByRole("button", { name: "Add Event" })).toBeVisible();
  });

  test("adding event marks config as dirty", async ({ page }) => {
    await page.getByRole("button", { name: "Add Event" }).click();
    await expect(page.locator(".unsaved")).toBeVisible();
  });
});
