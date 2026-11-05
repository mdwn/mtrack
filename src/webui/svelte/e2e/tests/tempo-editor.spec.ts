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

test.describe("Tempo Editor", () => {
  test("lighting tab loads for song with beat grid", async ({ page }) => {
    // Test Song Beta has a beat grid and sections.
    await page.goto("/#/songs/Test%20Song%20Beta/lighting");
    await expect(page.locator(".tab.active")).toContainText("Lighting");
    await expect(page.locator(".lighting-section")).toBeVisible();
  });

  test("lighting tab loads for song without beat grid", async ({ page }) => {
    // Test Song Alpha has no beat grid.
    await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
    await expect(page.locator(".tab.active")).toContainText("Lighting");
    await expect(page.locator(".lighting-section")).toBeVisible();
    // Tempo info badge should not show specific BPM info.
    await expect(page.locator(".tempo-info")).not.toBeVisible();
  });
});

test.describe("Tempo start unit", () => {
  test("switching the unit converts the start, which stays where it was", async ({
    page,
  }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
    await expect(page.locator(".lighting-section")).toBeVisible();
    const raw = page.locator(".tab-btn", { hasText: "Raw DSL" });
    const timeline = page.locator(".tab-btn", { hasText: "Timeline" });
    const textarea = page.locator(".raw-textarea");
    const source =
      'tempo {\n    start: 500ms\n    bpm: 120\n}\nshow "S" {\n    @1/1\n    a: static, color: "red", duration: 1s\n}\n';
    await raw.click();
    await textarea.fill(source);
    await timeline.click();
    await page.locator(".tempo-lane-clickable").click();
    const unit = page.locator(".start-row select");
    const value = page.locator(".start-row input");

    // The file as the editor would save it.
    const startOf = async () => {
      await raw.click();
      const text = await textarea.inputValue();
      await timeline.click();
      // The timeline comes back with its tempo editor closed.
      await page.locator(".tempo-lane-clickable").click();
      return text;
    };
    await unit.selectOption("s");
    await expect(value).toHaveValue("0.5");
    expect(await startOf()).toContain("start: 0.5s\n");
    await unit.selectOption("ms");
    await expect(value).toHaveValue("500");
    // Back in the unit it was written in: the text as it was.
    expect(await startOf()).toContain("start: 500ms\n");
  });
});
