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

// The Venues plan's selection, arrange and aim tools (design §10). Every test
// routes its own venue file, fixture types and metadata with `page.route` and
// a per-test venue name, so nothing depends on state another test left in the
// shared mock server.

type Vec3 = [number, number, number];
interface FixtureBody {
  name: string;
  fixture_type: string;
  universe: number;
  start_channel: number;
  tags: string[];
  position: Vec3 | null;
  rotation: Vec3 | null;
  beam_angle?: number | null;
}
interface VenueBody {
  fixtures: FixtureBody[];
  focus_points: Record<string, Vec3>;
  source: { mvr: string; origin: Vec3 } | null;
}

/** The house rig, as measured: two bricks a side, four across the front. */
const HOUSE: [string, Vec3][] = [
  ["Brick1", [-5.3, 3.87, 0]],
  ["Brick2", [-5.39, 1.325, 0]],
  ["Brick3", [-3.69, 0.01, 0]],
  ["Brick4", [-1.0, 0.03, 0]],
  ["Brick5", [1.5, 0.0, 0]],
  ["Brick6", [3.69, 0.02, 0]],
  ["Brick7", [6.8, 1.325, 0]],
  ["Brick8", [6.91, 3.87, 0]],
];

let counter = 0;

async function sendWs(page: Page, wsId: string, msg: object) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

function houseVenue(extra: FixtureBody[] = []): VenueBody {
  return {
    fixtures: [
      ...HOUSE.map(([name, position], i) => ({
        name,
        fixture_type: "brick",
        universe: 1,
        start_channel: 1 + i * 4,
        tags: ["wash", i < 2 ? "left" : "front"],
        position,
        rotation: [110, 0, 0] as Vec3,
      })),
      ...extra,
    ],
    // A focus point and an MVR provenance, which no operation may lose.
    focus_points: { center: [0.7, 1.9, 1.5] },
    source: { mvr: "imports/rig.mvr", origin: [1, 2, 3] },
  };
}

const MOVER: FixtureBody = {
  name: "Mover1",
  fixture_type: "mover",
  universe: 1,
  start_channel: 100,
  tags: ["mover"],
  position: [2, 4, 5],
  rotation: [0, 0, 180],
};

const FIXTURE_TYPES = {
  brick: channels({ red: 1, green: 2, blue: 3 }),
  mover: channels({ pan: 1, tilt: 3, dimmer: 5 }),
  // A GDTF-referential type lists no channels of its own.
  gdtfmover: { ...channels({}), referential: true },
};

function channels(ch: Record<string, number>) {
  return {
    fixture_type: {
      name: "t",
      channels: ch,
      max_strobe_frequency: null,
      min_strobe_frequency: null,
      strobe_dmx_offset: null,
    },
    file: "t.fixture",
    extension: "fixture",
    referential: false,
    rich: false,
  };
}

/** Serves the venue file, remembering each save so a second operation reads
 *  the first. Every PUT is collected. */
async function routeVenue(page: Page, name: string, initial: VenueBody) {
  let current = structuredClone(initial);
  const puts: VenueBody[] = [];
  await page.route(
    new RegExp(`/api/lighting/venues/${name}(\\?.*)?$`),
    async (route) => {
      const request = route.request();
      if (request.method() === "PUT") {
        current = request.postDataJSON();
        puts.push(structuredClone(current));
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "saved" }),
        });
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          venue: {
            name,
            fixtures: Object.fromEntries(
              current.fixtures.map((f) => [f.name, f]),
            ),
            focus_points: current.focus_points,
            source: current.source,
          },
          dsl: "",
        }),
      });
    },
  );
  await page.route("**/api/lighting/fixture-types*", async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    if (/fixture-types\/[^?]/.test(route.request().url()))
      return route.fallback();
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ fixture_types: FIXTURE_TYPES, errors: [] }),
    });
  });
  return { puts, current: () => current };
}

