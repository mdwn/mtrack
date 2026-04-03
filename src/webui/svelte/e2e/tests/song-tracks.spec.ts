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

test.describe("Song Detail - Tracks Tab", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha");
    // Wait for data to load
    await expect(page.locator(".tab.active")).toContainText("Tracks");
  });

  test("shows track editor", async ({ page }) => {
    await expect(page.locator(".track-editor")).toBeVisible();
  });

  test("shows track rows from song config", async ({ page }) => {
    const rows = page.locator(".track-row");
    await expect(rows.first()).toBeVisible();
  });

  test("shows add track button", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /add track/i }),
    ).toBeVisible();
  });

  test("shows file upload drop zone", async ({ page }) => {
    await expect(page.locator(".drop-zone")).toBeVisible();
  });
});

test.describe("Song Detail - Config Tab", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/config");
    await expect(page.locator(".tab.active")).toContainText("Config");
  });

  test("shows YAML editor textarea", async ({ page }) => {
    const editor = page.locator(".config-editor");
    await expect(editor).toBeVisible();
  });

  test("YAML editor contains song config", async ({ page }) => {
    const editor = page.locator(".config-editor");
    const value = await editor.inputValue();
    expect(value).toContain("name:");
    expect(value).toContain("tracks:");
  });

  test("editing config makes save available", async ({ page }) => {
    const editor = page.locator(".config-editor");
    await editor.click();
    await editor.pressSequentially("\n# test comment");

    // Should show unsaved indicator
    await expect(page.getByText("Unsaved")).toBeVisible();
  });

  test("save button calls update API", async ({ page }) => {
    const editor = page.locator(".config-editor");
    await editor.click();
    await editor.pressSequentially("\n# test");

    const requestPromise = page.waitForRequest(
      (req) =>
        req.url().includes("/api/songs/Test%20Song%20Alpha") &&
        req.method() === "PUT",
    );
    await page.getByRole("button", { name: "Save" }).click();
    await requestPromise;
  });
});

test.describe("Song Detail - MIDI Section", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/tracks");
    await expect(page.locator(".tab.active")).toContainText("Tracks");
    // Expand the MIDI collapsible section
    await page.locator(".collapsible-header", { hasText: "MIDI" }).click();
  });

  test("shows MIDI section content", async ({ page }) => {
    await expect(page.locator(".collapsible-body")).toBeVisible();
  });

  test("shows file upload for MIDI files", async ({ page }) => {
    await expect(page.locator(".collapsible-body .drop-zone")).toBeVisible();
  });
});
