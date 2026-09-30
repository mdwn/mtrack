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

import { test, expect, type Page } from "@playwright/test";

// The GDTF mode picker (lighting UI design, section 12.2). Each test routes
// its own inspection with `page.route`, so none depends on the mock server's
// default archive.

const INSPECTION = {
  fixture: 'Spiider "Pro" {v2}',
  manufacturer: "Robe",
  suggested_name: "Spiider Pro v2",
  fixture_types_dir: "lighting/fixture_types",
  modes: [
    {
      name: "RGB",
      channel_count: 3,
      footprint: 3,
      capabilities: ["color"],
      cells: 0,
      channels: [
        [1, "red"],
        [2, "green"],
        [3, "blue"],
      ],
      warnings: [],
    },
    {
      name: "RGB + strobe",
      channel_count: 5,
      footprint: 5,
      capabilities: ["color", "dimmer", "strobe"],
      cells: 0,
      strobe_range: { min_hz: 0.4, max_hz: 25 },
      channels: [
        [1, "dimmer"],
        [2, "red"],
        [3, "green"],
        [4, "blue"],
        [5, "strobe"],
      ],
      warnings: ["skipped virtual channel (no DMX offset): Shutter"],
    },
    {
      name: "Mover",
      channel_count: 6,
      footprint: 8,
      capabilities: ["pan_tilt", "dimmer", "color_wheel", "gobo"],
      cells: 0,
      channels: [
        [1, "pan"],
        [2, "pan_fine"],
        [3, "tilt"],
        [4, "tilt_fine"],
        [5, "dimmer"],
        [6, "color1"],
      ],
      warnings: [],
    },
    {
      name: "Pixel 12",
      channel_count: 36,
      footprint: 36,
      capabilities: ["color", "dimmer", "cells"],
      cells: 12,
      channels: [[1, "red"]],
      warnings: [],
    },
    {
      name: "Matrix",
      channel_count: 0,
      footprint: 90,
      refused: "another mode has this name; an import would take the first",
    },
  ],
};

async function open(page: Page, inspection: object = INSPECTION) {
  await page.route("**/api/lighting/gdtf/inspect", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(inspection),
    });
  });
  await page.goto("/#/lighting/fixtures");
  await page.locator('input[type="file"]').setInputFiles({
    name: "spiider.gdtf",
    mimeType: "application/octet-stream",
    buffer: Buffer.from("not inspected by the mock"),
  });
  await expect(page.getByTestId("gdtf-mode-picker")).toBeVisible();
}

const option = (page: Page, name: string) =>
  page.locator(`[role="option"][data-mode="${name}"]`);