/** Opens the Venues page with `venue` drawn on the plan. */
async function open(page: Page, venue: VenueBody = houseVenue()) {
  const name = `l4-${test.info().parallelIndex}-${++counter}-${Date.now()}`;
  const wsId = name;
  const served = await routeVenue(page, name, venue);
  await page.goto(`/?wsId=${wsId}#/lighting/venues`);
  // The venue list above the plan loads late and pushes the plan down;
  // wait for it so clicks land where the plan is drawn.
  await expect(page.locator(".item-card").first()).toBeVisible();
  await sendWs(page, wsId, {
    type: "metadata",
    fixtures: Object.fromEntries(
      venue.fixtures.map((f) => [
        f.name,
        {
          tags: f.tags,
          type: f.fixture_type,
          capabilities: f.fixture_type.endsWith("mover")
            ? ["color", "pan_tilt"]
            : ["color"],
          position: f.position,
          rotation: f.rotation,
          beam_angle: f.beam_angle ?? null,
        },
      ]),
    ),
    venue: { name, dir: null, focus_points: venue.focus_points },
  });
  await expect(page.locator(".inspector")).toBeVisible();
  await expect
    .poll(async () => (await positions(page))["Brick1"] ?? null)
    .not.toBeNull();
  return { ...served, name, wsId };
}

const canvas = (page: Page) => page.locator(".stage-card__viewport canvas");

async function positions(page: Page) {
  const raw = await canvas(page).getAttribute("data-positions");
  return raw
    ? (JSON.parse(raw) as Record<string, { x: number; y: number }>)
    : {};
}

async function clickFixture(page: Page, name: string, shift = false) {
  const at = (await positions(page))[name];
  await canvas(page).click({
    position: { x: at.x, y: at.y },
    modifiers: shift ? ["Shift"] : [],
  });
}

/** Drags a box from just outside `from`'s bounding corner to just outside
 *  `to`'s, over empty deck. */
async function marquee(page: Page, names: string[], shift = false) {
  const all = await positions(page);
  const pts = names.map((n) => all[n]);
  await canvas(page).scrollIntoViewIfNeeded();
  const box = (await canvas(page).boundingBox())!;
  const x0 = Math.min(...pts.map((p) => p.x)) - 22;
  const y0 = Math.min(...pts.map((p) => p.y)) - 22;
  const x1 = Math.max(...pts.map((p) => p.x)) + 22;
  const y1 = Math.max(...pts.map((p) => p.y)) + 22;
  if (shift) await page.keyboard.down("Shift");
  await page.mouse.move(box.x + x0, box.y + y0);
  await page.mouse.down();
  await page.mouse.move(box.x + (x0 + x1) / 2, box.y + (y0 + y1) / 2);
  await page.mouse.move(box.x + x1, box.y + y1);
  await page.mouse.up();
  if (shift) await page.keyboard.up("Shift");
}

const inspector = (page: Page) => page.locator(".inspector");
const tick = (page: Page, name: string) =>
  inspector(page).getByLabel(name, { exact: true });

async function selectByList(page: Page, names: string[]) {
  for (const n of names) await tick(page, n).check();
}

async function expectSelected(page: Page, names: string[]) {
  for (const [n] of HOUSE.concat([["Mover1", [0, 0, 0]]])) {
    if (await tick(page, n).count()) {
      if (names.includes(n)) await expect(tick(page, n)).toBeChecked();
      else await expect(tick(page, n)).not.toBeChecked();
    }
  }
}

/** The fixture in a save, by name. */
function saved(body: VenueBody, name: string) {
  return body.fixtures.find((f) => f.name === name)!;
}

