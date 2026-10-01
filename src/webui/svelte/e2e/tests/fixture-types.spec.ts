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

/** The card of one named type. The mock serves three, in all three forms,
 *  so position no longer identifies one. */
function card(page: Page, name: string) {
  return page.locator(".item-card").filter({
    has: page.locator(".item-name", { hasText: new RegExp(`^${name}$`) }),
  });
}

test.describe("Fixture Types Management", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/lighting/fixtures");
    await expect(page.getByTestId("import-gdtf")).toBeVisible();
  });

  test("a fixture type file that will not parse is named, and the rest still list", async ({
    page,
  }) => {
    // Same shape as the venue case: a directory is a set of independent files,
    // so one that no longer parses must not empty the list — and must not go
    // unmentioned either, or the only signal is a fixture type quietly missing.
    await page.route("**/api/lighting/fixture-types*", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          fixture_types: {
            RGBW_Par: {
              fixture_type: { name: "RGBW_Par", channels: {} },
              file: "rgbw_par.light",
              extension: "light",
              referential: false,
              rich: false,
            },
          },
          errors: [
            {
              file: "broken.light",
              error: "expected channel definition at line 4",
            },
          ],
        }),
      });
    });
    // Re-fetch through the panel's own Refresh rather than reloading the page,
    // which would discard the navigation `beforeEach` performed.
    await page.getByRole("button", { name: "Refresh" }).first().click();

    // The good fixture type is still listed.
    await expect(page.locator(".item-name")).toContainText("RGBW_Par");

    // And the bad file is named, with the reason.
    const errors = page.getByTestId("fixture-type-file-errors");
    await expect(errors).toBeVisible();
    await expect(errors).toContainText("broken.light");
    await expect(errors).toContainText("expected channel definition");
  });

  test("shows existing fixture type from mock data", async ({ page }) => {
    await expect(card(page, "par")).toBeVisible();
  });

  test("shows channel info for fixture type", async ({ page }) => {
    await expect(card(page, "par")).toContainText(/4 channels/);
  });

  test("each card names its file and its extension", async ({ page }) => {
    // The form a type is in decides how it may be edited, so the panel says
    // which form each one is in rather than leaving it to be discovered.
    await expect(card(page, "par").getByTestId("ft-ext")).toHaveText(".light");
    await expect(card(page, "mover").getByTestId("ft-ext")).toHaveText(
      ".fixture",
    );
    await expect(card(page, "mover")).toContainText("mover.fixture");
  });

  test("a fixture from a GDTF shows no file and no extension", async ({
    page,
  }) => {
    // mtrack's record of it is an implementation detail, never shown.
    const brick = card(page, "pixelbrick");
    await expect(brick.getByTestId("ft-ext")).toHaveCount(0);
    const text = await brick.innerText();
    expect(text).not.toContain(".fixture");
    expect(text).not.toMatch(/definition/i);
  });

  test("a referential type says where it comes from, not '0 channels'", async ({
    page,
  }) => {
    await expect(card(page, "pixelbrick")).not.toContainText("0 channels");
  });

  test("a referential card says what the fixture is, its modes and its mode in use", async ({
    page,
  }) => {
    const brick = card(page, "pixelbrick");
    await expect(brick.getByTestId("ft-card-fixture")).toHaveText(
      "Astera LED Technology · PB15 PixelBrick",
    );
    await expect(brick.getByTestId("ft-card-modes")).toContainText(
      "2 modes · wash, 13°",
    );
    // One pill per mode in use, most-used first.
    await expect(brick.getByTestId("ft-card-mode")).toHaveText([
      "8: RGBS × 2",
      "1: RGB × 1",
    ]);
    // No rig in the store yet: the dashed box, not a broken image.
    await expect(brick.getByTestId("ft-card-thumb")).toHaveCount(0);
    await expect(brick).toContainText("no model");
    // A native type's card is as it was.
    await expect(card(page, "par").getByTestId("ft-card-fixture")).toHaveCount(
      0,
    );
    await expect(card(page, "par")).toContainText("4 channels");
  });

  test("a GDTF card whose archive is unreadable falls back, and a used-by of 0 shows no pill", async ({
    page,
  }) => {
    const entry = (gdtf: unknown) => ({
      fixture_type: { name: "x", channels: {} },
      file: "x.fixture",
      extension: "fixture",
      referential: true,
      rich: false,
      gdtf,
    });
    await page.route("**/api/lighting/fixture-types*", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          fixture_types: {
            lost: entry(null),
            unused: entry({
              fixture: "PB15 PixelBrick",
              manufacturer: "Astera LED Technology",
              modes: 1,
              beam: null,
              thumbnail: "abc/thumbnail.png",
              used_by: 0,
              in_use: [],
            }),
            busy: entry({
              fixture: "PB15 PixelBrick",
              manufacturer: "Astera LED Technology",
              modes: 9,
              beam: null,
              thumbnail: null,
              used_by: 10,
              in_use: [
                { mode: "a", count: 4 },
                { mode: "b", count: 3 },
                { mode: "c", count: 2 },
                { mode: "d", count: 1 },
              ],
            }),
          },
          errors: [],
        }),
      }),
    );
    await page.getByRole("button", { name: "Refresh" }).first().click();
    await expect(card(page, "lost")).toContainText(
      "Its GDTF cannot be read right now.",
    );
    // At most three modes, then how many more.
    await expect(card(page, "busy").getByTestId("ft-card-mode")).toHaveText([
      "a × 4",
      "b × 3",
      "c × 2",
    ]);
    await expect(card(page, "busy").getByTestId("ft-card-more")).toHaveText(
      "+1 more",
    );
    // Nothing uses it: no pill (there is no default to show instead).
    await expect(card(page, "unused").getByTestId("ft-card-mode")).toHaveCount(
      0,
    );
    await expect(card(page, "unused").getByTestId("ft-card-modes")).toHaveText(
      /^1 mode\s/,
    );
    await expect(
      card(page, "unused").getByTestId("ft-card-thumb"),
    ).toHaveAttribute("src", "/api/lighting/assets/abc/thumbnail.png");
  });

  test("clicking fixture type opens editor form", async ({ page }) => {
    await card(page, "par").click();
    await expect(page.locator(".editor-form")).toBeVisible();
    await expect(page.locator("#ft-name")).toBeVisible();
  });

  test("editor shows channel rows for existing fixture", async ({ page }) => {
    await card(page, "par").click();
    await expect(page.locator(".channel-row")).toHaveCount(4); // red, green, blue, dimmer
  });

  test("a .fixture type opens the text editor, not the channel map", async ({
    page,
  }) => {
    await card(page, "mover").click();
    const editor = page.getByTestId("ft-text-editor");
    await expect(editor).toBeVisible();
    await expect(page.getByTestId("ft-dsl")).toHaveValue(/channel "pan" @ 1/);
    await expect(page.locator(".channel-row")).toHaveCount(0);
  });

  test("a fixture from a GDTF opens on its page, with no text to edit", async ({
    page,
  }) => {
    await card(page, "pixelbrick").click();
    await expect(page.getByTestId("ft-details")).toBeVisible();
    await expect(page.getByTestId("ft-dsl")).toHaveCount(0);
    await expect(page.getByTestId("ft-referential-note")).toHaveCount(0);
  });

  test("saving a .fixture type PUTs the DSL as text", async ({ page }) => {
    await card(page, "mover").click();
    await expect(page.getByTestId("ft-dsl")).toBeVisible();

    const requestPromise = page.waitForRequest(
      (req) =>
        req.url().includes("/api/lighting/fixture-types/mover") &&
        req.method() === "PUT",
    );
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    const request = await requestPromise;
    expect(request.url()).toContain("ext=fixture");
    expect(request.headers()["content-type"]).toContain("text/plain");
    expect(request.postData()).toContain('channel "pan"');
  });

  test("a .light type can be opened as text and converted", async ({
    page,
  }) => {
    // The channel map is the default way in; this is the way out of it, and
    // the only path from a v1 file to the rich form.
    await card(page, "par").getByTestId("ft-edit-text").click();
    await expect(page.getByTestId("ft-dsl")).toHaveValue(/fixture_type par/);
    await expect(page.locator(".channel-row")).toHaveCount(0);

    // Saved back as it was found, unless the form is changed.
    const selector = page.getByTestId("ft-ext-select");
    await expect(selector).toHaveValue("light");
    await selector.selectOption("fixture");

    const requestPromise = page.waitForRequest(
      (req) =>
        req.url().includes("/api/lighting/fixture-types/par") &&
        req.method() === "PUT",
    );
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    const request = await requestPromise;
    expect(request.url()).toContain("ext=fixture");
  });

  test("a rich type is not offered the .light form", async ({ page }) => {
    // v2 syntax in a `.light` file is skipped by the loader, so there is no
    // choice to offer.
    await card(page, "mover").click();
    await expect(page.getByTestId("ft-dsl")).toBeVisible();
    await expect(page.getByTestId("ft-ext-select")).toHaveCount(0);
  });

  test("the name in text mode is read from the definition", async ({
    page,
  }) => {
    // The file is keyed on the declared name, and the server refuses a save
    // where the URL and the text disagree — so the field reads it out.
    await card(page, "mover").click();
    const nameField = page.getByTestId("ft-name-derived");
    await expect(nameField).toHaveValue("mover");
    await expect(nameField).toHaveJSProperty("readOnly", true);

    await page
      .getByTestId("ft-dsl")
      .fill('fixture_type "renamed" {\n  channel "pan" @ 1 fine 2\n}\n');
    await expect(nameField).toHaveValue("renamed");

    // The rename goes to the declared name, and the old file is deleted —
    // the same two steps the channel-map form takes.
    const putPromise = page.waitForRequest(
      (req) =>
        req.url().includes("/api/lighting/fixture-types/renamed") &&
        req.method() === "PUT",
    );
    const deletePromise = page.waitForRequest(
      (req) =>
        req.url().includes("/api/lighting/fixture-types/mover") &&
        req.method() === "DELETE",
    );
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    await putPromise;
    await deletePromise;
  });

  test("Define a fixture by hand offers both forms", async ({ page }) => {
    await page
      .getByRole("button", { name: "Define a fixture by hand" })
      .click();
    await expect(page.getByTestId("new-ft-choice")).toBeVisible();
    await page.getByRole("button", { name: ".light (channel map)" }).click();
    await expect(page.locator(".editor-form")).toBeVisible();
    await expect(page.locator("#ft-name")).toHaveValue("");
  });

  test("a new .fixture starts from a commented template", async ({ page }) => {
    await page
      .getByRole("button", { name: "Define a fixture by hand" })
      .click();
    await page.getByTestId("new-ft-fixture").click();
    await expect(page.getByTestId("ft-dsl")).toHaveValue(
      /channel "dimmer" @ 1/,
    );
  });

  test("Add Channel adds a new channel row", async ({ page }) => {
    await card(page, "par").click();
    const initialCount = await page.locator(".channel-row").count();
    await page.getByRole("button", { name: "Add Channel" }).click();
    await expect(page.locator(".channel-row")).toHaveCount(initialCount + 1);
  });

  test("removing a channel row decreases count", async ({ page }) => {
    await card(page, "par").click();
    const initialCount = await page.locator(".channel-row").count();
    // Click the first X button in a channel row.
    await page.locator(".channel-row").first().locator(".btn-danger").click();
    await expect(page.locator(".channel-row")).toHaveCount(initialCount - 1);
  });

  test("saving fixture type calls PUT API", async ({ page }) => {
    let saveCalled = false;
    await page.route("**/api/lighting/fixture-types/*", async (route) => {
      if (route.request().method() === "PUT") {
        saveCalled = true;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "saved" }),
        });
      } else {
        await route.continue();
      }
    });

    await card(page, "par").click();
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    expect(saveCalled).toBe(true);
  });

  test("deleting a fixture from a GDTF names the venues that use it, then DELETEs", async ({
    page,
  }) => {
    const deletes: string[] = [];
    page.on("request", (req) => {
      if (req.method() === "DELETE") deletes.push(req.url());
    });
    await card(page, "pixelbrick")
      .getByRole("button", { name: "Delete" })
      .click();
    const dialog = page.locator(".dialog-overlay");
    await expect(dialog).toContainText('Delete the fixture "pixelbrick"?');
    await expect(dialog).toContainText(
      "2 fixtures in built-in use it, and that venue will stop loading.",
    );
    await expect(dialog).toContainText(
      "1 fixture in club uses it, and that venue will stop loading.",
    );
    // Cancelled: nothing is deleted.
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await page.waitForTimeout(300);
    expect(deletes).toHaveLength(0);

    await card(page, "pixelbrick")
      .getByRole("button", { name: "Delete" })
      .click();
    const requestPromise = page.waitForRequest(
      (req) =>
        req.url().includes("/api/lighting/fixture-types/pixelbrick") &&
        req.method() === "DELETE",
    );
    await dialog.getByRole("button", { name: "Confirm" }).click();
    await requestPromise;
  });

  test("cancel button closes editor form", async ({ page }) => {
    await card(page, "par").click();
    await expect(page.locator(".editor-form")).toBeVisible();
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.locator(".editor-form")).not.toBeVisible();
  });

  test("the venue editor offers .fixture types too", async ({ page }) => {
    // A type the panel could not list was also missing from this dropdown,
    // which is where a venue's fixtures pick one.
    await page.goto("/#/lighting/venues");
    await page.locator('[data-testid^="venue-edit-"]').first().click();
    await expect(page.locator(".editor-form")).toBeVisible();
    const options = page.locator(".editor-form select option");
    await expect(options.filter({ hasText: "pixelbrick" })).toHaveCount(1);
    await expect(options.filter({ hasText: "mover" })).toHaveCount(1);
  });
});

