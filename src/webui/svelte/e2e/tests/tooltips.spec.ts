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

test.describe("Tooltips", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
  });

  test("tooltip icon is visible next to fields", async ({ page }) => {
    // Audio tab is active by default — check that tooltip icons appear
    const tooltipIcons = page.locator(".tooltip-icon");
    await expect(tooltipIcons.first()).toBeVisible();
    // Audio section should have multiple tooltip icons
    expect(await tooltipIcons.count()).toBeGreaterThanOrEqual(4);
  });

  test("hovering tooltip icon shows popover", async ({ page }) => {
    const icon = page.locator(".tooltip-icon").first();
    await icon.hover();
    const popover = page.locator('[role="tooltip"]');
    await expect(popover).toBeVisible();
    // Popover should have non-empty text
    const text = await popover.textContent();
    expect(text?.trim().length).toBeGreaterThan(10);
  });

  test("popover hides when mouse leaves", async ({ page }) => {
    const icon = page.locator(".tooltip-icon").first();
    await icon.hover();
    await expect(page.locator('[role="tooltip"]')).toBeVisible();
    // Move mouse away
    await page.mouse.move(0, 0);
    await expect(page.locator('[role="tooltip"]')).not.toBeVisible();
  });

  test("clicking tooltip icon toggles popover", async ({ page }) => {
    // Dispatch a touch pointerdown to simulate real touch interaction,
    // then verify click-to-toggle works without hover interference.
    const icon = page.locator(".tooltip-icon").first();
    await icon.dispatchEvent("pointerdown", { pointerType: "touch" });
    await icon.click();
    await expect(page.locator('[role="tooltip"]')).toBeVisible();
    // Second click should close
    await icon.dispatchEvent("pointerdown", { pointerType: "touch" });
    await icon.click();
    await expect(page.locator('[role="tooltip"]')).not.toBeVisible();
  });

  test("clicking outside closes popover", async ({ page }) => {
    const icon = page.locator(".tooltip-icon").first();
    await icon.dispatchEvent("pointerdown", { pointerType: "touch" });
    await icon.click();
    await expect(page.locator('[role="tooltip"]')).toBeVisible();
    // Click elsewhere (pointerdown outside wrapEl triggers close)
    await page.locator(".detail-toolbar").click();
    await expect(page.locator('[role="tooltip"]')).not.toBeVisible();
  });

  test("popover is not clipped by overflow containers", async ({ page }) => {
    const icon = page.locator(".tooltip-icon").first();
    await icon.hover();
    const popover = page.locator('[role="tooltip"]');
    await expect(popover).toBeVisible();
    // Popover should be fully within viewport
    const box = await popover.boundingBox();
    expect(box).not.toBeNull();
    if (box) {
      const viewport = page.viewportSize()!;
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.y).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(viewport.width);
      expect(box.y + box.height).toBeLessThanOrEqual(viewport.height);
    }
  });

  test("audio section has tooltips on expected fields", async ({ page }) => {
    // Buffer Size should have a tooltip
    const bufferLabel = page.locator('label[for="audio-buffer-size"]');
    const icon = bufferLabel.locator(".tooltip-icon");
    await expect(icon).toBeVisible();
    await icon.hover();
    const popover = page.locator('[role="tooltip"]');
    await expect(popover).toContainText("audio frames per hardware callback");
  });

  test("midi section has tooltips", async ({ page }) => {
    await page.locator(".tab", { hasText: "MIDI" }).click();
    // Need to enable MIDI first if not enabled
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    if (!(await checkbox.isChecked())) {
      await checkbox.check();
    }
    const tooltipIcons = page.locator(".tooltip-icon");
    await expect(tooltipIcons.first()).toBeVisible();
  });

  test("dmx section has tooltips", async ({ page }) => {
    await page.locator(".tab", { hasText: "Lighting" }).click();
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    if (!(await checkbox.isChecked())) {
      await checkbox.check();
    }
    const tooltipIcons = page.locator(".tooltip-icon");
    await expect(tooltipIcons.first()).toBeVisible();
  });

  test("trigger section has tooltips", async ({ page }) => {
    await page.locator(".tab", { hasText: "Triggers" }).click();
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    if (!(await checkbox.isChecked())) {
      await checkbox.check();
    }
    const tooltipIcons = page.locator(".tooltip-icon");
    await expect(tooltipIcons.first()).toBeVisible();
  });

  test("controllers section has tooltips", async ({ page }) => {
    await page.locator(".tab", { hasText: "Controllers" }).click();
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    if (!(await checkbox.isChecked())) {
      await checkbox.check();
    }
    const tooltipIcons = page.locator(".tooltip-icon");
    await expect(tooltipIcons.first()).toBeVisible();
  });
});
