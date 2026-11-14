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
import { Code } from "@connectrpc/connect";
import {
  ConfigService,
  LightingService,
  UploadPurpose,
} from "../../src/gen/player/v1/player_pb";
import {
  fixtureTypeListReply,
  gdtfImportReply,
  fixtureTypeSavedReply,
  savedTypeRequest,
  typeEntry,
} from "../support/lighting-rpc";
import type { FixtureTypeEntry } from "../../src/gen/rules/mtrack_rules";
import { card } from "../support/locators";
import { mockRpc } from "../support/rpc";

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
    const rpc = await mockRpc(page);
    rpc.unary(
      LightingService.method.listFixtureTypes,
      fixtureTypeListReply({
        fixture_types: { RGBW_Par: typeEntry("RGBW_Par") },
        errors: [
          {
            file: "broken.light",
            error: "expected channel definition at line 4",
          },
        ],
      }),
    );
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
    const entry = (name: string, gdtf: FixtureTypeEntry["gdtf"]) =>
      typeEntry(name, { extension: "fixture", referential: true, gdtf });
    const rpc = await mockRpc(page);
    rpc.unary(
      LightingService.method.listFixtureTypes,
      fixtureTypeListReply({
        fixture_types: {
          lost: entry("lost", null),
          unused: entry("unused", {
            fixture: "PB15 PixelBrick",
            manufacturer: "Astera LED Technology",
            modes: 1,
            beam: null,
            thumbnail: "abc/thumbnail.png",
            used_by: 0,
            in_use: [],
          }),
          busy: entry("busy", {
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
    ).toHaveAttribute("src", "/lighting-assets/abc/thumbnail.png");
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

  test("saving a .fixture type sends the DSL as text", async ({ page }) => {
    const rpc = await mockRpc(page);
    await card(page, "mover").click();
    await expect(page.getByTestId("ft-dsl")).toBeVisible();

    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    await expect
      .poll(() => rpc.calls(LightingService.method.saveFixtureType).length)
      .toBe(1);
    const sent = savedTypeRequest(
      rpc.calls(LightingService.method.saveFixtureType)[0],
    );
    expect(sent.name).toBe("mover");
    expect(sent.ext).toBe("fixture");
    // The text, not the channel-map form.
    expect(sent.form).toBeNull();
    expect(sent.text).toContain('channel "pan"');
  });

  test("a .light type can be opened as text and converted", async ({
    page,
  }) => {
    // The channel map is the default way in; this is the way out of it, and
    // the only path from a v1 file to the rich form.
    const rpc = await mockRpc(page);
    await card(page, "par").getByTestId("ft-edit-text").click();
    await expect(page.getByTestId("ft-dsl")).toHaveValue(/fixture_type "par"/);
    await expect(page.locator(".channel-row")).toHaveCount(0);

    // Saved back as it was found, unless the form is changed.
    const selector = page.getByTestId("ft-ext-select");
    await expect(selector).toHaveValue("light");
    await selector.selectOption("fixture");

    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    await expect
      .poll(() => rpc.calls(LightingService.method.saveFixtureType).length)
      .toBe(1);
    const sent = savedTypeRequest(
      rpc.calls(LightingService.method.saveFixtureType)[0],
    );
    expect(sent.name).toBe("par");
    expect(sent.ext).toBe("fixture");
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
    const rpc = await mockRpc(page);
    const nameField = page.getByTestId("ft-name-derived");
    await expect(nameField).toHaveValue("mover");
    await expect(nameField).toHaveJSProperty("readOnly", true);

    await page
      .getByTestId("ft-dsl")
      .fill('fixture_type "renamed" {\n  channel "pan" @ 1 fine 2\n}\n');
    await expect(nameField).toHaveValue("renamed");

    // The rename is one save under the declared name, from the old one:
    // the server moves or renames the file, so nothing is deleted after.
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    await expect
      .poll(() => rpc.calls(LightingService.method.saveFixtureType).length)
      .toBe(1);
    const put = savedTypeRequest(
      rpc.calls(LightingService.method.saveFixtureType)[0],
    );
    expect(put.name).toBe("renamed");
    expect(put.from).toBe("mover");
    // The rename retyped the venue lines naming it, and says so, as the
    // fixture settings' rename does.
    await expect(page.locator(".save-msg").first()).toContainText(
      "Renamed in 3 venue lines: built-in (2), club (1).",
    );
    await page.waitForTimeout(300);
    expect(rpc.calls(LightingService.method.deleteFixtureType)).toHaveLength(0);
  });

  test("Define a fixture by hand offers both forms", async ({ page }) => {
    await page
      .getByRole("button", { name: "Define a fixture by hand" })
      .click();
    await expect(page.getByTestId("new-ft-choice")).toBeVisible();
    await page
      .getByRole("button", { name: "Channel map", exact: true })
      .click();
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
    // The form opens when the address changes, a moment after the click: an
    // immediate count can read zero rows and then find them all plus one.
    await expect(page.locator(".channel-row").first()).toBeVisible();
    const initialCount = await page.locator(".channel-row").count();
    await page.getByRole("button", { name: "Add Channel" }).click();
    await expect(page.locator(".channel-row")).toHaveCount(initialCount + 1);
  });

  test("removing a channel row decreases count", async ({ page }) => {
    await card(page, "par").click();
    // The form opens when the address changes, a moment after the click: an
    // immediate count can read zero rows and then find them all plus one.
    await expect(page.locator(".channel-row").first()).toBeVisible();
    const initialCount = await page.locator(".channel-row").count();
    // Click the first X button in a channel row.
    await page.locator(".channel-row").first().locator(".btn-danger").click();
    await expect(page.locator(".channel-row")).toHaveCount(initialCount - 1);
  });

  test("saving a fixture type calls SaveFixtureType", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.unary(
      LightingService.method.saveFixtureType,
      fixtureTypeSavedReply({ name: "par" }),
    );

    await card(page, "par").click();
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    await expect
      .poll(() => rpc.calls(LightingService.method.saveFixtureType).length)
      .toBe(1);
    // A channel-map type is saved from the form, as its channels.
    const sent = savedTypeRequest(
      rpc.calls(LightingService.method.saveFixtureType)[0],
    );
    expect(sent.name).toBe("par");
    expect(sent.form?.channels).toMatchObject({ red: 1, dimmer: 4 });
  });

  test("deleting a fixture from a GDTF names the venues that use it, then DELETEs", async ({
    page,
  }) => {
    const rpc = await mockRpc(page);
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
    expect(rpc.calls(LightingService.method.deleteFixtureType)).toHaveLength(0);

    await card(page, "pixelbrick")
      .getByRole("button", { name: "Delete" })
      .click();
    await dialog.getByRole("button", { name: "Confirm" }).click();
    await expect
      .poll(() => rpc.calls(LightingService.method.deleteFixtureType))
      .toMatchObject([{ name: "pixelbrick" }]);
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

  const GDTF_FILE = {
    name: "pb15.gdtf",
    mimeType: "application/octet-stream",
    buffer: Buffer.from("not read by the mock"),
  };

  test("choosing a file imports it in one step and opens the fixture", async ({
    page,
  }) => {
    const rpc = await mockRpc(page);
    const puts: string[] = [];
    page.on("request", (req) => {
      if (req.method() === "PUT" && req.url().includes("/uploads/")) {
        puts.push(req.url());
      }
    });
    await page.locator('input[type="file"]').setInputFiles(GDTF_FILE);
    const report = page.getByTestId("gdtf-report");
    await expect(report).toHaveText(
      "Imported PB15 PixelBrick (Astera LED Technology) — 2 modes.",
    );
    // One announcement of the file, its bytes sent once to the path that
    // answered, and one import, with no name.
    const begun = rpc.calls(ConfigService.method.beginUpload);
    expect(begun).toHaveLength(1);
    expect(begun[0].purpose).toBe(UploadPurpose.GDTF_IMPORT);
    expect(begun[0].fileName).toBe("pb15.gdtf");
    expect(begun[0].size).toBe(BigInt(GDTF_FILE.buffer.length));
    expect(puts).toHaveLength(1);
    const imports = rpc.calls(LightingService.method.importGdtf);
    expect(imports).toHaveLength(1);
    expect(imports[0].name).toBeUndefined();
    expect(puts[0]).toContain(`/uploads/${imports[0].uploadId}`);
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
      const rpc = await mockRpc(page);
      rpc.unary(
        LightingService.method.importGdtf,
        gdtfImportReply({
          type_name: "pixelbrick",
          fixture: "PB15 PixelBrick",
          manufacturer: "Astera LED Technology",
          modes: 2,
          archive: "lighting/library/pb15.gdtf",
          already_imported: false,
          renamed_from: null,
          refused_modes: [],
          fixture_file: null,
          ...answer,
        }),
      );
      await page.locator('input[type="file"]').setInputFiles(GDTF_FILE);
      await expect(page.getByTestId("gdtf-report")).toHaveText(said);
      await expect(page.getByTestId("ft-title")).toHaveText("pixelbrick");
    });
  }

  test("an unparseable upload surfaces the error", async ({ page }) => {
    const rpc = await mockRpc(page);
    rpc.error(
      LightingService.method.importGdtf,
      Code.InvalidArgument,
      "Not a parseable GDTF: not a zip",
    );
    await page.locator('input[type="file"]').setInputFiles({
      name: "junk.gdtf",
      mimeType: "application/octet-stream",
      buffer: Buffer.from("junk"),
    });
    await expect(page.getByTestId("gdtf-error")).toContainText("not a zip");
  });

  test("a file the server will not take is refused before its bytes are sent", async ({
    page,
  }) => {
    const rpc = await mockRpc(page);
    rpc.error(
      ConfigService.method.beginUpload,
      Code.InvalidArgument,
      "No file uploaded",
    );
    let puts = 0;
    page.on("request", (req) => {
      if (req.method() === "PUT" && req.url().includes("/uploads/")) puts += 1;
    });
    await page.locator('input[type="file"]').setInputFiles(GDTF_FILE);
    await expect(page.getByTestId("gdtf-error")).toContainText("No file");
    expect(puts).toBe(0);
    expect(rpc.calls(LightingService.method.importGdtf)).toHaveLength(0);
  });

  test("bytes the server refuses are an error, and nothing is imported", async ({
    page,
  }) => {
    const rpc = await mockRpc(page);
    await page.route("**/uploads/*", (route) =>
      route.fulfill({
        status: 413,
        contentType: "application/json",
        body: JSON.stringify({ error: "The body is longer than announced" }),
      }),
    );
    await page.locator('input[type="file"]').setInputFiles(GDTF_FILE);
    await expect(page.getByTestId("gdtf-error")).toContainText("longer");
    expect(rpc.calls(LightingService.method.importGdtf)).toHaveLength(0);
  });
});