test.describe("Venues: selection", () => {
  test("click selects, shift extends, and the list mirrors", async ({
    page,
  }) => {
    await open(page);
    await expect(inspector(page)).toContainText("Nothing selected");

    await clickFixture(page, "Brick1");
    await expect(inspector(page)).toContainText("1 selected");
    await expectSelected(page, ["Brick1"]);

    await clickFixture(page, "Brick4", true);
    await expect(inspector(page)).toContainText("2 selected");
    await expectSelected(page, ["Brick1", "Brick4"]);

    // Shift-click on a member takes it back out.
    await clickFixture(page, "Brick1", true);
    await expectSelected(page, ["Brick4"]);

    // A plain click replaces the selection.
    await clickFixture(page, "Brick6");
    await expectSelected(page, ["Brick6"]);
  });

  test("the list drives the selection too", async ({ page }) => {
    await open(page);
    await selectByList(page, ["Brick2", "Brick3", "Brick5"]);
    await expect(inspector(page)).toContainText("3 selected");
    await tick(page, "Brick3").uncheck();
    await expectSelected(page, ["Brick2", "Brick5"]);
  });

  test("dragging a box on the empty deck selects what it encloses", async ({
    page,
  }) => {
    await open(page);
    await marquee(page, ["Brick1", "Brick2"]);
    await expectSelected(page, ["Brick1", "Brick2"]);
    await expect(inspector(page)).toContainText("2 selected");

    // Shift adds a second box to the first.
    await marquee(page, ["Brick7", "Brick8"], true);
    await expectSelected(page, ["Brick1", "Brick2", "Brick7", "Brick8"]);

    // A plain box replaces.
    await marquee(page, ["Brick3", "Brick4"]);
    await expectSelected(page, ["Brick3", "Brick4"]);
  });

  test("Escape clears the selection", async ({ page }) => {
    const { puts } = await open(page);
    await clickFixture(page, "Brick2");
    await clickFixture(page, "Brick3", true);
    await expect(inspector(page)).toContainText("2 selected");
    await page.keyboard.press("Escape");
    await expect(inspector(page)).toContainText("Nothing selected");
    await expectSelected(page, []);
    expect(puts).toHaveLength(0);
  });

  test("a click that does not move saves nothing, and a drag still saves", async ({
    page,
  }) => {
    const { puts } = await open(page);
    await clickFixture(page, "Brick3");
    await expect(inspector(page)).toContainText("1 selected");
    await page.waitForTimeout(300);
    expect(puts).toHaveLength(0);

    const at = (await positions(page))["Brick3"];
    await canvas(page).scrollIntoViewIfNeeded();
    const box = (await canvas(page).boundingBox())!;
    await page.mouse.move(box.x + at.x, box.y + at.y);
    await page.mouse.down();
    await page.mouse.move(box.x + at.x + 20, box.y + at.y - 30);
    await page.mouse.up();
    await expect.poll(() => puts.length).toBe(1);
    const moved = saved(puts[0], "Brick3").position!;
    expect(moved[0]).toBeGreaterThan(-3.69);
    expect(moved[1]).toBeGreaterThan(0.01);
    expect(moved[2]).toBe(0);
  });
});

