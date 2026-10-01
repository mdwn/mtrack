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

import { type Page } from "@playwright/test";
import { api, expect, synthGdtf, synthMvr, test } from "./harness";

// The Lighting area against the real binary: each journey does what a user
// does, then checks what persisted — through a fresh page load, the API and
// the file on disk. Mock specs cannot see a server that disagrees with
// itself (the importer naming a file one way and the API looking for it
// another); these can.

/** The synthetic GDTF's modes: "8: RGBS" is 4 addresses, "Mover 16bit" 5. */
const BRICK = `fixture_type "Brick" from gdtf("lighting/library/synth.gdtf", mode "8: RGBS") {
}
`;
const PAR = `fixture_type "Par" {
  channels: 3
  channel_map: { "red": 1, "green": 2, "blue": 3 }
}
`;

/** A venue of bricks, placed, made current by the journey config. */
function house(lines: string[]): string {
  return `# The house rig — comments survive every save.\nvenue "house" {\n${lines
    .map((l) => `  ${l}\n`)
    .join("")}}\n`;
}

const withBricks = (venue: string) => ({
  "lighting/library/synth.gdtf": synthGdtf(),
  "lighting/fixture_types/brick.fixture": BRICK,
  "lighting/venues/house.venue": venue,
});

function card(page: Page, name: string) {
  return page.locator(".item-card").filter({
    has: page.locator(".item-name", { hasText: new RegExp(`^${name}$`) }),
  });
}

const confirmDialog = (page: Page) => page.locator(".dialog-overlay");

/** The line a fixture has in a venue file. */
function lineOf(text: string, fixture: string): string {
  const line = text.split("\n").find((l) => l.includes(`fixture "${fixture}"`));
  if (!line) throw new Error(`no line for ${fixture} in:\n${text}`);
  return line;
}

/** Opens the Venues page and selects one fixture in the inspector. */
async function inspect(page: Page, fixture: string) {
  await page.goto("/#/lighting/venues");
  await expect(page.locator(".inspector")).toBeVisible();
  await page.locator(".inspector").getByLabel(fixture, { exact: true }).check();
  await expect(page.locator("#insp-name")).toHaveValue(fixture);
}

test.describe("Fixture types", () => {
  test.use({ files: { "lighting/venues/house.light": house([]) } });

  test("a GDTF imported through the UI is listed, opens by its hyphenated name, and shows its modes", async ({
    page,
    project,
  }) => {
    await page.goto("/#/lighting/fixtures");
    await page.locator('input[type="file"]').setInputFiles({
      name: "synth.gdtf",
      mimeType: "application/octet-stream",
      buffer: synthGdtf(),
    });
    await expect(page.getByTestId("gdtf-mode-picker")).toBeVisible();
    await page.getByTestId("gdtf-type-name").fill("Synth-Brick");
    await page.getByTestId("gdtf-import-confirm").click();
    await expect(page.getByTestId("gdtf-report")).toBeVisible();

    // The importer names the file its own way; the API must still find it.
    expect(project.exists("lighting/fixture_types/synth_brick.fixture")).toBe(
      true,
    );
    const listed = await api<{ fixture_types: Record<string, unknown> }>(
      project,
      "/lighting/fixture-types",
    );
    expect(Object.keys(listed.fixture_types)).toContain("Synth-Brick");
    await api(project, "/lighting/fixture-types/Synth-Brick");

    // A fresh load, then open it: this is where "Fixture type not found"
    // used to be.
    await page.goto("/#/lighting/fixtures");
    await card(page, "Synth-Brick").click();
    await expect(page.getByTestId("ft-title")).toHaveText("Synth-Brick");
    await expect(page.getByTestId("ft-details-error")).toHaveCount(0);
    const modes = page.getByTestId("ft-details-modes");
    await expect(modes.locator('[data-mode="8: RGBS"]')).toBeVisible();
    await expect(modes.locator('[data-mode="Mover 16bit"]')).toBeVisible();
  });

  test("a fixture type and a venue can be deleted", async ({
    page,
    project,
  }) => {
    project.write("lighting/fixture_types/par.light", PAR);
    project.write(
      "lighting/venues/spare.light",
      'venue "spare" {\n  fixture "P1" Par @ 1:1\n}\n',
    );
    await page.goto("/#/lighting/fixtures");
    await card(page, "Par").getByRole("button", { name: "Delete" }).click();
    await confirmDialog(page).getByRole("button", { name: "Confirm" }).click();
    await expect(card(page, "Par")).toHaveCount(0);
    expect(project.exists("lighting/fixture_types/par.light")).toBe(false);
    const res = await fetch(`${project.url}/api/lighting/fixture-types/Par`);
    expect(res.status).toBe(404);

    await page.goto("/#/lighting/venues");
    await card(page, "spare").getByRole("button", { name: "Delete" }).click();
    await confirmDialog(page).getByRole("button", { name: "Confirm" }).click();
    await expect(card(page, "spare")).toHaveCount(0);
    expect(project.exists("lighting/venues/spare.light")).toBe(false);
  });
});

