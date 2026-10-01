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

// The venue inspector's mode select and patch strip (venue-exchange design
// §21). Every test routes its own venue file, fixture types, archive and
// patch with `page.route` under a per-test venue name, so nothing depends
// on state another test left in the shared mock server.

interface FixtureBody {
  name: string;
  fixture_type: string;
  universe: number;
  start_channel: number;
  tags: string[];
  position: [number, number, number] | null;
  rotation: [number, number, number] | null;
  mode?: string | null;
}
interface VenueBody {
  fixtures: FixtureBody[];
  focus_points: Record<string, [number, number, number]>;
  source: null;
}

/** The PB15's modes, as the archive gives them. */
const MODES = [
  { name: "1: RGB", channel_count: 4, footprint: 3, capabilities: ["color"] },
  {
    name: "8: RGBS",
    channel_count: 5,
    footprint: 4,
    capabilities: ["color", "strobe"],
  },
  {
    name: "9: RGBWS",
    channel_count: 6,
    footprint: 5,
    capabilities: ["color", "strobe"],
  },
  {
    name: "13: DIM RGBAWS",
    channel_count: 7,
    footprint: 7,
    capabilities: ["color", "dimmer", "strobe"],
  },
  { name: "99: Broken", channel_count: 1, footprint: 1, refused: "no" },
];
const FOOTPRINT: Record<string, number> = Object.fromEntries(
  MODES.map((m) => [m.name, m.footprint]),
);

/** Eight bricks, four addresses each, back to back from 1. */
function rig(): VenueBody {
  return {
    fixtures: Array.from({ length: 8 }, (_, i) => ({
      name: `Brick${i + 1}`,
      fixture_type: "brick",
      universe: 1,
      start_channel: 1 + i * 4,
      tags: ["wash"],
      position: [-6 + i * 1.6, 2, 0] as [number, number, number],
      rotation: null,
      // Every fixture of a GDTF type names its own mode.
      mode: "8: RGBS" as string | null,
    })),
    focus_points: {},
    source: null,
  };
}

let counter = 0;

async function sendWs(page: Page, wsId: string, msg: object) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

/** Serves one venue and everything the inspector asks about it; collects
 *  the PUTs. */
async function routeAll(page: Page, name: string, initial: VenueBody) {
  let current = structuredClone(initial);
  const puts: VenueBody[] = [];
  const json = (body: unknown) => ({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify(body),
  });
  await page.route(
    new RegExp(`/api/lighting/venues/${name}(\\?.*)?$`),
    async (route) => {
      if (route.request().method() === "PUT") {
        current = route.request().postDataJSON();
        puts.push(structuredClone(current));
        return route.fulfill(json({ status: "saved", venue_error: null }));
      }
      return route.fulfill(
        json({
          venue: {
            name,
            fixtures: Object.fromEntries(
              current.fixtures.map((f) => [f.name, f]),
            ),
            focus_points: current.focus_points,
            source: null,
          },
          dsl: "",
        }),
      );
    },
  );
  await page.route(
    new RegExp(`/api/lighting/venues/${name}/patch(\\?.*)?$`),
    (route) =>
      route.fulfill(
        json({
          spans: current.fixtures.map((f) => ({
            fixture: f.name,
            universe: f.universe,
            address: f.start_channel,
            footprint: f.mode ? FOOTPRINT[f.mode] : null,
            type: f.fixture_type,
            mode: f.mode ?? null,
          })),
          overlaps: [],
          overruns: [],
        }),
      ),
  );
  await page.route("**/api/lighting/fixture-types*", (route) => {
    if (/fixture-types\/[^?]/.test(route.request().url()))
      return route.fallback();
    return route.fulfill(
      json({
        fixture_types: {
          brick: {
            fixture_type: {
              name: "brick",
              channels: {},
              max_strobe_frequency: null,
              min_strobe_frequency: null,
              strobe_dmx_offset: null,
            },
            file: "brick.fixture",
            extension: "fixture",
            referential: true,
            footprint: null,
            rich: false,
            gdtf: null,
          },
        },
        errors: [],
      }),
    );
  });
  await page.route("**/api/lighting/fixture-types/brick/gdtf*", (route) =>
    route.fulfill(
      json({
        archive: "library/pb15.gdtf",
        rig: null,
        thumbnail: null,
        beam: null,
        about: null,
        venues: [],
        inspection: {
          fixture: "PB15 PixelBrick",
          manufacturer: "Astera LED Technology",
          modes: MODES,
        },
      }),
    ),
  );
  return { puts };
}

async function open(page: Page, change: (v: VenueBody) => void = () => {}) {
  const name = `l21-${test.info().parallelIndex}-${++counter}-${Date.now()}`;
  const venue = rig();
  change(venue);
  const served = await routeAll(page, name, venue);
  await page.goto(`/?wsId=${name}#/lighting/venues`);
  await expect(page.locator(".item-card").first()).toBeVisible();
  await sendWs(page, name, {
    type: "metadata",
    fixtures: Object.fromEntries(
      venue.fixtures.map((f) => [
        f.name,
        {
          tags: f.tags,
          type: f.fixture_type,
          capabilities: ["color"],
          position: f.position,
          rotation: f.rotation,
        },
      ]),
    ),
    venue: { name, dir: null, focus_points: {} },
  });
  await expect(page.locator(".inspector")).toBeVisible();
  await page
    .locator(".inspector")
    .getByLabel("Brick3", { exact: true })
    .check();
  await expect(page.getByTestId("insp-mode")).toBeVisible();
  return served;
}

