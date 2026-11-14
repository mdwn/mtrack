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
import { LightingService } from "../../src/gen/player/v1/player_pb";
import { FIXTURE_TYPES, GDTF_INSPECTION } from "../mock-server/test-data";
import {
  fixtureSettingsReply,
  fixtureTypeGdtfReply,
  fixtureTypeInfoReply,
  fixtureTypeListReply,
  savedSettingsRequest,
} from "../support/lighting-rpc";
import { card } from "../support/locators";
import { mockRpc, type RpcMock } from "../support/rpc";

// The fixture page's settings form (lighting UI design §12.4): name and
// movement limits, saved through a plan the user confirms. A fixture from a
// GDTF has no default mode and no file to show. The mock's settings calls
// are stateless; tests that need another answer answer their own with
// `mockRpc`.

/** Every settings save the page makes, by `write`. */
function settingsPosts(rpc: RpcMock) {
  return {
    all: () =>
      rpc
        .calls(LightingService.method.saveFixtureTypeSettings)
        .map((r) => ({ write: r.write, body: savedSettingsRequest(r) })),
  };
}

const confirmDialog = (page: Page) => page.locator(".dialog-overlay");

test.describe("Fixture settings", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/lighting/fixtures");
    await expect(page.getByTestId("import-gdtf")).toBeVisible();
  });

  test("the form shows the fixture's settings, and nothing of a file or a default", async ({
    page,
  }) => {
    await card(page, "pixelbrick").click();
    const settings = page.getByTestId("ft-settings");
    await expect(settings).toContainText("Your settings for this fixture");
    await expect(page.getByTestId("ft-set-name")).toHaveValue("pixelbrick");
    // No mode pans or tilts: nothing to limit.
    await expect(page.getByTestId("ft-set-not-moving")).toBeVisible();
    await expect(page.getByTestId("ft-set-pan")).toHaveCount(0);
    // A fixture's mode is each venue fixture's choice: no default here.
    await expect(page.getByTestId("ft-set-default")).toHaveCount(0);
    await expect(page.getByTestId("ft-make-default")).toHaveCount(0);
    // mtrack's record is never shown as a file.
    await expect(page.getByTestId("ft-file")).toHaveCount(0);
    await expect(page.getByTestId("ft-dsl")).toHaveCount(0);
    const text = await page.locator(".editor-form").innerText();
    expect(text).not.toContain(".fixture");
    expect(text).not.toMatch(/definition/i);
    expect(text).not.toMatch(/default mode/i);
    await expect(page.getByTestId("ft-set-save")).toBeDisabled();
  });

  test("a rename says which venue lines it rewrites", async ({ page }) => {
    const rpc = await mockRpc(page);
    const posts = settingsPosts(rpc);
    // After the save the page follows the new name; the stateless mock
    // still knows the type as pixelbrick, so the type is answered under
    // either name.
    const pixelbrick = FIXTURE_TYPES.pixelbrick;
    const known = (name: string) => (name === "PB15" ? "pixelbrick" : name);
    rpc.unary(LightingService.method.getFixtureType, (req) =>
      fixtureTypeInfoReply(FIXTURE_TYPES[known(req.name)]),
    );
    rpc.unary(LightingService.method.getFixtureTypeSettings, (req) =>
      fixtureSettingsReply({
        name: req.name,
        movement: { max_pan_speed: null, max_tilt_speed: null },
        strobe_curve: null,
        strobe: { steps: 0, automatic: "declared" },
        version: "mock-v1",
      }),
    );
    rpc.unary(LightingService.method.getFixtureTypeGdtfDetails, () =>
      fixtureTypeGdtfReply({
        archive: "library/pb15.gdtf",
        rig: null,
        thumbnail: null,
        beam: null,
        about: null,
        venues: [],
        inspection: GDTF_INSPECTION,
      }),
    );
    // The list, re-read after the save, knows it by its new name too (the
    // page's address follows the rename).
    rpc.unary(
      LightingService.method.listFixtureTypes,
      fixtureTypeListReply({
        fixture_types: { ...FIXTURE_TYPES, PB15: pixelbrick },
        errors: [],
      }),
    );
    await card(page, "pixelbrick").click();
    await page.getByTestId("ft-set-name").fill("PB15");
    await page.getByTestId("ft-set-save").click();
    await expect(confirmDialog(page)).toContainText(
      'Renaming "pixelbrick" to "PB15" rewrites 3 venue lines: built-in (2), club (1).',
    );
    await confirmDialog(page).getByRole("button", { name: "Save" }).click();
    await expect.poll(() => posts.all().filter((p) => p.write).length).toBe(1);
    expect(posts.all().find((p) => p.write)!.body.name).toBe("PB15");
    await expect(page.getByTestId("ft-set-msg")).toContainText(
      "Renamed in 3 venue lines: built-in (2), club (1).",
    );
    await expect(page.getByTestId("ft-title")).toHaveText("PB15");
    await expect(page).toHaveURL(/#\/lighting\/fixtures\/PB15$/);
  });

  test("a fixture that pans or tilts shows its movement limits", async ({
    page,
  }) => {
    const rpc = await mockRpc(page);
    const posts = settingsPosts(rpc);
    rpc.unary(
      LightingService.method.getFixtureTypeSettings,
      fixtureSettingsReply({
        name: "pixelbrick",
        movement: { max_pan_speed: 240, max_tilt_speed: null },
        strobe_curve: null,
        strobe: null,
        version: "mock-v1",
      }),
    );
    rpc.unary(
      LightingService.method.getFixtureTypeGdtfDetails,
      fixtureTypeGdtfReply({
        archive: "library/mover.gdtf",
        rig: null,
        thumbnail: null,
        beam: null,
        about: null,
        venues: [],
        inspection: {
          modes: [
            {
              name: "8: RGBS",
              footprint: 6,
              capabilities: ["color", "pan_tilt"],
              channels: [
                [1, "pan"],
                [2, "tilt"],
              ],
            },
          ],
        },
      }),
    );
    await card(page, "pixelbrick").click();
    await expect(page.getByTestId("ft-set-not-moving")).toHaveCount(0);
    await expect(page.getByTestId("ft-set-pan")).toHaveValue("240");
    await expect(page.getByTestId("ft-set-tilt")).toHaveValue("");
    await page.getByTestId("ft-set-tilt").fill("180");
    await page.getByTestId("ft-set-save").click();
    await expect.poll(() => posts.all().filter((p) => p.write).length).toBe(1);
    const write = posts.all().find((p) => p.write)!.body;
    expect(write.movement).toEqual({
      max_pan_speed: 240,
      max_tilt_speed: 180,
    });
    expect(write).not.toHaveProperty("default_mode");
  });
});
