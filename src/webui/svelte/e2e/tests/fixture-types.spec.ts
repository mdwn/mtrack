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
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await page.locator(".tab", { hasText: "Lighting" }).click();
    await expect(page.locator(".tab.active")).toContainText("Lighting");
    await page.getByRole("button", { name: "Enable Lighting" }).click();
    // Navigate to Fixture Types sub-tab.
    await page.locator(".sub-tab", { hasText: "Fixture Types" }).click();
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

  test("a referential type says where it comes from, not '0 channels'", async ({
    page,
  }) => {
    await expect(card(page, "pixelbrick")).toContainText("GDTF archive");
    await expect(card(page, "pixelbrick")).not.toContainText("0 channels");
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

  test("a referential type is edited as text, with its gdtf line kept", async ({
    page,
  }) => {
    await card(page, "pixelbrick").click();
    await expect(page.getByTestId("ft-referential-note")).toContainText(
      "from gdtf",
    );
    await expect(page.getByTestId("ft-dsl")).toHaveValue(/from gdtf\(/);
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

  test("a rich or referential type is not offered the .light form", async ({
    page,
  }) => {
    // v2 syntax in a `.light` file is skipped by the loader, so there is no
    // choice to offer.
    await card(page, "pixelbrick").click();
    await expect(page.getByTestId("ft-dsl")).toBeVisible();
    await expect(page.getByTestId("ft-ext-select")).toHaveCount(0);

    await page.getByRole("button", { name: "Cancel" }).click();
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

  test("New Fixture Type offers both forms", async ({ page }) => {
    await page.getByRole("button", { name: "New Fixture Type" }).click();
    await expect(page.getByTestId("new-ft-choice")).toBeVisible();
    await page.getByRole("button", { name: ".light (channel map)" }).click();
    await expect(page.locator(".editor-form")).toBeVisible();
    await expect(page.locator("#ft-name")).toHaveValue("");
  });

  test("a new .fixture starts from a commented template", async ({ page }) => {
    await page.getByRole("button", { name: "New Fixture Type" }).click();
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

  test("deleting a .fixture type calls DELETE", async ({ page }) => {
    const requestPromise = page.waitForRequest(
      (req) =>
        req.url().includes("/api/lighting/fixture-types/pixelbrick") &&
        req.method() === "DELETE",
    );
    await card(page, "pixelbrick")
      .getByRole("button", { name: "Delete" })
      .click();
    await page
      .locator(".dialog-overlay")
      .getByRole("button", { name: "Confirm" })
      .click();
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
    await page.locator(".sub-tab", { hasText: "Venues" }).click();
    await page.locator(".item-card").first().click();
    await expect(page.locator(".editor-form")).toBeVisible();
    const options = page.locator(".editor-form select option");
    await expect(options.filter({ hasText: "pixelbrick" })).toHaveCount(1);
    await expect(options.filter({ hasText: "mover" })).toHaveCount(1);
  });
});

test.describe("GDTF Import", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await page.locator(".tab", { hasText: "Lighting" }).click();
    await page.getByRole("button", { name: "Enable Lighting" }).click();
    await page.locator(".sub-tab", { hasText: "Fixture Types" }).click();
  });

  test("import flows from file pick through mode choice to a report", async ({
    page,
  }) => {
    // Pick a file; the inspect response drives the mode picker.
    const chooser = page.getByTestId("import-gdtf");
    await expect(chooser).toBeVisible();
    await page.locator('input[type="file"]').setInputFiles({
      name: "pb15.gdtf",
      mimeType: "application/octet-stream",
      buffer: Buffer.from("not inspected by the mock"),
    });

    const picker = page.getByTestId("gdtf-mode-picker");
    await expect(picker).toBeVisible();
    await expect(picker).toContainText("PB15 PixelBrick");
    // Mode selection is the human input: both modes are offered.
    await expect(picker.locator("option")).toHaveCount(2);
    await picker.locator("select").selectOption("8: RGBS");

    await page.getByTestId("gdtf-import-confirm").click();

    // The report shows what was written and what the distiller skipped.
    const report = page.getByTestId("gdtf-report");
    await expect(report).toBeVisible();
    await expect(report).toContainText(
      "lighting/fixture_types/pb15_pixelbrick.fixture",
    );
    await expect(report).toContainText("channel 4: strobe");
    await expect(report).toContainText("virtual channel");
  });

  test("an unparseable upload surfaces the error", async ({ page }) => {
    await page.route("**/api/lighting/gdtf/inspect", async (route) => {
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
