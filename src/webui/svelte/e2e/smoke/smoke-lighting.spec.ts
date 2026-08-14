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
import { waitForWebSocket } from "./helpers.js";

test.describe("Smoke: Song Lighting Editor", () => {
  test("song with DSL lighting show shows lighting tab", async ({ page }) => {
    await page.goto(`/#/songs/${encodeURIComponent("A really cool song")}`);
    await waitForWebSocket(page);
    await expect(page.getByText("A really cool song")).toBeVisible({
      timeout: 10000,
    });

    // Click Lighting tab.
    const lightingTab = page.locator(".tab", { hasText: "Lighting" });
    await expect(lightingTab).toBeVisible();
    await lightingTab.click();

    // Lighting section should render.
    await expect(page.locator(".lighting-section")).toBeVisible({
      timeout: 10000,
    });

    // Should have the lighting file listed.
    await expect(page.locator(".light-file-entry").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("DSL light show song has multiple lighting files listed", async ({
    page,
  }) => {
    await page.goto(`/#/songs/${encodeURIComponent("DSL Light Show Song")}`);
    await waitForWebSocket(page);
    await expect(page.getByText("DSL Light Show Song")).toBeVisible({
      timeout: 10000,
    });

    // Click Lighting tab.
    const lightingTab = page.locator(".tab", { hasText: "Lighting" });
    await expect(lightingTab).toBeVisible();
    await lightingTab.click();

    // Should show the lighting section.
    await expect(page.locator(".lighting-section")).toBeVisible({
      timeout: 10000,
    });

    // Light file entries should be present (main_show.light, outro.light).
    await expect(page.locator(".light-file-entry").first()).toBeVisible({
      timeout: 10000,
    });
    const fileCount = await page.locator(".light-file-entry").count();
    expect(fileCount).toBeGreaterThanOrEqual(2);
  });

  test("timeline editor loads for song with lighting", async ({ page }) => {
    await page.goto(`/#/songs/${encodeURIComponent("DSL Light Show Song")}`);
    await waitForWebSocket(page);
    await expect(page.getByText("DSL Light Show Song")).toBeVisible({
      timeout: 10000,
    });

    // Navigate to lighting tab.
    await page.locator(".tab", { hasText: "Lighting" }).click();
    await expect(page.locator(".lighting-section")).toBeVisible({
      timeout: 10000,
    });

    // Timeline tab should be the default view.
    const timelineTab = page.locator(".tab-btn", { hasText: "Timeline" });
    await expect(timelineTab).toBeVisible();
  });

  test("Raw DSL tab shows editable text for light show", async ({ page }) => {
    await page.goto(`/#/songs/${encodeURIComponent("DSL Light Show Song")}`);
    await waitForWebSocket(page);
    await expect(page.getByText("DSL Light Show Song")).toBeVisible({
      timeout: 10000,
    });

    await page.locator(".tab", { hasText: "Lighting" }).click();
    await expect(page.locator(".lighting-section")).toBeVisible({
      timeout: 10000,
    });

    // Switch to Raw DSL tab.
    await page.locator(".tab-btn", { hasText: "Raw DSL" }).click();

    // Should show a textarea with DSL content.
    const textarea = page.locator(".raw-textarea");
    await expect(textarea).toBeVisible();

    // Should contain actual DSL content (not empty).
    const content = await textarea.inputValue();
    expect(content.length).toBeGreaterThan(0);
  });

  test("validate button works on Raw DSL", async ({ page }) => {
    await page.goto(`/#/songs/${encodeURIComponent("DSL Light Show Song")}`);
    await waitForWebSocket(page);
    await expect(page.getByText("DSL Light Show Song")).toBeVisible({
      timeout: 10000,
    });

    await page.locator(".tab", { hasText: "Lighting" }).click();
    await expect(page.locator(".lighting-section")).toBeVisible({
      timeout: 10000,
    });

    // Switch to Raw DSL.
    await page.locator(".tab-btn", { hasText: "Raw DSL" }).click();
    await expect(page.locator(".raw-textarea")).toBeVisible();

    // Click Validate.
    await page.getByRole("button", { name: "Validate" }).click();

    // Should show validation result (success or errors).
    await expect(
      page
        .locator(".validation-ok, .validation-errors, .validation-result")
        .first(),
    ).toBeVisible({ timeout: 10000 });
  });

  test("+ DSL button opens add dialog", async ({ page }) => {
    await page.goto(`/#/songs/${encodeURIComponent("DSL Light Show Song")}`);
    await waitForWebSocket(page);
    await expect(page.getByText("DSL Light Show Song")).toBeVisible({
      timeout: 10000,
    });

    await page.locator(".tab", { hasText: "Lighting" }).click();
    await expect(page.locator(".lighting-section")).toBeVisible({
      timeout: 10000,
    });

    // Click + DSL button.
    await page.getByRole("button", { name: /\+ DSL/i }).click();

    // Dialog should appear.
    await expect(page.locator('[role="dialog"]')).toBeVisible();

    // Cancel to clean up.
    await page
      .locator('[role="dialog"]')
      .getByRole("button", { name: "Cancel" })
      .click();
    await expect(page.locator('[role="dialog"]')).not.toBeVisible();
  });

  test("song without lighting shows lighting tab", async ({ page }) => {
    // "A really fast one" has no lighting config.
    await page.goto(`/#/songs/${encodeURIComponent("A really fast one")}`);
    await waitForWebSocket(page);
    await expect(page.getByText("A really fast one")).toBeVisible({
      timeout: 10000,
    });

    // Click Lighting tab.
    const lightingTab = page.locator(".tab", { hasText: "Lighting" });
    await expect(lightingTab).toBeVisible();
    await lightingTab.click();

    // Should show lighting section (with implicit file).
    await expect(page.locator(".lighting-section")).toBeVisible({
      timeout: 10000,
    });
    // Should NOT be marked as dirty.
    await expect(page.locator(".unsaved")).not.toBeVisible();
  });
});