test.describe("Venues: arrange", () => {
  test("align on a line puts a side at its mean x", async ({ page }) => {
    const { puts } = await open(page);
    await selectByList(page, ["Brick1", "Brick2"]);
    await inspector(page)
      .getByRole("button", { name: "Align on a line" })
      .click();
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Brick1").position).toEqual([-5.345, 3.87, 0]);
    expect(saved(puts[0], "Brick2").position).toEqual([-5.345, 1.325, 0]);
    // The rest of the rig is where it was.
    expect(saved(puts[0], "Brick3").position).toEqual([-3.69, 0.01, 0]);
  });

  test("align works on the right side and on a row", async ({ page }) => {
    const { puts } = await open(page);
    await selectByList(page, ["Brick7", "Brick8"]);
    await inspector(page)
      .getByRole("button", { name: "Align on a line" })
      .click();
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Brick7").position).toEqual([6.855, 1.325, 0]);
    expect(saved(puts[0], "Brick8").position).toEqual([6.855, 3.87, 0]);

    await page.keyboard.press("Escape");
    await selectByList(page, ["Brick3", "Brick4", "Brick5", "Brick6"]);
    await inspector(page)
      .getByRole("button", { name: "Align on a line" })
      .click();
    await expect.poll(() => puts.length).toBe(2);
    for (const n of ["Brick3", "Brick4", "Brick5", "Brick6"]) {
      expect(saved(puts[1], n).position![1]).toBe(0.015);
    }
  });

  test("space evenly spreads the front row 2.46 m apart", async ({ page }) => {
    const { puts } = await open(page);
    await marquee(page, ["Brick3", "Brick6"]);
    await expect(inspector(page)).toContainText("4 selected");
    await inspector(page).getByRole("button", { name: "Space evenly" }).click();
    await expect.poll(() => puts.length).toBe(1);
    expect(
      ["Brick3", "Brick4", "Brick5", "Brick6"].map(
        (n) => saved(puts[0], n).position![0],
      ),
    ).toEqual([-3.69, -1.23, 1.23, 3.69]);
  });

  test("mirror across centre negates x", async ({ page }) => {
    const { puts } = await open(page);
    await selectByList(page, ["Brick1", "Brick7"]);
    await inspector(page)
      .getByRole("button", { name: "Mirror across centre" })
      .click();
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Brick1").position).toEqual([5.3, 3.87, 0]);
    expect(saved(puts[0], "Brick7").position).toEqual([-6.8, 1.325, 0]);
  });

  test("mirror across centre mirrors the aim too", async ({ page }) => {
    const venue = houseVenue();
    saved(venue, "Brick1").rotation = [110, 0, -90];
    saved(venue, "Brick7").rotation = [110, 0, 90];
    saved(venue, "Brick3").rotation = null;
    const { puts } = await open(page, venue);
    await selectByList(page, ["Brick1", "Brick7", "Brick3"]);
    await inspector(page)
      .getByRole("button", { name: "Mirror across centre" })
      .click();
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Brick1").rotation).toEqual([110, 0, 90]);
    expect(saved(puts[0], "Brick7").rotation).toEqual([110, 0, -90]);
    // No rotation before, none after.
    expect(saved(puts[0], "Brick3").rotation).toBeNull();
  });

  test("arrange needs two, and space evenly three", async ({ page }) => {
    await open(page);
    await clickFixture(page, "Brick1");
    await expect(
      inspector(page).getByRole("button", { name: "Align on a line" }),
    ).toHaveCount(0);
    await clickFixture(page, "Brick2", true);
    await expect(
      inspector(page).getByRole("button", { name: "Align on a line" }),
    ).toBeEnabled();
    await expect(
      inspector(page).getByRole("button", { name: "Space evenly" }),
    ).toBeDisabled();
  });

  test("every save keeps focus points, provenance and the other fields", async ({
    page,
  }) => {
    const initial = houseVenue();
    const { puts } = await open(page, initial);
    await selectByList(page, ["Brick1", "Brick2"]);
    await inspector(page)
      .getByRole("button", { name: "Align on a line" })
      .click();
    await expect.poll(() => puts.length).toBe(1);
    await inspector(page)
      .getByRole("button", { name: "Mirror across centre" })
      .click();
    await expect.poll(() => puts.length).toBe(2);
    await inspector(page)
      .getByRole("button", { name: "Face this way" })
      .click();
    await expect.poll(() => puts.length).toBe(3);

    for (const body of puts) {
      expect(body.focus_points).toEqual(initial.focus_points);
      expect(body.source).toEqual(initial.source);
      expect(body.fixtures.map((f) => f.name)).toEqual(
        initial.fixtures.map((f) => f.name),
      );
      for (const f of body.fixtures) {
        const before = saved(initial, f.name);
        expect(f.fixture_type).toBe(before.fixture_type);
        expect(f.universe).toBe(before.universe);
        expect(f.start_channel).toBe(before.start_channel);
        expect(f.tags).toEqual(before.tags);
      }
    }
  });
});

