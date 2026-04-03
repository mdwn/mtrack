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

test.describe("Song Lighting Editor - File Management", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
    await expect(page.locator(".tab.active")).toContainText("Lighting");
    await expect(page.locator(".lighting-section")).toBeVisible();
  });

  test("shows light file list with existing files", async ({ page }) => {
    await expect(page.locator(".light-file-entry")).toBeVisible();
    await expect(page.locator(".light-file-name")).toContainText("show.light");
  });

  test("each light file has a remove button", async ({ page }) => {
    await expect(page.locator(".light-file-remove")).toBeVisible();
  });

  test("remove button shows confirmation dialog", async ({ page }) => {
    await page.locator(".light-file-remove").click();
    const dialog = page.locator('[role="dialog"]');
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText("Remove");
  });

  test("cancelling remove keeps file in list", async ({ page }) => {
    await page.locator(".light-file-remove").click();
    const dialog = page.locator('[role="dialog"]');
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Cancel" }).click();

    // File should still be listed
    await expect(page.locator(".light-file-entry")).toBeVisible();
  });

  test("confirming remove removes file from list without calling DELETE", async ({
    page,
  }) => {
    let deleteCalled = false;
    await page.route("**/api/lighting/show.light", async (route) => {
      if (route.request().method() === "DELETE") {
        deleteCalled = true;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "deleted" }),
        });
      } else {
        await route.continue();
      }
    });

    await page.locator(".light-file-remove").click();
    const dialog = page.locator('[role="dialog"]');
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Confirm" }).click();

    // File should no longer be listed
    await expect(page.locator(".light-file-entry")).not.toBeVisible();
    // DELETE should NOT have been called yet (deferred until save)
    expect(deleteCalled).toBe(false);
  });

  test("removing file marks editor as dirty", async ({ page }) => {
    // Before removal — should not show unsaved indicator
    await expect(page.locator(".unsaved")).not.toBeVisible();

    await page.locator(".light-file-remove").click();
    const dialog = page.locator('[role="dialog"]');
    await dialog.getByRole("button", { name: "Confirm" }).click();

    // After removal — should show unsaved indicator
    await expect(page.locator(".unsaved")).toBeVisible();
  });

  test("add DSL button opens prompt and adds file without saving to disk", async ({
    page,
  }) => {
    let putCalled = false;
    await page.route("**/api/lighting/*", async (route) => {
      if (route.request().method() === "PUT") {
        putCalled = true;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "saved" }),
        });
      } else {
        await route.continue();
      }
    });

    await page.getByRole("button", { name: /\+ DSL/i }).click();
    // Should show prompt dialog
    const dialog = page.locator('[role="dialog"]');
    await expect(dialog).toBeVisible();

    // Enter a filename
    await dialog.locator(".dialog-input").fill("verse_lights");
    await dialog.getByRole("button", { name: "Confirm" }).click();

    // New file should appear in the list
    await expect(
      page.locator(".light-file-name", { hasText: "verse_lights.light" }),
    ).toBeVisible();
    // PUT should NOT have been called yet (deferred until save)
    expect(putCalled).toBe(false);
  });

  test("adding new file marks editor as dirty", async ({ page }) => {
    await page.getByRole("button", { name: /\+ DSL/i }).click();
    const dialog = page.locator('[role="dialog"]');
    await dialog.locator(".dialog-input").fill("new_show");
    await dialog.getByRole("button", { name: "Confirm" }).click();

    await expect(page.locator(".unsaved")).toBeVisible();
  });
});

