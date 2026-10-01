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

// A GDTF-referential type's details: what its archive holds, read from
// GET /api/lighting/fixture-types/{name}/gdtf. Every test here only reads
// the mock; nothing is saved, so no test sees another's state.

function card(page: Page, name: string) {
  return page.locator(".item-card").filter({
    has: page.locator(".item-name", { hasText: new RegExp(`^${name}$`) }),
  });
}

test.describe("Fixture type details (GDTF)", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/lighting/fixtures");
    await expect(page.getByTestId("import-gdtf")).toBeVisible();
  });

  test("a fixture from a GDTF shows its archive, its modes and the ones in use", async ({
    page,
  }) => {
    await card(page, "pixelbrick").click();
    const details = page.getByTestId("ft-details");
    await expect(details).toBeVisible();
    // Titled as the fixture: the type's name, then what the archive says it is.
    await expect(page.getByTestId("ft-title")).toHaveText("pixelbrick");
    await expect(page.getByTestId("ft-title-sub")).toHaveText(
      "Astera LED Technology · PB15 PixelBrick",
    );
    await expect(page.getByTestId("ft-details-archive")).toHaveText(
      "library/pb15.gdtf",
    );
    await expect(page.getByTestId("ft-fact-modes")).toHaveText("2");
    await expect(page.getByTestId("ft-fact-beam")).toHaveText(
      "Wash · 13° beam, 25° field",
    );
    await expect(page.getByTestId("ft-fact-output")).toHaveText(
      "475 lm · 5500 K",
    );
    await expect(page.getByTestId("ft-fact-power")).toHaveText("12 W");
    await expect(page.getByTestId("ft-fact-venues")).toHaveText(
      "built-in: 2 fixtures, club: 1 fixture",
    );
    await expect(page.getByTestId("ft-details-about")).toContainText(
      "Battery-powered uplight",
    );

    // Every mode of the archive, each one in use saying how many use it;
    // there is no default. The page opens on the mode used most.
    const modes = page.getByTestId("ft-details-modes");
    await expect(modes.getByRole("option")).toHaveCount(2);
    await expect(page.getByTestId("ft-details-mode-default")).toHaveCount(0);
    const rgbs = modes.locator('[data-mode="8: RGBS"]');
    await expect(rgbs.getByTestId("ft-details-mode-in-use")).toHaveText(
      "2 in use",
    );
    await expect(rgbs).toHaveAttribute("aria-selected", "true");
    await expect(
      modes
        .locator('[data-mode="1: RGB"]')
        .getByTestId("ft-details-mode-in-use"),
    ).toHaveText("1 in use");
    await expect(
      page.getByTestId("ft-details-channels").locator("tbody tr").nth(3),
    ).toHaveText(/4\s*strobe/);
    // The mode's users; the viewer names the mode being looked at.
    await expect(page.getByTestId("ft-details-used-by")).toHaveText(
      "Used by Brick1, Brick2 in built-in.",
    );
    await expect(page.getByTestId("ft-viewer-mode")).toHaveText("8: RGBS");

    // The viewer drew, or fell back — never blank, never an error.
    await expect(page.getByTestId("ft-viewer")).toHaveAttribute(
      "data-renderer",
      /webgl|none/,
      { timeout: 15000 },
    );

    // mtrack's record of it is never shown: no file, no text editor.
    await expect(page.getByTestId("ft-dsl")).toHaveCount(0);
    const text = await page.locator(".editor-form").innerText();
    expect(text).not.toContain(".fixture");
    expect(text).not.toMatch(/definition/i);
  });

  test("choosing another mode shows its channel map, by click and by arrow", async ({
    page,
  }) => {
    await card(page, "pixelbrick").click();
    const modes = page.getByTestId("ft-details-modes");
    await expect(modes.getByRole("option")).toHaveCount(2);

    await modes.locator('[data-mode="1: RGB"]').click();
    await expect(modes.locator('[data-mode="1: RGB"]')).toHaveAttribute(
      "aria-selected",
      "true",
    );
    const channels = page.getByTestId("ft-details-channels");
    await expect(channels.locator("tbody tr")).toHaveCount(3);
    await expect(channels.locator("tbody tr").nth(2)).toHaveText(/3\s*blue/);
    await expect(channels).not.toContainText("strobe");
    // Each mode lists the fixtures in it.
    await expect(page.getByTestId("ft-details-used-by")).toHaveText(
      "Used by Solo in club.",
    );
    await expect(page.getByTestId("ft-viewer-mode")).toHaveText("1: RGB");
    // What a show can do is the picker's words: this mode has no strobe,
    // and says which mode does.
    await expect(page.getByTestId("ft-details-mode-detail")).toContainText(
      'Strobe — the mode "8: RGBS" has it',
    );
    // The in-use pills stay where they are, whichever is being looked at.
    await expect(
      modes
        .locator('[data-mode="8: RGBS"]')
        .getByTestId("ft-details-mode-in-use"),
    ).toHaveText("2 in use");

    await modes.press("ArrowDown");
    await expect(modes.locator('[data-mode="8: RGBS"]')).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(channels.locator("tbody tr")).toHaveCount(4);
  });

  test("a fixture whose mode the archive lacks is named in red", async ({
    page,
  }) => {
    await page.route("**/api/lighting/fixture-types/pixelbrick/gdtf*", (r) =>
      r.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          archive: "library/pb15.gdtf",
          rig: null,
          thumbnail: null,
          beam: null,
          about: null,
          venues: [
            {
              name: "club",
              fixtures: [
                { name: "Solo", mode: "1: RGB" },
                { name: "Odd", mode: null },
              ],
            },
          ],
          inspection: {
            fixture: "PB15 PixelBrick",
            manufacturer: "Astera LED Technology",
            modes: [
              { name: "1: RGB", channel_count: 3, footprint: 3 },
              { name: "8: RGBS", channel_count: 4, footprint: 4 },
            ],
          },
        }),
      }),
    );
    await card(page, "pixelbrick").click();
    // Having no default is the normal state: nothing is said about it.
    await expect(page.getByTestId("ft-details-no-default")).toHaveCount(0);
    await expect(page.getByTestId("ft-details-unresolved")).toContainText(
      "Odd (club)",
    );
    await expect(page.getByTestId("ft-details-mode-default")).toHaveCount(0);
    await expect(page.getByTestId("ft-details-mode-in-use")).toHaveText(
      "1 in use",
    );
  });

  test("the filter narrows the modes", async ({ page }) => {
    await card(page, "pixelbrick").click();
    const modes = page.getByTestId("ft-details-modes");
    await expect(modes.getByRole("option")).toHaveCount(2);
    await page.getByTestId("ft-details-filter").fill("rgbs");
    await expect(modes.getByRole("option")).toHaveCount(1);
    await expect(modes.getByRole("option")).toHaveAttribute(
      "data-mode",
      "8: RGBS",
    );
    await page.getByTestId("ft-details-filter").fill("nothing like it");
    await expect(modes.getByRole("option")).toHaveCount(0);
    await expect(modes).toContainText("No mode matches.");
  });

  test("a native type has no details", async ({ page }) => {
    await card(page, "mover").click();
    await expect(page.getByTestId("ft-dsl")).toBeVisible();
    await expect(page.getByTestId("ft-details")).toHaveCount(0);
  });

  test("an archive that cannot be read is said inline, with no file to edit", async ({
    page,
  }) => {
    await page.route("**/api/lighting/fixture-types/pixelbrick/gdtf*", (r) =>
      r.fulfill({
        status: 400,
        contentType: "application/json",
        body: JSON.stringify({
          error:
            'fixture type "pixelbrick" references a GDTF archive that cannot be read: library/pb15.gdtf',
        }),
      }),
    );
    await card(page, "pixelbrick").click();
    await expect(page.getByTestId("ft-details-error")).toContainText(
      "library/pb15.gdtf",
    );
    await expect(page.getByTestId("ft-dsl")).toHaveCount(0);
    // Back still leaves the page.
    await page.getByRole("button", { name: "Back" }).click();
    await expect(card(page, "pixelbrick")).toBeVisible();
  });

  test("without a rig the viewer shows the thumbnail, and unstated facts are omitted", async ({
    page,
  }) => {
    await page.route("**/api/lighting/fixture-types/pixelbrick/gdtf*", (r) =>
      r.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          archive: "library/pb15.gdtf",
          rig: null,
          thumbnail: "abc/thumbnail.png",
          beam: {
            type: null,
            beam_angle: null,
            field_angle: null,
            luminous_flux: 800,
            color_temperature: null,
            power: null,
          },
          about: null,
          venues: [],
          inspection: {
            fixture: "PB15 PixelBrick",
            manufacturer: "Astera LED Technology",
            modes: [{ name: "8: RGBS", channel_count: 4, footprint: 4 }],
          },
        }),
      }),
    );
    await card(page, "pixelbrick").click();
    await expect(page.getByTestId("ft-viewer")).toHaveAttribute(
      "data-renderer",
      "none",
    );
    await expect(page.getByTestId("ft-viewer-thumbnail")).toHaveAttribute(
      "src",
      "/api/lighting/assets/abc/thumbnail.png",
    );
    // What the archive does not state is left out, not shown as 0.
    await expect(page.getByTestId("ft-fact-output")).toHaveText("800 lm");
    await expect(page.getByTestId("ft-fact-beam")).toHaveCount(0);
    await expect(page.getByTestId("ft-fact-power")).toHaveCount(0);
    await expect(page.getByTestId("ft-fact-venues")).toHaveCount(0);
    await expect(page.getByTestId("ft-details-about")).toHaveCount(0);
    await expect(page.getByTestId("ft-details-used-by")).toHaveText(
      "No fixture uses this mode.",
    );
  });
});
