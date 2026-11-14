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
import {
  ConfigService,
  LightingService,
} from "../../src/gen/player/v1/player_pb";
import { mockRpc } from "../support/rpc";

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
    const rpc = await mockRpc(page);
    rpc.unary(LightingService.method.deleteLightShow, {});

    await page.locator(".light-file-remove").click();
    const dialog = page.locator('[role="dialog"]');
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Confirm" }).click();

    // File should no longer be listed
    await expect(page.locator(".light-file-entry")).not.toBeVisible();
    // DELETE should NOT have been called yet (deferred until save)
    expect(rpc.calls(LightingService.method.deleteLightShow)).toHaveLength(0);
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
    const rpc = await mockRpc(page);
    rpc.unary(LightingService.method.saveLightShow, {});

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
    expect(rpc.calls(LightingService.method.saveLightShow)).toHaveLength(0);
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
    const rpc = await mockRpc(page);
    rpc.unary(ConfigService.method.saveSongConfig, {});
    const savedYaml = () =>
      rpc.calls(ConfigService.method.saveSongConfig).at(-1)?.yaml ?? "";
    rpc.unary(LightingService.method.saveLightShow, {});

    await page.getByRole("button", { name: /\+ DSL/i }).click();
    const dialog = page.locator('[role="dialog"]');
    await dialog.locator(".dialog-input").fill("extra_show");
    await dialog.getByRole("button", { name: "Confirm" }).click();

    // Click save
    await page.getByRole("button", { name: "Save" }).click();

    // Wait for save to complete
    await expect(page.locator(".save-msg")).toBeVisible();

    // New file should have been PUT to disk
    expect(
      rpc
        .calls(LightingService.method.saveLightShow)
        .some((c) => c.name.includes("extra_show.light")),
    ).toBe(true);
    // Saved YAML should contain both lighting entries
    expect(savedYaml()).toContain("lighting:");
    expect(savedYaml()).toContain("show.light");
    expect(savedYaml()).toContain("extra_show.light");
  });

  test("save deletes removed files from disk and cleans up YAML", async ({
    page,
  }) => {
    // Set up intercepts before performing actions
    const rpc = await mockRpc(page);
    rpc.unary(ConfigService.method.saveSongConfig, {});
    const savedYaml = () =>
      rpc.calls(ConfigService.method.saveSongConfig).at(-1)?.yaml ?? "";
    rpc.unary(LightingService.method.deleteLightShow, {});

    // Remove the file (deferred — no API call yet)
    await page.locator(".light-file-remove").click();
    const dialog = page.locator('[role="dialog"]');
    await dialog.getByRole("button", { name: "Confirm" }).click();

    // Click save
    await page.getByRole("button", { name: "Save" }).click();

    // Wait for save to complete
    await expect(page.locator(".save-msg")).toBeVisible();

    // DELETE should have been called during save
    expect(
      rpc
        .calls(LightingService.method.deleteLightShow)
        .some((c) => c.name.includes("show.light")),
    ).toBe(true);
    // Saved YAML should NOT contain lighting key
    expect(savedYaml()).not.toContain("lighting:");
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
    // The page is locked until the player's first status arrives, and a
    // locked page opens no dialog.
    await expect(page.locator(".topnav__lock")).not.toHaveClass(
      /topnav__lock--locked/,
    );

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