test.describe("Song Lighting Editor - Save Integration", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
    await expect(page.locator(".lighting-section")).toBeVisible();
  });

  test("save writes new files to disk and updates YAML", async ({ page }) => {
    // Set up intercepts before performing actions
    let savedYaml = "";
    const putPaths: string[] = [];
    await page.route("**/api/songs/Test%20Song%20Alpha", async (route) => {
      if (route.request().method() === "PUT") {
        savedYaml = route.request().postData() ?? "";
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "updated" }),
        });
      } else {
        await route.continue();
      }
    });
    await page.route("**/api/lighting/**", async (route) => {
      if (route.request().method() === "PUT") {
        putPaths.push(new URL(route.request().url()).pathname);
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "saved" }),
        });
      } else {
        await route.continue();
      }
    });

    await page.getByRole("button", { name: /\+ DSL/i }).click();
    const dialog = page.locator('[role="dialog"]');
    await dialog.locator(".dialog-input").fill("extra_show");
    await dialog.getByRole("button", { name: "Confirm" }).click();

    // Click save
    await page.getByRole("button", { name: "Save" }).click();

    // Wait for save to complete
    await expect(page.locator(".save-msg")).toBeVisible();

    // New file should have been PUT to disk
    expect(putPaths.some((p) => p.includes("extra_show.light"))).toBe(true);
    // Saved YAML should contain both lighting entries
    expect(savedYaml).toContain("lighting:");
    expect(savedYaml).toContain("show.light");
    expect(savedYaml).toContain("extra_show.light");
  });

  test("save deletes removed files from disk and cleans up YAML", async ({
    page,
  }) => {
    // Set up intercepts before performing actions
    let savedYaml = "";
    let deleteCalled = false;
    await page.route("**/api/songs/Test%20Song%20Alpha", async (route) => {
      if (route.request().method() === "PUT") {
        savedYaml = route.request().postData() ?? "";
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "updated" }),
        });
      } else {
        await route.continue();
      }
    });
    await page.route("**/api/lighting/show.light", async (route) => {
      if (route.request().method() === "DELETE") {
        deleteCalled = true;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "deleted" }),
        });
      } else {
        await route.continue();
      }
    });

    // Remove the file (deferred — no API call yet)
    await page.locator(".light-file-remove").click();
    const dialog = page.locator('[role="dialog"]');
    await dialog.getByRole("button", { name: "Confirm" }).click();

    // Click save
    await page.getByRole("button", { name: "Save" }).click();

    // Wait for save to complete
    await expect(page.locator(".save-msg")).toBeVisible();

    // DELETE should have been called during save
    expect(deleteCalled).toBe(true);
    // Saved YAML should NOT contain lighting key
    expect(savedYaml).not.toContain("lighting:");
  });
});

test.describe("Song Lighting Editor - Implicit file creation", () => {
  test("song without lighting files gets implicit file with show lanes", async ({
    page,
  }) => {
    await page.goto("/#/songs/Test%20Song%20Beta/lighting");
    await expect(page.locator(".lighting-section")).toBeVisible();
    // Implicit file should appear in file list
    await expect(page.locator(".light-file-entry")).toBeVisible();
    // Show lanes should be visible (implicit "Main" show)
    await expect(
      page.locator('[aria-label="Lighting timeline"]'),
    ).toBeVisible();
  });

  test("implicit file is not dirty until user edits", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Beta/lighting");
    await expect(page.locator(".lighting-section")).toBeVisible();
    // Should not show unsaved indicator just from loading
    await expect(page.locator(".unsaved")).not.toBeVisible();
  });

  test("can add additional light file to song with implicit file", async ({
    page,
  }) => {
    await page.goto("/#/songs/Test%20Song%20Beta/lighting");
    await expect(page.locator(".lighting-section")).toBeVisible();

    await page.getByRole("button", { name: /\+ DSL/i }).click();
    const dialog = page.locator('[role="dialog"]');
    await dialog.locator(".dialog-input").fill("first_show");
    await dialog.getByRole("button", { name: "Confirm" }).click();

    // Should now have two files (implicit + new)
    const entries = page.locator(".light-file-entry");
    await expect(entries).toHaveCount(2);
    await expect(page.locator(".unsaved")).toBeVisible();
  });
});
