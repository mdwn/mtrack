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

// The Venues page's plot for a venue that is not the current one: it shows
// that venue's file (not live), and place / arrange / aim save to that venue
// through the same venue save, against the file's version. Every test routes
// its own venue API, with a per-test venue name, so nothing depends on state
// another test left in the shared mock server.

type Vec3 = [number, number, number];
interface FixtureBody {
  name: string;
  fixture_type: string;
  universe: number;
  start_channel: number;
  tags: string[];
  position: Vec3 | null;
  rotation: Vec3 | null;
}
interface VenueBody {
  fixtures: FixtureBody[];
  focus_points: Record<string, Vec3>;
  source: { mvr: string; origin: Vec3 } | null;
}
interface Put {
  name: string;
  ifMatch: string | undefined;
  body: VenueBody;
}

let counter = 0;
const uniq = (prefix: string) =>
  `${prefix}-${test.info().parallelIndex}-${++counter}-${Date.now()}`;

async function sendWs(page: Page, wsId: string, msg: object) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

const BRICK_TYPE = {
  fixture_type: {
    name: "brick",
    channels: { red: 1, green: 2, blue: 3 },
    max_strobe_frequency: null,
    min_strobe_frequency: null,
    strobe_dmx_offset: null,
  },
  file: "brick.fixture",
  extension: "fixture",
  referential: false,
  rich: false,
};

/** An in-memory venues directory behind `page.route`: the list, each venue's
 *  GET, and PUT with the version check the server makes (409 on a stale
 *  `If-Match`). `bump(name)` is another writer changing a file. */
async function routeVenues(
  page: Page,
  initial: Record<string, VenueBody> = {},
) {
  const files = new Map<string, { body: VenueBody; version: number }>();
  for (const [name, body] of Object.entries(initial)) {
    files.set(name, { body: structuredClone(body), version: 1 });
  }
  const puts: Put[] = [];
  const versionOf = (name: string) => `v${files.get(name)?.version}`;
  const asVenue = (name: string) => {
    const file = files.get(name)!;
    return {
      name,
      fixtures: Object.fromEntries(file.body.fixtures.map((f) => [f.name, f])),
      focus_points: file.body.focus_points,
      source: file.body.source,
    };
  };

  await page.route(/\/api\/lighting\/venues(\?.*)?$/, async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        venues: Object.fromEntries(
          [...files.keys()].map((n) => [n, asVenue(n)]),
        ),
        errors: [],
        versions: Object.fromEntries(
          [...files.keys()].map((n) => [n, versionOf(n)]),
        ),
      }),
    });
  });
  await page.route(
    /\/api\/lighting\/venues\/([^/?]+)(\?.*)?$/,
    async (route) => {
      const request = route.request();
      const name = decodeURIComponent(
        new URL(request.url()).pathname.split("/").pop()!,
      );
      if (request.method() === "PUT") {
        const ifMatch = request.headers()["if-match"];
        const current = files.get(name);
        if (ifMatch && (!current || versionOf(name) !== ifMatch)) {
          await route.fulfill({
            status: 409,
            contentType: "application/json",
            body: JSON.stringify({
              error: "changed since you loaded it",
              conflict: true,
              version: current ? versionOf(name) : null,
            }),
          });
          return;
        }
        const body = request.postDataJSON() as VenueBody;
        puts.push({ name, ifMatch, body: structuredClone(body) });
        files.set(name, { body, version: (current?.version ?? 0) + 1 });
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            status: "saved",
            name,
            reloaded: false,
            version: versionOf(name),
          }),
        });
        return;
      }
      if (request.method() !== "GET") return route.fallback();
      if (!files.has(name)) {
        await route.fulfill({
          status: 404,
          contentType: "application/json",
          body: JSON.stringify({ error: `Venue not found: ${name}` }),
        });
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          venue: asVenue(name),
          dsl: "",
          version: versionOf(name),
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
      body: JSON.stringify({
        fixture_types: { brick: BRICK_TYPE },
        errors: [],
      }),
    });
  });
  return {
    puts,
    file: (name: string) => files.get(name)!.body,
    /** Another writer changes the file: the version moves on. */
    bump: (name: string) => {
      files.get(name)!.version++;
    },
  };
}

const canvas = (page: Page) => page.locator(".stage-card__viewport canvas");
const inspector = (page: Page) => page.locator(".inspector");
const label = (page: Page) => page.getByTestId("stage-venue-label");

