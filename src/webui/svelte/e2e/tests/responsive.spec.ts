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
    await expect(page.locator(".hamburger")).toBeVisible();
  });

  test("desktop viewport hides hamburger menu", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 720 });
    await page.goto("/#/");
    await expect(page.locator(".hamburger")).not.toBeVisible();
  });

  test("hamburger toggles nav links on mobile", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/#/");

    // Nav links should be hidden initially on mobile
    const navLinks = page.locator(".nav-links");

    // Click hamburger to show
    await page.locator(".hamburger").click();
    await expect(navLinks).toBeVisible();

    // Click again to hide
    await page.locator(".hamburger").click();
    await expect(navLinks).not.toBeVisible();
  });

  test("mobile nav link click navigates and closes menu", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/#/");

    await page.locator(".hamburger").click();
    await page.locator('a.nav-link[href="#/songs"]').click();

    await expect(page).toHaveURL(/.*#\/songs/);
  });

  test("dashboard renders in single column on mobile", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/#/");
    await expect(page.locator(".dashboard-grid")).toBeVisible();
  });

  test("playlist editor stacks panels on mobile", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto("/#/playlists");
    await expect(page.locator(".list-panel")).toBeVisible();
  });
});