test.describe("A new venue", () => {
  test.use({
    files: {
      "lighting/fixture_types/par.light": PAR,
      "lighting/venues/house.light": house([]),
    },
  });

  test("Add Fixture three times with the defaults saves three fixtures at continuing addresses", async ({
    page,
    project,
  }) => {
    await page.goto("/#/lighting/venues");
    await page.getByRole("button", { name: "New Venue" }).click();
    await page.locator("#venue-name").fill("club");
    for (let i = 0; i < 3; i++)
      await page.getByRole("button", { name: "Add Fixture" }).click();
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    await expect(page.locator(".editor-form")).toHaveCount(0);

    const got = await api<{
      venue: {
        fixtures: Record<string, { universe: number; start_channel: number }>;
      };
    }>(project, "/lighting/venues/club");
    const patch = Object.entries(got.venue.fixtures)
      .map(([name, f]) => [name, f.universe, f.start_channel])
      .sort();
    expect(patch).toEqual([
      ["Fixture 1", 1, 1],
      ["Fixture 2", 1, 4],
      ["Fixture 3", 1, 7],
    ]);
    const file = project.read("lighting/venues/club.light");
    expect(file).toContain('fixture "Fixture 3" Par @ 1:7');
  });
});

test.describe("A fixture's mode", () => {
  test.use({
    files: withBricks(
      house([
        'fixture "B1" Brick @ 1:1 position (-2, 2, 3)',
        'fixture "B2" Brick @ 1:10 position (0, 2, 3)',
        'fixture "B3" Brick @ 1:20 position (2, 2, 3)',
      ]),
    ),
  });

  test("set in the inspector, it is on the line and survives the venue form and a drag", async ({
    page,
    project,
  }) => {
    await inspect(page, "B1");
    await page.getByTestId("insp-mode").selectOption("Mover 16bit");
    await expect
      .poll(() => lineOf(project.read("lighting/venues/house.venue"), "B1"))
      .toContain('mode "Mover 16bit"');

    // An unrelated edit from the venue form.
    await page.goto("/#/lighting/venues");
    await card(page, "house").locator('[data-testid^="venue-edit-"]').click();
    const row = page.getByTestId("venue-fixture-row").nth(1);
    const tag = row.locator(".tag-text-input");
    await tag.fill("back");
    await tag.press("Enter");
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    await expect(page.locator(".editor-form")).toHaveCount(0);
    let file = project.read("lighting/venues/house.venue");
    expect(file).toContain("The house rig");
    expect(lineOf(file, "B1")).toContain('mode "Mover 16bit"');

    // A drag on the plot (select first: the inspector grows and moves it).
    await page.goto("/#/lighting/venues");
    const canvas = page.locator(".stage-card__viewport canvas");
    await expect(canvas).toHaveAttribute("data-positions", /B3/);
    const where = async () =>
      JSON.parse((await canvas.getAttribute("data-positions"))!).B3 as {
        x: number;
        y: number;
      };
    await canvas.scrollIntoViewIfNeeded();
    const first = await where();
    await canvas.click({ position: first });
    await page.waitForTimeout(300);
    const at = await where();
    const box = (await canvas.boundingBox())!;
    await page.mouse.move(box.x + at.x, box.y + at.y);
    await page.mouse.down();
    await page.mouse.move(box.x + at.x + 20, box.y + at.y - 10);
    await page.mouse.move(box.x + at.x + 60, box.y + at.y + 30);
    await page.mouse.up();
    await expect
      .poll(() => lineOf(project.read("lighting/venues/house.venue"), "B3"))
      .not.toContain("position (2, 2, 3)");
    file = project.read("lighting/venues/house.venue");
    expect(lineOf(file, "B1")).toContain('mode "Mover 16bit"');

    // And from the server's own reading of it.
    const got = await api<{
      venue: { fixtures: Record<string, { mode?: string }> };
    }>(project, "/lighting/venues/house");
    expect(got.venue.fixtures.B1.mode).toBe("Mover 16bit");
  });
});

test.describe("A colliding mode", () => {
  test.use({
    files: withBricks(
      house([
        'fixture "B1" Brick @ 1:1 position (-2, 2, 3)',
        'fixture "B2" Brick @ 1:5 position (0, 2, 3)',
      ]),
    ),
  });

  test("is refused, and nothing changes on disk", async ({ page, project }) => {
    const before = project.read("lighting/venues/house.venue");
    await inspect(page, "B1");
    await page.getByTestId("insp-mode").selectOption("Mover 16bit");
    await expect(page.getByTestId("insp-patch-msg")).toContainText(
      "Mover 16bit needs addresses 1 to 5, and B2 already uses some of them",
    );
    await expect(page.getByTestId("insp-mode")).toHaveValue("");
    await page.waitForTimeout(500);
    expect(project.read("lighting/venues/house.venue")).toBe(before);
  });
});

