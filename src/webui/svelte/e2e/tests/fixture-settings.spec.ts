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

import { test, expect, type Page, type Request } from "@playwright/test";

// The fixture page's settings form (lighting UI design §12.4): name and
// movement limits, saved through a plan the user confirms. A fixture from a
// GDTF has no default mode and no file to show. The mock's settings routes
// are stateless; tests that need another answer route their own with
// `page.route`.

function card(page: Page, name: string) {
  return page.locator(".item-card").filter({
    has: page.locator(".item-name", { hasText: new RegExp(`^${name}$`) }),
  });
}

/** Every settings POST the page makes, by `write`. */
function settingsPosts(page: Page) {
  const posts: {
    write: boolean;
    body: Record<string, unknown>;
    req: Request;
  }[] = [];
  page.on("request", (req) => {
    if (req.method() === "POST" && /\/settings(\?|$)/.test(req.url())) {
      const body = req.postDataJSON();
      posts.push({ write: body.write, body, req });
    }
  });
  return posts;
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
    const posts = settingsPosts(page);
    // After the save the page follows the new name; the stateless mock
    // still knows the type as pixelbrick.
    await page.route(/\/api\/lighting\/fixture-types\/PB15/, (r) =>
      r.continue({ url: r.request().url().replace("PB15", "pixelbrick") }),
    );
    // The list, re-read after the save, knows it by its new name too (the
    // page's address follows the rename).
    await page.route(/\/api\/lighting\/fixture-types(\?.*)?$/, async (r) => {
      const res = await r.fetch();
      const body = await res.json();
      body.fixture_types.PB15 = body.fixture_types.pixelbrick;
      await r.fulfill({ response: res, json: body });
    });
    await card(page, "pixelbrick").click();
    await page.getByTestId("ft-set-name").fill("PB15");
    await page.getByTestId("ft-set-save").click();
    await expect(confirmDialog(page)).toContainText(
      'Renaming "pixelbrick" to "PB15" rewrites 3 venue lines: built-in (2), club (1).',
    );
    await confirmDialog(page).getByRole("button", { name: "Save" }).click();
    await expect.poll(() => posts.filter((p) => p.write).length).toBe(1);
    expect(posts.find((p) => p.write)!.body.name).toBe("PB15");
    await expect(page.getByTestId("ft-set-msg")).toContainText(
      "Renamed in 3 venue lines: built-in (2), club (1).",
    );
    await expect(page.getByTestId("ft-title")).toHaveText("PB15");
    await expect(page).toHaveURL(/#\/lighting\/fixtures\/PB15$/);
  });

  test("a fixture that pans or tilts shows its movement limits", async ({
    page,
  }) => {
    const posts = settingsPosts(page);
    await page.route(
      "**/api/lighting/fixture-types/pixelbrick/settings*",
      (r) =>
        r.request().method() === "GET"
          ? r.fulfill({
              status: 200,
              contentType: "application/json",
              body: JSON.stringify({
                name: "pixelbrick",
                movement: { max_pan_speed: 240, max_tilt_speed: null },
                version: "mock-v1",
              }),
            })
          : r.fallback(),
    );
    await page.route("**/api/lighting/fixture-types/pixelbrick/gdtf*", (r) =>
      r.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          archive: "library/mover.gdtf",
          rig: null,
          thumbnail: null,
          beam: null,
          about: null,
          venues: [],
          inspection: {
            fixture: "Mover",
            manufacturer: "Test",
            modes: [
              {
                name: "8: RGBS",
                channel_count: 6,
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
      }),
    );
    await card(page, "pixelbrick").click();
    await expect(page.getByTestId("ft-set-not-moving")).toHaveCount(0);
    await expect(page.getByTestId("ft-set-pan")).toHaveValue("240");
    await expect(page.getByTestId("ft-set-tilt")).toHaveValue("");
    await page.getByTestId("ft-set-tilt").fill("180");
    await page.getByTestId("ft-set-save").click();
    await expect.poll(() => posts.filter((p) => p.write).length).toBe(1);
    const write = posts.find((p) => p.write)!.body;
    expect(write.movement).toEqual({
      max_pan_speed: 240,
      max_tilt_speed: 180,
    });
    expect(write).not.toHaveProperty("default_mode");
  });
});