test.describe("GDTF mode picker", () => {
  test("names the archive, its maker and its mode count", async ({ page }) => {
    await open(page);
    const picker = page.getByTestId("gdtf-mode-picker");
    await expect(picker).toContainText('Spiider "Pro" {v2}');
    await expect(picker).toContainText("Robe");
    await expect(page.getByTestId("gdtf-mode-count")).toHaveText("5 modes");
    // Name, addresses and cells on each row.
    await expect(option(page, "Pixel 12")).toContainText("36 addresses");
    await expect(option(page, "Pixel 12")).toContainText("12 cells");
    await expect(option(page, "RGB")).toContainText("3 addresses");
  });

  test("filters the list, and moves the choice off a hidden mode", async ({
    page,
  }) => {
    await open(page);
    await expect(option(page, "RGB")).toHaveAttribute("aria-selected", "true");
    await page.getByLabel("Filter modes").fill("pixel");
    await expect(page.getByRole("option")).toHaveCount(1);
    await expect(option(page, "Pixel 12")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(page.getByTestId("gdtf-can")).toContainText(
      "Per-pixel effects, 12 cells",
    );
    await page.getByLabel("Filter modes").fill("zzz");
    await expect(page.getByRole("option")).toHaveCount(0);
    await expect(page.getByTestId("gdtf-mode-picker")).toContainText(
      "No mode matches",
    );
    await expect(page.getByTestId("gdtf-import-confirm")).toHaveCount(0);
  });

  test("the list is a listbox the arrow keys walk", async ({ page }) => {
    await open(page);
    const list = page.getByRole("listbox", { name: "DMX modes" });
    await list.focus();
    await page.keyboard.press("ArrowDown");
    await expect(option(page, "RGB + strobe")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await page.keyboard.press("End");
    // The last offered mode: the refused one after it is skipped.
    await expect(option(page, "Pixel 12")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await page.keyboard.press("ArrowDown");
    await expect(option(page, "Pixel 12")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await page.keyboard.press("Home");
    await expect(option(page, "RGB")).toHaveAttribute("aria-selected", "true");
    await expect(list).toHaveAttribute("aria-activedescendant", /gdtf-mode-0/);
  });

  test("a colour-only mode dims through colour and points at modes that add the rest", async ({
    page,
  }) => {
    await open(page);
    const can = page.getByTestId("gdtf-can");
    await expect(can.locator("li")).toHaveText([
      "Set any colour",
      "Dim, through colour",
    ]);
    const missing = page.getByTestId("gdtf-missing");
    await expect(missing).toContainText(
      'Strobe — the mode "RGB + strobe" has it',
    );
    await expect(missing).toContainText('Movement — the mode "Mover" has it');
    await expect(missing).toContainText(
      'Per-pixel effects — the mode "Pixel 12" has it',
    );
    // Dimming is not missing where colour can dim.
    await expect(missing).not.toContainText("Dimming");
  });

  test("a strobe channel says how fast it flashes", async ({ page }) => {
    await open(page);
    await option(page, "RGB + strobe").click();
    await expect(page.getByTestId("gdtf-can").locator("li")).toHaveText([
      "Set any colour",
      "Dim",
      "Strobe, 0.4 to 25 flashes a second",
    ]);
    await expect(page.getByTestId("gdtf-missing")).not.toContainText("Strobe");
    await expect(page.getByTestId("gdtf-address")).toHaveText(
      "Occupies 5 addresses: dimmer, red, green, blue, strobe — patch the next fixture at least 5 on",
    );
  });

  test("a wheel-only mover is told to pick a slot with a static", async ({
    page,
  }) => {
    await open(page);
    await option(page, "Mover").click();
    await expect(page.getByTestId("gdtf-can").locator("li")).toHaveText([
      "Dim",
      "Move",
      "Colour from a wheel — pick a slot with a static",
    ]);
    await expect(page.getByTestId("gdtf-extras")).toContainText("gobo");
    await expect(page.getByTestId("gdtf-missing")).toContainText(
      'Colour — the mode "RGB" has it',
    );
    await expect(page.getByTestId("gdtf-address")).toContainText(
      "Occupies 8 addresses: pan, pan_fine, tilt, tilt_fine, dimmer, color1",
    );
  });

  test("a pixel mode counts its cells", async ({ page }) => {
    await open(page);
    await option(page, "Pixel 12").click();
    await expect(page.getByTestId("gdtf-can")).toContainText(
      "Per-pixel effects, 12 cells",
    );
    await expect(page.getByTestId("gdtf-missing")).not.toContainText(
      "Per-pixel",
    );
  });

  test("a refused mode is listed greyed with its reason and cannot be chosen", async ({
    page,
  }) => {
    await open(page);
    const matrix = option(page, "Matrix");
    await expect(matrix).toHaveAttribute("aria-disabled", "true");
    await expect(matrix.getByTestId("gdtf-refused")).toContainText(
      "another mode has this name",
    );
    await matrix.click({ force: true });
    await expect(matrix).toHaveAttribute("aria-selected", "false");
    await expect(option(page, "RGB")).toHaveAttribute("aria-selected", "true");
  });

  test("the type name defaults to a safe one and shows the file it writes", async ({
    page,
  }) => {
    await open(page);
    const name = page.getByTestId("gdtf-type-name");
    await expect(name).toHaveValue("Spiider Pro v2");
    await expect(page.getByTestId("gdtf-file")).toHaveText(
      "Writes lighting/fixture_types/spiider_pro_v2.fixture",
    );
    await name.fill('Mini "Wash" {A}');
    await expect(name).toHaveValue("Mini Wash A");
    await expect(page.getByTestId("gdtf-file")).toContainText(
      "mini_wash_a.fixture",
    );
    // Nothing left to save under: the import is not offered.
    await name.fill("");
    await expect(page.getByTestId("gdtf-import-confirm")).toBeDisabled();
  });

  test("the channel map and warnings sit behind a disclosure", async ({
    page,
  }) => {
    await open(page);
    await option(page, "RGB + strobe").click();
    await expect(page.getByTestId("gdtf-channels")).toBeHidden();
    await page.getByText("Channel map and warnings").click();
    await expect(page.getByTestId("gdtf-channels")).toContainText(
      "Address 5: strobe",
    );
    await expect(page.getByTestId("gdtf-warnings")).toContainText(
      "skipped virtual channel",
    );
  });

  test("Add fixture type imports the chosen mode under the chosen name", async ({
    page,
  }) => {
    const imports: URL[] = [];
    await page.route("**/api/lighting/gdtf/import**", async (route) => {
      imports.push(new URL(route.request().url()));
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          type_name: "Stage Mover",
          mode: "Mover",
          archive: "lighting/library/spiider.gdtf",
          replaced_archive: false,
          fixture_file: "lighting/fixture_types/stage_mover.fixture",
          channels: [[1, "pan"]],
          warnings: [],
        }),
      });
    });
    await open(page);
    await option(page, "Mover").click();
    await page.getByTestId("gdtf-type-name").fill("Stage Mover");
    await page.getByRole("button", { name: "Add fixture type" }).click();

    await expect(page.getByTestId("gdtf-report")).toContainText(
      "stage_mover.fixture",
    );
    expect(imports).toHaveLength(1);
    expect(imports[0].searchParams.get("mode")).toBe("Mover");
    expect(imports[0].searchParams.get("name")).toBe("Stage Mover");
    await expect(page.getByTestId("gdtf-mode-picker")).toHaveCount(0);
  });

  test("Cancel closes the picker", async ({ page }) => {
    await open(page);
    await page
      .getByTestId("gdtf-mode-picker")
      .getByRole("button", { name: "Cancel" })
      .click();
    await expect(page.getByTestId("gdtf-mode-picker")).toHaveCount(0);
  });

  test("fits a phone", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 800 });
    await open(page);
    const overflow = await page.evaluate(
      () =>
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
    await expect(page.getByTestId("gdtf-import-confirm")).toBeVisible();
  });
});