test.describe("Venues: nudge", () => {
  test("arrow keys nudge 0.1 m, shift 1 m, while the plan has focus", async ({
    page,
  }) => {
    const { puts } = await open(page);
    await clickFixture(page, "Brick3");
    await expect(canvas(page)).toBeFocused();
    await page.keyboard.press("ArrowRight");
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Brick3").position).toEqual([-3.59, 0.01, 0]);

    await page.keyboard.press("Shift+ArrowUp");
    await expect.poll(() => puts.length).toBe(2);
    expect(saved(puts[1], "Brick3").position).toEqual([-3.59, 1.01, 0]);
    // The others did not move.
    expect(saved(puts[1], "Brick4").position).toEqual([-1.0, 0.03, 0]);
  });

  test("a burst of presses is one save", async ({ page }) => {
    const { puts } = await open(page);
    await selectByList(page, ["Brick3", "Brick4"]);
    await canvas(page).focus();
    for (let i = 0; i < 3; i++) await page.keyboard.press("ArrowLeft");
    await expect.poll(() => puts.length).toBe(1);
    await page.waitForTimeout(500);
    expect(puts).toHaveLength(1);
    expect(saved(puts[0], "Brick3").position![0]).toBe(-3.99);
    expect(saved(puts[0], "Brick4").position![0]).toBe(-1.3);
  });

  test("arrow keys do nothing when the plan does not have focus", async ({
    page,
  }) => {
    const { puts } = await open(page);
    await selectByList(page, ["Brick3"]);
    await expect(tick(page, "Brick3")).toBeFocused();
    await page.keyboard.press("ArrowRight");
    await page.waitForTimeout(500);
    expect(puts).toHaveLength(0);
  });
});

