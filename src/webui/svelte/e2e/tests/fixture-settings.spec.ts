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

// The fixture page's settings form (lighting UI design §12.4): name, default
// mode and movement limits, saved through a plan the user confirms, with the
// file behind a disclosure. The mock's settings routes are stateless; tests
// that need another answer route their own with `page.route`.

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

  test("the form shows the fixture's settings and the file is tucked away", async ({
    page,
  }) => {
    await card(page, "pixelbrick").click();
    const settings = page.getByTestId("ft-settings");
    await expect(settings).toContainText("Your settings for this fixture");
    await expect(page.getByTestId("ft-set-name")).toHaveValue("pixelbrick");
    await expect(page.getByTestId("ft-set-default")).toHaveValue("8: RGBS");
    // No mode pans or tilts: nothing to limit.
    await expect(page.getByTestId("ft-set-not-moving")).toBeVisible();
    await expect(page.getByTestId("ft-set-pan")).toHaveCount(0);
    // The file, behind a closed disclosure; the page never says "definition".
    const file = page.getByTestId("ft-file");
    await expect(file.locator("summary")).toHaveText(
      "Saved as pixelbrick.fixture · show the file",
    );
    await expect(file).not.toHaveAttribute("open", "");
    await expect(page.locator(".editor-form")).not.toContainText(/definition/i);
    await expect(page.getByTestId("ft-set-save")).toBeDisabled();
  });

  test("make-default from the list, then the save asks about the fixtures it changes", async ({
    page,
  }) => {
    const posts = settingsPosts(page);
    await card(page, "pixelbrick").click();
    const modes = page.getByTestId("ft-details-modes");
    await modes.locator('[data-mode="1: RGB"]').click();
    await page.getByTestId("ft-make-default").click();
    await expect(page.getByTestId("ft-set-default")).toHaveValue("1: RGB");
    // The default's own detail offers no button.
    await expect(page.getByTestId("ft-make-default")).toHaveCount(0);

    // Cancelled: planned, never written.
    await page.getByTestId("ft-set-save").click();
    await expect(confirmDialog(page)).toContainText(
      "2 fixtures in built-in use the default and will change to 1: RGB.",
    );
    await confirmDialog(page).getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByTestId("ft-set-msg")).toHaveText("Not saved.");
    expect(posts.map((p) => p.write)).toEqual([false]);

    // Confirmed: written with the plan's venue versions and the file's.
    await page.getByTestId("ft-set-save").click();
    await confirmDialog(page).getByRole("button", { name: "Save" }).click();
    await expect.poll(() => posts.length).toBe(3);
    const write = posts[2];
    expect(write.write).toBe(true);
    expect(write.body.default_mode).toBe("1: RGB");
    expect(write.body.venue_versions).toEqual({
      "built_in.venue": "v-b",
      "club.venue": "v-c",
    });
    expect(write.req.headers()["if-match"]).toBe("mock-v1");
    await expect(page.getByTestId("ft-set-msg")).toContainText("Saved");
  });

  test("a new default that would overlap is a warning that needs a confirm", async ({
    page,
  }) => {
    const posts = settingsPosts(page);
    await page.route(
      "**/api/lighting/fixture-types/pixelbrick/settings*",
      (r) => {
        if (r.request().method() !== "POST") return r.fallback();
        return r.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            write: r.request().postDataJSON().write,
            file: "pixelbrick.fixture",
            version: "mock-v1",
            dsl: "",
            rename: null,
            default_change: {
              from: "8: RGBS",
              to: "1: RGB",
              footprint: 7,
              count: 1,
              venues: [{ venue: "built-in", fixtures: ["Brick1"] }],
            },
            overlaps: [
              {
                venue: "built-in",
                a: "Brick1",
                b: "Brick2",
                a_gang: ["Brick1"],
                b_gang: ["Brick2"],
                universe: 1,
                from: 5,
                to: 7,
                message:
                  'fixtures "Brick1" and "Brick2" are both patched on universe 1 at addresses 5-7; each will overwrite the other',
              },
            ],
            overruns: [],
            venue_versions: {},
            config_references: [],
            reloaded: false,
            venue_error: null,
          }),
        });
      },
    );
    await card(page, "pixelbrick").click();
    await page.getByTestId("ft-set-default").selectOption("1: RGB");
    await page.getByTestId("ft-set-save").click();
    const dialog = confirmDialog(page);
    await expect(dialog).toContainText(
      "1 fixture in built-in uses the default and will change to 1: RGB.",
    );
    await expect(dialog).toContainText(
      'Warning — built-in: fixtures "Brick1" and "Brick2"',
    );
    await expect(dialog.getByRole("button", { name: "Save" })).toHaveClass(
      /btn-danger/,
    );
    await dialog.getByRole("button", { name: "Cancel" }).click();
    expect(posts.every((p) => !p.write)).toBe(true);
  });

  test("a rename says which venue lines it rewrites", async ({ page }) => {
    const posts = settingsPosts(page);
    // After the save the page follows the new name; the stateless mock
    // still knows the type as pixelbrick.
    await page.route(/\/api\/lighting\/fixture-types\/PB15/, (r) =>
      r.continue({ url: r.request().url().replace("PB15", "pixelbrick") }),
    );
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
  });

  test("the file behind the disclosure still edits and saves, and locks the form while it does", async ({
    page,
  }) => {
    await card(page, "pixelbrick").click();
    await page.getByTestId("ft-file").locator("summary").click();
    const dsl = page.getByTestId("ft-dsl");
    await expect(dsl).toHaveValue(/from gdtf\(/);
    await expect(page.getByTestId("ft-file-save")).toBeDisabled();
    await dsl.fill(
      'fixture_type "pixelbrick" from gdtf("library/pb15.gdtf", mode "1: RGB") {\n}\n',
    );
    // Only one of the two may have unsaved changes.
    await expect(page.getByTestId("ft-settings-locked")).toBeVisible();
    await expect(page.getByTestId("ft-set-name")).toBeDisabled();

    const put = page.waitForRequest(
      (r) =>
        r.method() === "PUT" &&
        /\/api\/lighting\/fixture-types\/pixelbrick/.test(r.url()),
    );
    await page.getByTestId("ft-file-save").click();
    const request = await put;
    expect(request.postData()).toContain('mode "1: RGB"');
    // Still on the page, reloaded.
    await expect(page.getByTestId("ft-settings")).toBeVisible();
    await expect(page.getByTestId("ft-set-name")).toBeEnabled();
  });

  test("editing the form makes the file read-only until it is saved or discarded", async ({
    page,
  }) => {
    await card(page, "pixelbrick").click();
    await page.getByTestId("ft-set-name").fill("Other");
    await page.getByTestId("ft-file").locator("summary").click();
    await expect(page.getByTestId("ft-file-locked")).toBeVisible();
    await expect(page.getByTestId("ft-dsl")).toHaveAttribute("readonly", "");
    await page.getByTestId("ft-set-discard").click();
    await expect(page.getByTestId("ft-set-name")).toHaveValue("pixelbrick");
    await expect(page.getByTestId("ft-dsl")).not.toHaveAttribute("readonly");
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
                default_mode: "8: RGBS",
                movement: { max_pan_speed: 240, max_tilt_speed: null },
                file: "pixelbrick.fixture",
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
          mode: "8: RGBS",
          matched_mode: "8: RGBS",
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
    expect(posts.find((p) => p.write)!.body.movement).toEqual({
      max_pan_speed: 240,
      max_tilt_speed: 180,
    });
  });
});
