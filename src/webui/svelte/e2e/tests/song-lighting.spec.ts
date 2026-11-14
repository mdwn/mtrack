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
import { LightingService } from "../../src/gen/player/v1/player_pb";
import { mockRpc } from "../support/rpc";

test.describe("Song Detail - Lighting Tab", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
    await expect(page.locator(".tab.active")).toContainText("Lighting");
  });

  test("shows lighting section", async ({ page }) => {
    await expect(page.locator(".lighting-section")).toBeVisible();
  });

  test("shows tab buttons for Timeline and Raw DSL", async ({ page }) => {
    await expect(
      page.locator(".tab-btn", { hasText: "Timeline" }),
    ).toBeVisible();
    await expect(
      page.locator(".tab-btn", { hasText: "Raw DSL" }),
    ).toBeVisible();
  });

  test("shows + DSL button", async ({ page }) => {
    await expect(page.getByRole("button", { name: /\+ DSL/i })).toBeVisible();
  });

  test("shows MIDI DMX button", async ({ page }) => {
    await expect(page.getByRole("button", { name: /midi dmx/i })).toBeVisible();
  });

  test("switching to Raw DSL tab shows textarea", async ({ page }) => {
    await page.locator(".tab-btn", { hasText: "Raw DSL" }).click();
    // Should show the raw DSL editor textarea
    await expect(page.locator(".raw-textarea")).toBeVisible();
  });

  test("Raw DSL tab shows Validate button", async ({ page }) => {
    await page.locator(".tab-btn", { hasText: "Raw DSL" }).click();
    await expect(page.getByRole("button", { name: "Validate" })).toBeVisible();
  });

  test("Validate button calls validation API", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.unary(LightingService.method.validateLightShow, { valid: true });

    await page.locator(".tab-btn", { hasText: "Raw DSL" }).click();
    await page.getByRole("button", { name: "Validate" }).click();
    await expect
      .poll(() => rpc.calls(LightingService.method.validateLightShow).length)
      .toBe(1);
  });

  test("successful validation shows OK message", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.unary(LightingService.method.validateLightShow, { valid: true });

    await page.locator(".tab-btn", { hasText: "Raw DSL" }).click();
    await page.getByRole("button", { name: "Validate" }).click();
    await expect(page.locator(".validation-ok")).toBeVisible();
  });

  test("failed validation shows errors", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.unary(LightingService.method.validateLightShow, {
      valid: false,
      errors: ["Unknown effect at line 3"],
    });

    await page.locator(".tab-btn", { hasText: "Raw DSL" }).click();
    await page.getByRole("button", { name: "Validate" }).click();
    await expect(page.locator(".validation-errors")).toBeVisible();
    await expect(page.locator(".validation-error")).toContainText(
      "Unknown effect",
    );
  });

  test("MIDI DMX button opens modal", async ({ page }) => {
    await page.getByRole("button", { name: /midi dmx/i }).click();
    await expect(page.locator(".modal-overlay")).toBeVisible();
    await expect(page.locator(".modal")).toBeVisible();
  });

  test("MIDI DMX modal has Close button", async ({ page }) => {
    await page.getByRole("button", { name: /midi dmx/i }).click();
    await expect(
      page.locator(".modal").getByRole("button", { name: "Close" }),
    ).toBeVisible();
  });

  test("Close button closes MIDI DMX modal", async ({ page }) => {
    await page.getByRole("button", { name: /midi dmx/i }).click();
    await page.locator(".modal").getByRole("button", { name: "Close" }).click();
    await expect(page.locator(".modal-overlay")).not.toBeVisible();
  });
});