test.describe("Venues: aim", () => {
  test("face a direction writes (90 + tilt, 0, bearing)", async ({ page }) => {
    const { puts } = await open(page);
    await selectByList(page, ["Brick1", "Brick2"]);
    const panel = inspector(page);
    await panel.getByLabel("Direction").selectOption("stageLeft");
    await panel
      .getByLabel("Tilt up from the floor (degrees)")
      .first()
      .fill("20");
    await panel.getByRole("button", { name: "Face this way" }).click();
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Brick1").rotation).toEqual([110, 0, -90]);
    expect(saved(puts[0], "Brick2").rotation).toEqual([110, 0, -90]);
    expect(saved(puts[0], "Brick3").rotation).toEqual([110, 0, 0]);
  });

  test("the four named directions and a bearing", async ({ page }) => {
    const { puts } = await open(page);
    await selectByList(page, ["Brick3", "Brick7"]);
    const panel = inspector(page);
    await panel
      .getByLabel("Tilt up from the floor (degrees)")
      .first()
      .fill("20");
    let n = 0;
    for (const [dir, bearing] of [
      ["upstage", 0],
      ["stageRight", 90],
      ["downstage", 180],
      ["stageLeft", -90],
    ] as const) {
      await panel.getByLabel("Direction").selectOption(dir);
      await panel.getByRole("button", { name: "Face this way" }).click();
      await expect.poll(() => puts.length).toBe(++n);
      expect(saved(puts[n - 1], "Brick3").rotation).toEqual([110, 0, bearing]);
    }
    await panel.getByLabel("Direction").selectOption("bearing");
    await panel.getByLabel("Bearing (degrees)").fill("45");
    await panel
      .getByLabel("Tilt up from the floor (degrees)")
      .first()
      .fill("-10");
    await panel.getByRole("button", { name: "Face this way" }).click();
    await expect.poll(() => puts.length).toBe(++n);
    expect(saved(puts[n - 1], "Brick7").rotation).toEqual([80, 0, 45]);
  });

  test("the perimeter rig: sides face in, the front faces upstage", async ({
    page,
  }) => {
    const { puts } = await open(page);
    const panel = inspector(page);
    const faceTo = async (names: string[], dir: string) => {
      await page.keyboard.press("Escape");
      await selectByList(page, names);
      await panel.getByLabel("Direction").selectOption(dir);
      await panel
        .getByLabel("Tilt up from the floor (degrees)")
        .first()
        .fill("20");
      const before = puts.length;
      await panel.getByRole("button", { name: "Face this way" }).click();
      await expect.poll(() => puts.length).toBe(before + 1);
    };
    await faceTo(["Brick1", "Brick2"], "stageLeft");
    await faceTo(["Brick3", "Brick4", "Brick5", "Brick6"], "upstage");
    await faceTo(["Brick7", "Brick8"], "stageRight");
    const last = puts[puts.length - 1];
    expect(saved(last, "Brick1").rotation).toEqual([110, 0, -90]);
    expect(saved(last, "Brick4").rotation).toEqual([110, 0, 0]);
    expect(saved(last, "Brick8").rotation).toEqual([110, 0, 90]);
  });

  test("at a focus point uses the worked example", async ({ page }) => {
    // Brick4 stands at (0.09, -0.1, 0) here, as in the docs.
    const venue = houseVenue();
    saved(venue, "Brick4").position = [0.09, -0.1, 0];
    const { puts } = await open(page, venue);
    await selectByList(page, ["Brick4"]);
    await inspector(page).getByLabel("Focus point").selectOption("center");
    await inspector(page).getByRole("button", { name: "Aim at point" }).click();
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Brick4").rotation).toEqual([125.7, 0, -17]);
  });

  test("a fixture at the point is skipped, and said so", async ({ page }) => {
    const venue = houseVenue();
    saved(venue, "Brick5").position = [0.7, 1.9, 1.5];
    const { puts } = await open(page, venue);
    await selectByList(page, ["Brick4", "Brick5"]);
    await inspector(page).getByRole("button", { name: "Aim at point" }).click();
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Brick4").rotation![1]).toBe(0);
    expect(saved(puts[0], "Brick4").rotation).not.toEqual([110, 0, 0]);
    expect(saved(puts[0], "Brick5").rotation).toEqual([110, 0, 0]);
    await expect(inspector(page)).toContainText(
      "Skipped, already at the point: Brick5",
    );
  });

  test("with no focus points the tool says how to get one", async ({
    page,
  }) => {
    const venue = houseVenue();
    venue.focus_points = {};
    await open(page, venue);
    await selectByList(page, ["Brick4"]);
    await expect(inspector(page)).toContainText("no focus points");
    await expect(
      inspector(page).getByRole("button", { name: "Aim at point" }),
    ).toHaveCount(0);
  });

  test("movers get the two mountings, not a direction", async ({ page }) => {
    const { puts } = await open(page, houseVenue([MOVER]));
    await selectByList(page, ["Mover1"]);
    const panel = inspector(page);
    await expect(
      panel.getByRole("button", { name: "Face this way" }),
    ).toHaveCount(0);
    await panel.getByRole("button", { name: "Standing on the deck" }).click();
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Mover1").rotation).toEqual([180, 0, 0]);
    await panel.getByRole("button", { name: "Hung, facing downstage" }).click();
    await expect.poll(() => puts.length).toBe(2);
    expect(saved(puts[1], "Mover1").rotation).toEqual([0, 0, 180]);
  });

  test("a mover of a GDTF-referential type still gets the mountings", async ({
    page,
  }) => {
    const gdtf = { ...MOVER, name: "Gdtf1", fixture_type: "gdtfmover" };
    const { puts } = await open(page, houseVenue([gdtf]));
    await selectByList(page, ["Gdtf1"]);
    const panel = inspector(page);
    await expect(
      panel.getByRole("button", { name: "Face this way" }),
    ).toHaveCount(0);
    await panel.getByRole("button", { name: "Standing on the deck" }).click();
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Gdtf1").rotation).toEqual([180, 0, 0]);
  });

  test("a mixed selection aims the fixed ones and mounts the movers", async ({
    page,
  }) => {
    const { puts } = await open(page, houseVenue([MOVER]));
    await selectByList(page, ["Brick3", "Mover1"]);
    const panel = inspector(page);
    await panel.getByLabel("Direction").selectOption("downstage");
    await panel
      .getByLabel("Tilt up from the floor (degrees)")
      .first()
      .fill("0");
    await panel.getByRole("button", { name: "Face this way" }).click();
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Brick3").rotation).toEqual([90, 0, 180]);
    // The mover is untouched by a direction.
    expect(saved(puts[0], "Mover1").rotation).toEqual([0, 0, 180]);
  });

  test("the raw rotation is shown and editable", async ({ page }) => {
    const { puts } = await open(page);
    await selectByList(page, ["Brick3"]);
    const panel = inspector(page);
    await expect(panel.locator("#insp-rot-X")).toHaveValue("110");
    await expect(panel.locator("#insp-rot-Y")).toHaveValue("0");
    await expect(panel.locator("#insp-rot-Z")).toHaveValue("0");
    await panel.locator("#insp-rot-X").fill("12.5");
    await panel.locator("#insp-rot-Y").fill("3");
    await panel.locator("#insp-rot-Z").fill("45");
    await panel.getByRole("button", { name: "Set rotation" }).click();
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Brick3").rotation).toEqual([12.5, 3, 45]);

    // Two selected with different rotations show as mixed, and setting
    // writes to both.
    await selectByList(page, ["Brick1"]);
    await page.waitForTimeout(100);
    await expect(panel.locator("#insp-rot-X")).toHaveValue("");
    await panel.locator("#insp-rot-X").fill("100");
    await panel.locator("#insp-rot-Y").fill("0");
    await panel.locator("#insp-rot-Z").fill("10");
    await panel.getByRole("button", { name: "Set rotation" }).click();
    await expect.poll(() => puts.length).toBe(2);
    expect(saved(puts[1], "Brick1").rotation).toEqual([100, 0, 10]);
    expect(saved(puts[1], "Brick3").rotation).toEqual([100, 0, 10]);
  });
});