test.describe("The fixture's settings", () => {
  test.use({
    files: withBricks(
      house([
        'fixture "B1" Brick @ 1:1 position (-2, 2, 3)  # front left',
        'fixture "B2" Brick @ 1:100 position (0, 2, 3)',
      ]),
    ),
  });

  test("a rename rewrites the venue lines that use it", async ({
    page,
    project,
  }) => {
    await page.goto("/#/lighting/fixtures");
    await card(page, "Brick").click();
    await page.getByTestId("ft-set-name").fill("Brick-Two");
    await page.getByTestId("ft-set-save").click();
    await expect(confirmDialog(page)).toContainText("rewrites 2 venue lines");
    await confirmDialog(page).getByRole("button", { name: "Save" }).click();
    await expect(page.getByTestId("ft-set-msg")).toContainText("Renamed");

    const file = project.read("lighting/venues/house.venue");
    expect(lineOf(file, "B1")).toContain("Brick-Two @ 1:1");
    expect(lineOf(file, "B1")).toContain("# front left");
    expect(file).toContain("The house rig");
    // The type keeps its file, under its new name.
    expect(project.read("lighting/fixture_types/brick.fixture")).toContain(
      'fixture_type "Brick-Two"',
    );
    await api(project, "/lighting/fixture-types/Brick-Two");
    const status = await api<{
      hardware: { lighting_venue: { status: string } };
    }>(project, "/status");
    expect(status.hardware.lighting_venue.status).toBe("ok");
  });

  test("a new default mode, confirmed, changes the fixtures that take it", async ({
    page,
    project,
  }) => {
    await page.goto("/#/lighting/fixtures");
    await card(page, "Brick").click();
    await page.getByTestId("ft-set-default").selectOption("Mover 16bit");
    await page.getByTestId("ft-set-save").click();
    await expect(confirmDialog(page)).toContainText(
      "2 fixtures in house use the default and will change to Mover 16bit.",
    );
    await confirmDialog(page).getByRole("button", { name: "Save" }).click();
    await expect(page.getByTestId("ft-set-msg")).toContainText("Saved");

    expect(project.read("lighting/fixture_types/brick.fixture")).toContain(
      'mode "Mover 16bit"',
    );
    const patch = await api<{
      spans: { fixture: string; footprint: number }[];
    }>(project, "/lighting/venues/house/patch");
    expect(patch.spans.map((s) => s.footprint)).toEqual([5, 5]);
  });
});

test.describe("MVR import", () => {
  test.use({ files: { "lighting/venues/house.light": house([]) } });

  test("the test MVR seeds a venue and a type that both open", async ({
    page,
    project,
  }) => {
    await page.goto("/#/lighting/import");
    await page.getByTestId("mvr-file").setInputFiles({
      name: "Kellys.mvr",
      mimeType: "application/octet-stream",
      buffer: synthMvr(),
    });
    await page.getByTestId("mvr-continue").click();
    await page.getByTestId("mvr-to-review").click();
    await page.getByTestId("mvr-do-import").click();
    await expect(page.getByTestId("mvr-done")).toBeVisible();

    const venues = await api<{ venues: Record<string, unknown> }>(
      project,
      "/lighting/venues",
    );
    const name = Object.keys(venues.venues).find((v) => v !== "house")!;
    expect(name).toBeTruthy();
    const types = await api<{ fixture_types: Record<string, unknown> }>(
      project,
      "/lighting/fixture-types",
    );
    const typeName = Object.keys(types.fixture_types)[0];
    expect(typeName).toBeTruthy();

    await page.goto("/#/lighting/fixtures");
    await card(page, typeName).click();
    await expect(page.getByTestId("ft-details-modes")).toBeVisible();
    await expect(page.getByTestId("ft-details-error")).toHaveCount(0);
    await page.goto("/#/lighting/venues");
    await card(page, name).locator('[data-testid^="venue-edit-"]').click();
    await expect(page.getByTestId("venue-fixture-row")).toHaveCount(2);
  });
});

test.describe("A venue broken by hand", () => {
  test.use({
    files: withBricks(house(['fixture "B1" Brick @ 1:1 position (0, 2, 3)'])),
  });

  test("shows the banner after a cold boot, and a fix through the UI clears it", async ({
    page,
    project,
  }) => {
    project.write(
      "lighting/venues/house.venue",
      house(['fixture "B1" Brick mode "Nope" @ 1:1 position (0, 2, 3)']),
    );
    await project.restart();
    await page.goto("/#/");
    const banner = page.getByTestId("venue-failed-banner");
    await expect(banner).toBeVisible({ timeout: 15000 });
    await expect(banner).toContainText('Venue "house" did not load');
    await expect(banner).toContainText('"B1"');

    // Fixed where a user would fix it: the mode select, back to the default.
    await inspect(page, "B1");
    await page.getByTestId("insp-mode").selectOption("");
    await expect(banner).toHaveCount(0, { timeout: 15000 });
    expect(
      lineOf(project.read("lighting/venues/house.venue"), "B1"),
    ).not.toContain("mode");
  });
});