test.describe("Venue inspector: a fixture's mode", () => {
  test("the select shows the fixture's own mode among every mode with its footprint", async ({
    page,
  }) => {
    await open(page);
    const select = page.getByTestId("insp-mode");
    await expect(select).toHaveValue("8: RGBS");
    // No "type default": a mode is always the fixture's own choice.
    const options = select.locator("option");
    await expect(options).toHaveCount(MODES.length);
    await expect(options.nth(0)).toHaveText("1: RGB — 3 addresses");
    await expect(options.nth(2)).toHaveText("9: RGBWS — 5 addresses");
    await expect(select).not.toContainText(/default/i);
    // A refused mode is listed, not offered, its reason on hover.
    const broken = select.locator('option[value="99: Broken"]');
    await expect(broken).toBeDisabled();
    await expect(broken).toHaveAttribute("title", "no");
  });

  test("the patch strip shows this fixture among its neighbours", async ({
    page,
  }) => {
    await open(page);
    const strip = page.getByTestId("insp-patch-strip");
    await expect(strip.locator("li")).toHaveCount(32);
    const cell = (a: number) => strip.locator(`li[data-address="${a}"]`);
    await expect(cell(9)).toHaveClass(/patch-cell--me/);
    await expect(cell(12)).toHaveClass(/patch-cell--me/);
    await expect(cell(13)).toHaveClass(/patch-cell--other/);
    await expect(cell(13)).toHaveAttribute("title", "Brick4");
    await expect(page.locator(".inspector")).toContainText(
      "Universe 1 · addresses 1–32",
    );
  });

  test("a mode that collides is refused before the save, naming the neighbour", async ({
    page,
  }) => {
    const { puts } = await open(page);
    const select = page.getByTestId("insp-mode");
    await select.selectOption("13: DIM RGBAWS");
    await expect(page.getByTestId("insp-patch-msg")).toHaveText(
      "Not saved. 13: DIM RGBAWS needs addresses 9 to 15, and Brick4 already uses some of them. Move Brick4 or pick a mode of 4 addresses or fewer.",
    );
    // The select goes back to what is saved, and nothing was PUT.
    await expect(select).toHaveValue("8: RGBS");
    await page.waitForTimeout(300);
    expect(puts).toHaveLength(0);
  });

  test("a mode that fits saves, and the line carries it", async ({ page }) => {
    const { puts } = await open(page);
    const select = page.getByTestId("insp-mode");
    await select.selectOption("1: RGB");
    await expect.poll(() => puts.length).toBe(1);
    const brick3 = puts[0].fixtures.find((f) => f.name === "Brick3")!;
    expect(brick3.mode).toBe("1: RGB");
    // Nobody else's mode was touched.
    expect(
      puts[0].fixtures
        .filter((f) => f.name !== "Brick3")
        .every((f) => f.mode === "8: RGBS"),
    ).toBe(true);
    await expect(select).toHaveValue("1: RGB");

    // And back: the line names that mode again.
    await select.selectOption("8: RGBS");
    await expect.poll(() => puts.length).toBe(2);
    expect(puts[1].fixtures.find((f) => f.name === "Brick3")!.mode).toBe(
      "8: RGBS",
    );
  });

  test("a fixture read without its mode asks for one, and choosing it saves", async ({
    page,
  }) => {
    const { puts } = await open(page, (v) => (v.fixtures[2].mode = null));
    const select = page.getByTestId("insp-mode");
    await expect(select).toHaveValue("");
    const choose = select.locator('option[value=""]');
    await expect(choose).toHaveText("Choose a mode");
    await expect(choose).toBeDisabled();
    await select.selectOption("1: RGB");
    await expect.poll(() => puts.length).toBe(1);
    expect(puts[0].fixtures.find((f) => f.name === "Brick3")!.mode).toBe(
      "1: RGB",
    );
  });

  test("moving onto a neighbour's addresses is refused before the save", async ({
    page,
  }) => {
    const { puts } = await open(page);
    await page.locator("#insp-channel").fill("15");
    await page
      .locator(".inspector")
      .getByRole("button", { name: "Apply" })
      .click();
    await expect(page.getByTestId("insp-patch-msg")).toContainText(
      "Not saved. Brick3 at 1:15 needs addresses 15 to 18, and Brick4, Brick5 already use some of them.",
    );
    await page.waitForTimeout(300);
    expect(puts).toHaveLength(0);
  });
});

test("a fixture ganged on the same addresses is not a collision", async ({
  page,
}) => {
  const name = `l21g-${test.info().parallelIndex}-${++counter}-${Date.now()}`;
  const venue = rig();
  // Brick4 deliberately on Brick3's addresses: a gang.
  venue.fixtures[3].start_channel = 9;
  const { puts } = await routeAll(page, name, venue);
  await page.goto(`/?wsId=${name}#/lighting/venues`);
  await expect(page.locator(".item-card").first()).toBeVisible();
  await sendWs(page, name, {
    type: "metadata",
    fixtures: Object.fromEntries(
      venue.fixtures.map((f) => [
        f.name,
        {
          tags: f.tags,
          type: f.fixture_type,
          capabilities: ["color"],
          position: f.position,
          rotation: f.rotation,
        },
      ]),
    ),
    venue: { name, dir: null, focus_points: {} },
  });
  await page
    .locator(".inspector")
    .getByLabel("Brick3", { exact: true })
    .check();
  const strip = page.getByTestId("insp-patch-strip");
  await expect(strip.locator('li[data-address="9"]')).not.toHaveClass(
    /patch-cell--clash/,
  );
  // Renaming saves: the gang is not in the way.
  await page.locator("#insp-name").fill("Brick3b");
  await page
    .locator(".inspector")
    .getByRole("button", { name: "Apply" })
    .click();
  await expect.poll(() => puts.length).toBe(1);
});