test.describe("Venues: beam angle", () => {
  const withBeam = () => {
    const venue = houseVenue();
    saved(venue, "Brick1").beam_angle = 45;
    saved(venue, "Brick2").beam_angle = 45;
    return venue;
  };
  const field = (page: Page) => page.getByTestId("inspector-beam-angle");
  const setButton = (page: Page) =>
    inspector(page).getByRole("button", { name: "Set beam angle" });

  test("setting it saves that fixture alone", async ({ page }) => {
    const { puts } = await open(page);
    await selectByList(page, ["Brick3"]);
    await expect(field(page)).toHaveValue("");
    await field(page).fill("60.5");
    await setButton(page).click();
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Brick3").beam_angle).toBe(60.5);
    for (const f of puts[0].fixtures) {
      if (f.name !== "Brick3") expect(f.beam_angle ?? null).toBeNull();
    }
    // The rest of the fixture is as it was.
    expect(saved(puts[0], "Brick3").rotation).toEqual([110, 0, 0]);
    expect(saved(puts[0], "Brick3").position).toEqual([-3.69, 0.01, 0]);
  });

  test("a mover takes one too", async ({ page }) => {
    const { puts } = await open(page, houseVenue([MOVER]));
    await selectByList(page, ["Mover1"]);
    await field(page).fill("30");
    await setButton(page).click();
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Mover1").beam_angle).toBe(30);
  });

  test("clearing it removes the override", async ({ page }) => {
    const { puts } = await open(page, withBeam());
    await selectByList(page, ["Brick1"]);
    await expect(field(page)).toHaveValue("45");
    await field(page).fill("");
    await setButton(page).click();
    await expect.poll(() => puts.length).toBe(1);
    expect(typeof saved(puts[0], "Brick1").beam_angle).not.toBe("number");
    // The other fixture with one keeps it.
    expect(saved(puts[0], "Brick2").beam_angle).toBe(45);
  });

  test("a mixed selection shows mixed, and setting writes to all", async ({
    page,
  }) => {
    const venue = withBeam();
    saved(venue, "Brick2").beam_angle = 90;
    const { puts } = await open(page, venue);
    await selectByList(page, ["Brick1", "Brick2"]);
    await expect(field(page)).toHaveValue("");
    await expect(field(page)).toHaveAttribute("placeholder", "mixed");
    await expect(setButton(page)).toBeDisabled();
    await field(page).fill("70");
    await setButton(page).click();
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Brick1").beam_angle).toBe(70);
    expect(saved(puts[0], "Brick2").beam_angle).toBe(70);
  });

  test("values out of range are refused and nothing is saved", async ({
    page,
  }) => {
    const { puts } = await open(page);
    await selectByList(page, ["Brick3"]);
    for (const bad of ["0", "-5", "181", "1e9"]) {
      await field(page).fill(bad);
      await expect(
        page.getByTestId("inspector-beam-angle-error"),
      ).toBeVisible();
      await expect(setButton(page)).toBeDisabled();
    }
    await field(page).fill("180");
    await expect(page.getByTestId("inspector-beam-angle-error")).toHaveCount(0);
    await expect(setButton(page)).toBeEnabled();
    expect(puts).toHaveLength(0);
  });

  test("arrange and aim keep a fixture's beam angle", async ({ page }) => {
    const { puts } = await open(page, withBeam());
    await selectByList(page, ["Brick1", "Brick2"]);
    await inspector(page)
      .getByRole("button", { name: "Mirror across centre" })
      .click();
    await expect.poll(() => puts.length).toBe(1);
    await inspector(page)
      .getByRole("button", { name: "Align on a line" })
      .click();
    await expect.poll(() => puts.length).toBe(2);
    await inspector(page)
      .getByRole("button", { name: "Face this way" })
      .click();
    await expect.poll(() => puts.length).toBe(3);
    for (const body of puts) {
      expect(saved(body, "Brick1").beam_angle).toBe(45);
      expect(saved(body, "Brick2").beam_angle).toBe(45);
      expect(saved(body, "Brick3").beam_angle ?? null).toBeNull();
    }
  });
});