async function positions(page: Page) {
  const raw = await canvas(page).getAttribute("data-positions");
  return raw
    ? (JSON.parse(raw) as Record<string, { x: number; y: number }>)
    : {};
}

/** Drags fixture `name` from where the plot draws it to `(fx, fy)`, as
 *  fractions of the canvas. */
async function dragTo(page: Page, name: string, fx: number, fy: number) {
  // Selecting a fixture grows the inspector, which moves the plot: select
  // first, let the page settle, then measure and drag.
  await canvas(page).scrollIntoViewIfNeeded();
  const first = (await positions(page))[name];
  await canvas(page).click({ position: { x: first.x, y: first.y } });
  await page.waitForTimeout(300);
  const at = (await positions(page))[name];
  const box = (await canvas(page).boundingBox())!;
  await page.mouse.move(box.x + at.x, box.y + at.y);
  await page.mouse.down();
  await page.mouse.move(box.x + (at.x + box.width * fx) / 2, box.y + at.y - 10);
  await page.mouse.move(box.x + box.width * fx, box.y + box.height * fy);
  await page.mouse.up();
}

const brick = (name: string, channel: number): FixtureBody => ({
  name,
  fixture_type: "brick",
  universe: 1,
  start_channel: channel,
  tags: [],
  position: null,
  rotation: null,
});