test.describe("GDTF Import", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/lighting/fixtures");
  });

  test("choosing a file imports it in one step and opens the fixture", async ({
    page,
  }) => {
    const imports: string[] = [];
    page.on("request", (req) => {
      if (req.url().includes("/api/lighting/gdtf/")) imports.push(req.url());
    });
    await page.locator('input[type="file"]').setInputFiles({
      name: "pb15.gdtf",
      mimeType: "application/octet-stream",
      buffer: Buffer.from("not read by the mock"),
    });
    const report = page.getByTestId("gdtf-report");
    await expect(report).toHaveText(
      "Imported PB15 PixelBrick (Astera LED Technology) — 2 modes.",
    );
    // One request: the import itself, with no mode and no name.
    expect(imports).toHaveLength(1);
    const url = new URL(imports[0]);
    expect(url.pathname).toBe("/api/lighting/gdtf/import");
    expect(url.searchParams.get("mode")).toBeNull();
    expect(url.searchParams.get("name")).toBeNull();
    // The fixture's page is open.
    await expect(page.getByTestId("ft-title")).toHaveText("pixelbrick");
    await expect(page.getByTestId("ft-details")).toBeVisible();
    // In the user's terms: no file, no "definition".
    const text = await report.innerText();
    expect(text).not.toContain(".fixture");
    expect(text).not.toMatch(/definition/i);
  });

  for (const [what, answer, said] of [
    [
      "already imported",
      { already_imported: true },
      "pixelbrick was already imported from this GDTF; nothing changed.",
    ],
    [
      "renamed",
      { renamed_from: "PB15 PixelBrick" },
      "Imported PB15 PixelBrick (Astera LED Technology) — 2 modes. Named pixelbrick, because a fixture called PB15 PixelBrick already exists.",
    ],
  ] as const) {
    test(`an import that was ${what} says so`, async ({ page }) => {
      await page.route("**/api/lighting/gdtf/import*", (route) =>
        route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            type_name: "pixelbrick",
            fixture: "PB15 PixelBrick",
            manufacturer: "Astera LED Technology",
            modes: 2,
            archive: "lighting/library/pb15.gdtf",
            already_imported: false,
            renamed_from: null,
            refused_modes: [],
            warnings: [],
            ...answer,
          }),
        }),
      );
      await page.locator('input[type="file"]').setInputFiles({
        name: "pb15.gdtf",
        mimeType: "application/octet-stream",
        buffer: Buffer.from("not read by the mock"),
      });
      await expect(page.getByTestId("gdtf-report")).toHaveText(said);
      await expect(page.getByTestId("ft-title")).toHaveText("pixelbrick");
    });
  }

  test("an unparseable upload surfaces the error", async ({ page }) => {
    await page.route("**/api/lighting/gdtf/import*", async (route) => {
      await route.fulfill({
        status: 400,
        contentType: "application/json",
        body: JSON.stringify({ error: "Not a parseable GDTF: not a zip" }),
      });
    });
    await page.locator('input[type="file"]').setInputFiles({
      name: "junk.gdtf",
      mimeType: "application/octet-stream",
      buffer: Buffer.from("junk"),
    });
    await expect(page.getByTestId("gdtf-error")).toContainText("not a zip");
  });
});