test.describe("Venues: one fixture", () => {
  test("its own fields are editable and save once", async ({ page }) => {
    const { puts } = await open(page);
    await selectByList(page, ["Brick3"]);
    await expect(page.locator("#insp-name")).toHaveValue("Brick3");
    await expect(page.locator("#insp-universe")).toHaveValue("1");
    await expect(page.locator("#insp-channel")).toHaveValue("9");
    await page.locator("#insp-channel").fill("40");
    await inspector(page).getByRole("button", { name: "Apply" }).click();
    await expect.poll(() => puts.length).toBe(1);
    expect(saved(puts[0], "Brick3").start_channel).toBe(40);
    expect(saved(puts[0], "Brick3").position).toEqual([-3.69, 0.01, 0]);
    expect(puts[0].focus_points).toEqual({ center: [0.7, 1.9, 1.5] });
  });

  test("several selected show what they share", async ({ page }) => {
    await open(page);
    await selectByList(page, ["Brick3", "Brick4"]);
    await expect(page.getByTestId("shared-type")).toHaveText("brick");
    await expect(page.getByTestId("shared-tags")).toHaveText("wash, front");
    await selectByList(page, ["Brick1"]);
    await expect(page.getByTestId("shared-tags")).toHaveText("wash");
  });
});

test.describe("Venues: phone", () => {
  test.use({ viewport: { width: 390, height: 800 } });

  test("the inspector stacks below the plot", async ({ page }) => {
    await open(page);
    const plot = (await canvas(page).boundingBox())!;
    const panel = (await inspector(page).boundingBox())!;
    expect(panel.y).toBeGreaterThanOrEqual(plot.y + plot.height);
    expect(panel.width).toBeLessThanOrEqual(390);
    // No horizontal page scroll.
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - window.innerWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
  });
});
