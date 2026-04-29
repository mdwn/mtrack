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

test.describe("Responsive Layout", () => {
  test("mobile viewport shows hamburger menu", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/#/");
    await expect(page.locator(".topnav__hamburger")).toBeVisible();
  });

  test("desktop viewport hides hamburger menu", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 720 });
    await page.goto("/#/");
    await expect(page.locator(".topnav__hamburger")).not.toBeVisible();
  });

  test("hamburger opens drawer on mobile", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/#/");

    const drawer = page.locator(".drawer");
    await expect(drawer).not.toHaveClass(/drawer--open/);

    await page.locator(".topnav__hamburger").click();
    await expect(drawer).toHaveClass(/drawer--open/);

    // Backdrop click closes the drawer. Click at the far right of the
    // backdrop, outside the 280px drawer panel.
    await page
      .locator(".drawer-backdrop")
      .click({ position: { x: 360, y: 300 } });
    await expect(drawer).not.toHaveClass(/drawer--open/);
  });

  test("mobile drawer link click navigates and closes drawer", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/#/");

    await page.locator(".topnav__hamburger").click();
    await page.locator('.drawer__item[href="#/songs"]').click();

    await expect(page).toHaveURL(/.*#\/songs/);
    await expect(page.locator(".drawer")).not.toHaveClass(/drawer--open/);
  });

  test("dashboard playback card renders on mobile", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/#/");
    await expect(page.locator(".playback-card")).toBeVisible();
  });

  test("playlist editor stacks panels on mobile", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/#/playlists");
    await expect(page.locator(".list-panel")).toBeVisible();
  });
});