test.describe("Venues plot: a venue that is not current", () => {
  // Tall enough that the plot and its tray are both on screen to drag between.
  test.use({ viewport: { width: 1280, height: 1600 } });

  test("create a venue, place two fixtures, arrange and aim: every save goes to that venue", async ({
    page,
  }) => {
    const served = await routeVenues(page);
    const name = uniq("newvenue");
    await page.goto(`/?wsId=${name}#/lighting/venues`);

    // Create it: two fixtures.
    await page.getByRole("button", { name: "New Venue" }).click();
    await page.locator("#venue-name").fill(name);
    for (const [i, [fixture, channel]] of [
      ["A", 1],
      ["B", 5],
    ].entries()) {
      await page.getByRole("button", { name: "Add Fixture" }).click();
      const card = page.locator(".venue-fixture-card").nth(i);
      await card.getByPlaceholder("Fixture name").fill(String(fixture));
      await card.locator("select").selectOption("brick");
      await page.locator(`#fix-channel-${i}`).fill(String(channel));
    }
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    await expect.poll(() => served.puts.length).toBe(1);
    expect(served.puts[0].name).toBe(name);
    // A brand-new venue has no version to present.
    expect(served.puts[0].ifMatch).toBeUndefined();

    // Nothing is current, but the new venue is picked and its plot shown,
    // and it says it is the file.
    await expect(page.getByTestId(`venue-card-${name}`)).toHaveClass(
      /item-card--selected/,
    );
    await expect(label(page)).toContainText(`Venue file: ${name} · not live`);
    await expect(page.getByTestId("stage-file-hint")).toBeVisible();
    await expect(page.getByTestId("stage-no-venue")).toHaveCount(0);

    // Place: a first pin turns the plot into a stage, and the fixtures wait
    // in the tray to be dragged on.
    await page.getByRole("button", { name: "+ Focus point" }).click();
    await expect.poll(() => served.puts.length).toBe(2);
    expect(Object.keys(served.puts[1].body.focus_points)).toHaveLength(1);
    await expect
      .poll(async () => Object.keys(await positions(page)).length)
      .toBe(2);
    await dragTo(page, "A", 0.3, 0.3);
    await expect.poll(() => served.puts.length).toBe(3);
    await dragTo(page, "B", 0.6, 0.3);
    await expect.poll(() => served.puts.length).toBe(4);
    const placed = served.file(name).fixtures;
    expect(placed.find((f) => f.name === "A")!.position).not.toBeNull();
    expect(placed.find((f) => f.name === "B")!.position).not.toBeNull();

    // Arrange: two ticked in the inspector, aligned on a line.
    await inspector(page).getByLabel("A", { exact: true }).check();
    await inspector(page).getByLabel("B", { exact: true }).check();
    await inspector(page)
      .getByRole("button", { name: "Align on a line" })
      .click();
    await expect.poll(() => served.puts.length).toBe(5);
    const [a, b] = ["A", "B"].map(
      (n) => served.puts[4].body.fixtures.find((f) => f.name === n)!.position!,
    );
    expect(a[1]).toBe(b[1]);

    // Aim: one fixture faces stage left.
    await inspector(page).getByLabel("B", { exact: true }).uncheck();
    await inspector(page).getByLabel("Direction").selectOption("stageLeft");
    await inspector(page)
      .getByRole("button", { name: "Face this way" })
      .click();
    await expect.poll(() => served.puts.length).toBe(6);
    const aimed = served.puts[5].body.fixtures.find((f) => f.name === "A")!;
    expect(aimed.rotation![2]).toBe(-90);

    // Every save went to the new venue, each against the version the plot
    // had loaded (so a change made elsewhere would have been refused).
    for (const put of served.puts) expect(put.name).toBe(name);
    for (const put of served.puts.slice(1)) expect(put.ifMatch).toBeTruthy();
  });

  test("a change made elsewhere is refused, the plot reloads, and reapplying works", async ({
    page,
  }) => {
    const name = uniq("shared");
    const served = await routeVenues(page, {
      [name]: {
        fixtures: [brick("A", 1)],
        focus_points: { spot: [0, 2, 0] },
        source: null,
      },
    });
    await page.goto(`/#/lighting/venues`);
    await page.getByTestId(`venue-card-${name}`).click();
    await expect(label(page)).toContainText("not live");
    await expect(page.locator('input[aria-label="spot"]')).toBeVisible();

    // Another tab saves the file.
    served.bump(name);
    await page.getByRole("button", { name: "+ Focus point" }).click();
    await expect(
      page.getByText(/changed since you loaded it/).first(),
    ).toBeVisible();
    expect(served.puts).toHaveLength(0);

    // The plot has reloaded at the new version: the same action now saves.
    await page.getByRole("button", { name: "+ Focus point" }).click();
    await expect.poll(() => served.puts.length).toBe(1);
    expect(Object.keys(served.puts[0].body.focus_points)).toHaveLength(2);
  });

  test("the current venue keeps its live plot; picking another shows its file", async ({
    page,
  }) => {
    const live = uniq("live");
    const other = uniq("other");
    await routeVenues(page, {
      [live]: {
        fixtures: [{ ...brick("A", 1), position: [0, 2, 3] }],
        focus_points: { spot: [0, 1, 0] },
        source: null,
      },
      [other]: {
        fixtures: [{ ...brick("Z", 1), position: [1, 1, 3] }],
        focus_points: { spot: [0, 1, 0] },
        source: null,
      },
    });
    await page.goto(`/?wsId=${live}#/lighting/venues`);
    await expect(page.getByTestId(`venue-card-${live}`)).toBeVisible();
    await sendWs(page, live, {
      type: "metadata",
      fixtures: {
        A: {
          tags: [],
          type: "brick",
          capabilities: ["color"],
          position: [0, 2, 3],
          rotation: null,
        },
      },
      venue: { name: live, dir: null, focus_points: { spot: [0, 1, 0] } },
    });
    await expect(label(page)).toContainText(`Current venue: ${live} · live`);
    await expect(page.getByTestId(`venue-live-${live}`)).toBeVisible();

    await page.getByTestId(`venue-card-${other}`).click();
    await expect(label(page)).toContainText(`Venue file: ${other} · not live`);
    await expect
      .poll(async () => Object.keys(await positions(page)))
      .toEqual(["Z"]);

    await page.getByTestId(`venue-card-${live}`).click();
    await expect(label(page)).toContainText(`Current venue: ${live} · live`);
    await expect
      .poll(async () => Object.keys(await positions(page)))
      .toEqual(["A"]);
  });

  test("the venue form saves against the version it opened at", async ({
    page,
  }) => {
    const name = uniq("form");
    const served = await routeVenues(page, {
      [name]: { fixtures: [brick("A", 1)], focus_points: {}, source: null },
    });
    await page.goto(`/#/lighting/venues`);
    await page.getByTestId(`venue-edit-${name}`).click();
    await expect(page.locator(".editor-form")).toBeVisible();

    // The file changes under the open form.
    served.bump(name);
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    await expect(
      page.getByText(/changed since you opened it/).first(),
    ).toBeVisible();
    expect(served.puts).toHaveLength(0);

    // The form reopened on the current file; saving now goes through.
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    await expect.poll(() => served.puts.length).toBe(1);
    expect(served.puts[0].ifMatch).toBe("v2");
  });
});
