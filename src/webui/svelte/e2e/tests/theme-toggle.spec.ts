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

test.describe("Theme toggle", () => {
  test("topnav exposes a theme toggle button", async ({ page }) => {
    await page.goto("/#/");
    await expect(page.locator(".topnav__theme")).toBeVisible();
  });

  test("clicking cycles system → light → dark → system", async ({ page }) => {
    await page.goto("/#/");
    const btn = page.locator(".topnav__theme");
    const html = page.locator("html");

    // Default is "system" — clear localStorage just in case a prior test
    // persisted a choice.
    await page.evaluate(() => localStorage.removeItem("mtrack-theme"));
    await page.reload();
    await expect(btn).toBeVisible();

    // Click 1: → light. <html> must NOT have nc--dark.
    await btn.click();
    await expect(html).not.toHaveClass(/nc--dark/);
    expect(
      await page.evaluate(() => localStorage.getItem("mtrack-theme")),
    ).toBe("light");

    // Click 2: → dark. <html> gains nc--dark.
    await btn.click();
    await expect(html).toHaveClass(/nc--dark/);
    expect(
      await page.evaluate(() => localStorage.getItem("mtrack-theme")),
    ).toBe("dark");

    // Click 3: back to system — localStorage clears.
    await btn.click();
    expect(
      await page.evaluate(() => localStorage.getItem("mtrack-theme")),
    ).toBeNull();
  });

  test("explicit dark choice survives a reload", async ({ page }) => {
    await page.goto("/#/");
    await page.evaluate(() => {
      localStorage.setItem("mtrack-theme", "dark");
    });
    await page.reload();
    await expect(page.locator("html")).toHaveClass(/nc--dark/);
  });

  test("explicit light choice survives a reload", async ({ page }) => {
    await page.goto("/#/");
    await page.evaluate(() => {
      localStorage.setItem("mtrack-theme", "light");
    });
    await page.reload();
    await expect(page.locator("html")).not.toHaveClass(/nc--dark/);
  });
});
