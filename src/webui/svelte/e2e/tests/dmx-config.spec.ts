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

test.describe("DMX Configuration", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await page.locator(".tab", { hasText: "Lighting" }).click();
    await expect(page.locator(".tab.active")).toContainText("Lighting");
    await page.getByRole("button", { name: "Enable Lighting" }).click();
  });

  test("shows OLA port input", async ({ page }) => {
    await expect(page.locator("#dmx-ola-port")).toBeVisible();
  });

  test("shows null client checkbox", async ({ page }) => {
    await expect(page.locator("#dmx-null-client")).toBeVisible();
  });

  test("null client checkbox is toggleable", async ({ page }) => {
    const checkbox = page.locator("#dmx-null-client");
    const initialState = await checkbox.isChecked();
    await checkbox.click();
    expect(await checkbox.isChecked()).toBe(!initialState);
  });

  test("shows dim speed modifier input", async ({ page }) => {
    await expect(page.locator("#dmx-dim-speed")).toBeVisible();
  });

  test("adding universe creates a row", async ({ page }) => {
    await page.getByRole("button", { name: "Add" }).first().click();
    await expect(page.locator(".universe-row")).toBeVisible();
  });

  test("universe row has number and name inputs", async ({ page }) => {
    await page.getByRole("button", { name: "Add" }).first().click();
    const row = page.locator(".universe-row").first();
    await expect(row.locator(".universe-num")).toBeVisible();
    await expect(row.locator(".universe-name")).toBeVisible();
  });

  test("removing universe removes the row", async ({ page }) => {
    // Add two universes.
    await page.getByRole("button", { name: "Add" }).first().click();
    await page.getByRole("button", { name: "Add" }).first().click();
    await expect(page.locator(".universe-row")).toHaveCount(2);

    // Remove one.
    await page.locator(".universe-row").first().locator(".btn-danger").click();
    await expect(page.locator(".universe-row")).toHaveCount(1);
  });

  test("OLA port accepts numeric input", async ({ page }) => {
    const input = page.locator("#dmx-ola-port");
    await input.fill("9090");
    await expect(input).toHaveValue("9090");
  });
});
