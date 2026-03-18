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

test.describe("File Browser", () => {
  // The file browser appears as a modal when browsing for files.
  // We can trigger it from the song browser's "Import from Filesystem" button.
  test.beforeEach(async ({ page }) => {
    // Mock browse API to return test entries
    await page.route("**/api/browse**", async (route) => {
      const url = new URL(route.request().url());
      const path = url.searchParams.get("path") || "/songs";
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          path,
          root: "/songs",
          entries: [
            {
              name: "subfolder",
              path: `${path}/subfolder`,
              type: "directory",
              is_dir: true,
            },
            {
              name: "track1.wav",
              path: `${path}/track1.wav`,
              type: "audio",
              is_dir: false,
            },
            {
              name: "track2.flac",
              path: `${path}/track2.flac`,
              type: "audio",
              is_dir: false,
            },
            {
              name: "show.light",
              path: `${path}/show.light`,
              type: "lighting",
              is_dir: false,
            },
          ],
        }),
      });
    });

    await page.goto("/#/songs");
    // Click "Import from Filesystem" to open the file browser
    await page.getByRole("button", { name: /import from filesystem/i }).click();
    await expect(page.locator(".browser")).toBeVisible();
  });

  test("shows browser header with path input", async ({ page }) => {
    await expect(page.locator(".path-input")).toBeVisible();
  });

  test("shows Go button", async ({ page }) => {
    await expect(page.getByRole("button", { name: "Go" })).toBeVisible();
  });

  test("shows breadcrumb navigation", async ({ page }) => {
    await expect(page.locator(".breadcrumbs")).toBeVisible();
  });

  test("shows directory entries", async ({ page }) => {
    await expect(page.locator(".entry.dir")).toBeVisible();
    await expect(page.locator(".entry.dir")).toContainText("subfolder");
  });

  test("shows file entries", async ({ page }) => {
    // Audio files should be visible (browser defaults to showing all types)
    await expect(
      page.locator(".entry", { hasText: "track1.wav" }),
    ).toBeVisible();
  });

  test("shows Cancel button", async ({ page }) => {
    await expect(page.getByRole("button", { name: "Cancel" })).toBeVisible();
  });

  test("Cancel closes the browser", async ({ page }) => {
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.locator(".browser")).not.toBeVisible();
  });

  test("clicking a directory navigates into it", async ({ page }) => {
    await page.locator(".entry.dir", { hasText: "subfolder" }).click();
    // Path input should update
    await expect(page.locator(".path-input")).toHaveValue(/subfolder/);
  });

  test("footer shows file count", async ({ page }) => {
    await expect(page.locator(".footer-info")).toBeVisible();
  });
});
