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
import type { Vec3 } from "../../src/gen/rules/mtrack_rules";
import {
  LightingService,
  PlayerService,
} from "../../src/gen/player/v1/player_pb";
import { getConfigReply, savedReply } from "../support/config-rpc";
import {
  savedTypeRequest,
  savedVenueRequest,
  venueDetailReply,
  venueListReply,
  venueSavedReply,
} from "../support/lighting-rpc";
import { card } from "../support/locators";
import { mockRpc } from "../support/rpc";
import { MVR_INSPECTION } from "../mock-server/test-data";
import { sendStage } from "../support/stage";

// What a first-time user does in each Lighting editor: open it, press the
// add/new button, take whatever the form offers, save — and what the server
// is sent. Every row the user saw must be in the request; a default that
// cannot be saved must stop the save where the user can see it, with no
// request at all. Every answer is routed per test (page.route) and every
// write is caught here, so the shared mock is never changed.
//
// Journeys covered elsewhere, with the same body assertions:
// - New venue, Add Fixture ×2 → both rows in the PUT: venue-editor-rows.spec.
// - Fit's Apply (tags): lighting-fit.spec, "Apply writes the suggestion's
//   tags and keeps everything else in the venue".
// - Fit's Place on plan (focus point): lighting-fit.spec.
// - Aim points from the export dialog, and the keep copy: lighting-mvr.spec,
//   "adds an aim point per fixture, refreshes, then downloads"; export with
//   the defaults: "Export as is downloads without adding aim points, keep
//   off by default".

let counter = 0;
const unique = (base: string) =>
  `${base}-${test.info().parallelIndex}-${++counter}-${Date.now()}`;

const json = (body: unknown, status = 200) => ({
  status,
  contentType: "application/json",
  body: JSON.stringify(body),
});

test.describe("Fixture types", () => {
  test("a new channel-map type with the defaults: the name is required, then the default channel is sent", async ({
    page,
  }) => {
    const rpc = await mockRpc(page);
    const writes = () => rpc.calls(LightingService.method.saveFixtureType);
    await page.goto("/#/lighting/fixtures");
    await page
      .getByRole("button", { name: "Define a fixture by hand" })
      .click();
    await page
      .getByRole("button", { name: "Channel map", exact: true })
      .click();
    const save = page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" });
    await save.click();
    // No name has a sensible default: the save stops, visibly.
    await expect(page.locator(".save-msg")).toHaveText("Name is required");
    await page.waitForTimeout(300);
    expect(writes()).toHaveLength(0);

    await page.locator("#ft-name").fill("House Par");
    await save.click();
    await expect.poll(() => writes().length).toBe(1);
    const sent = savedTypeRequest(writes()[0]);
    expect(sent.name).toBe("House Par");
    // The one channel row the form started with is in the body.
    expect(sent.form?.channels).toEqual({ dimmer: 1 });
  });

  test("a new .fixture type with the template saves the template as it is", async ({
    page,
  }) => {
    const rpc = await mockRpc(page);
    const writes = () => rpc.calls(LightingService.method.saveFixtureType);
    await page.goto("/#/lighting/fixtures");
    await page
      .getByRole("button", { name: "Define a fixture by hand" })
      .click();
    await page.getByTestId("new-ft-fixture").click();
    const text = await page.getByTestId("ft-dsl").inputValue();
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    await expect.poll(() => writes().length).toBe(1);
    const sent = savedTypeRequest(writes()[0]);
    expect(sent.name).toBe("Name");
    expect(sent.ext).toBe("fixture");
    expect(sent.text).toBe(text);
  });

  test("a GDTF import is one step: the file alone, no mode, no name", async ({
    page,
  }) => {
    const imports: Request[] = [];
    await page.route("**/api/lighting/gdtf/import*", (route) => {
      imports.push(route.request());
      return route.fallback();
    });
    await page.goto("/#/lighting/fixtures");
    await page.locator('input[type="file"]').setInputFiles({
      name: "pb15.gdtf",
      mimeType: "application/octet-stream",
      buffer: Buffer.from("not read by the mock"),
    });
    await expect(page.getByTestId("gdtf-report")).toBeVisible();
    expect(imports).toHaveLength(1);
    const params = new URL(imports[0].url()).searchParams;
    expect(params.get("mode")).toBeNull();
    expect(params.get("name")).toBeNull();
    expect(imports[0].postData()).toContain('filename="pb15.gdtf"');
  });
});

/** A venue with everything a fixture can carry. */
function richVenue(name: string) {
  return {
    name,
    fixtures: {
      Spot: {
        name: "Spot",
        fixture_type: "brick",
        universe: 1,
        start_channel: 1,
        tags: ["front", "wash"],
        position: [-2, 3.5, 4.2] as Vec3,
        rotation: [0, 0, 180] as Vec3,
        beam_angle: 60,
        mode: "Mover 16bit",
      },
      Par: {
        name: "Par",
        fixture_type: "par",
        universe: 2,
        start_channel: 10,
        tags: [],
        position: null,
        rotation: null,
        beam_angle: null,
      },
    },
    focus_points: { drummer: [0, 2.8, 1.4] as Vec3 },
    source: {
      mvr: "lighting/library/rig.mvr",
      origin: [0, -3.5, 0] as Vec3,
    },
  };
}

/** Serves one venue in the list (and by name), and records its writes. */
async function routeVenueList(page: Page, venue: ReturnType<typeof richVenue>) {
  const rpc = await mockRpc(page);
  rpc.unary(
    LightingService.method.listVenues,
    venueListReply({
      venues: { [venue.name]: venue },
      owners: {},
      errors: [],
      versions: { [venue.name]: "v1" },
    }),
  );
  rpc.unary(
    LightingService.method.getVenue,
    venueDetailReply({ venue, dsl: "", version: "v1" }),
  );
  rpc.unary(LightingService.method.saveVenue, (request) =>
    venueSavedReply({ name: request.name, version: "v2" }),
  );
  rpc.unary(LightingService.method.deleteVenue, (request) => ({
    name: request.name,
  }));
  return rpc;
}

test.describe("Venues", () => {
  test("Edit then Save with nothing changed sends every fixture with every field", async ({
    page,
  }) => {
    const venue = richVenue(unique("rig"));
    const rpc = await routeVenueList(page, venue);
    const writes = () => rpc.calls(LightingService.method.saveVenue);
    await page.goto("/#/lighting/venues");
    await page.getByTestId(`venue-edit-${venue.name}`).click();
    await expect(page.getByTestId("venue-fixture-row")).toHaveCount(2);
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    await expect.poll(() => writes().length).toBe(1);
    expect(writes()[0].expectedRevision).toBe("v1");
    const body = savedVenueRequest(writes()[0]);
    expect(body.fixtures).toEqual([
      { ...venue.fixtures.Spot },
      { ...venue.fixtures.Par, mode: null },
    ]);
    expect(body.focus_points).toEqual(venue.focus_points);
    expect(body.source).toEqual(venue.source);
  });

  test("a rename is one save under the new name, from the old, at its version", async ({
    page,
  }) => {
    const venue = richVenue(unique("rig"));
    const rpc = await routeVenueList(page, venue);
    const writes = () => rpc.calls(LightingService.method.saveVenue);
    await page.goto("/#/lighting/venues");
    await page.getByTestId(`venue-edit-${venue.name}`).click();
    await page.locator("#venue-name").fill(`${venue.name}-b`);
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    await expect.poll(() => writes().length).toBe(1);
    // The server moves or renames the one file: nothing is deleted after.
    await page.waitForTimeout(300);
    expect(writes()).toHaveLength(1);
    expect(rpc.calls(LightingService.method.deleteVenue)).toHaveLength(0);
    expect(writes()[0].name).toBe(`${venue.name}-b`);
    expect(writes()[0].from).toBe(venue.name);
    // The old venue's file is what is written, at the version it was read.
    expect(writes()[0].expectedRevision).toBe("v1");
    expect(savedVenueRequest(writes()[0]).fixtures.map((f) => f.name)).toEqual([
      "Spot",
      "Par",
    ]);
  });

  test("Delete, confirmed, sends a DeleteVenue and nothing else", async ({
    page,
  }) => {
    const venue = richVenue(unique("rig"));
    const rpc = await routeVenueList(page, venue);
    const writes = () =>
      rpc.allCalls.filter(
        (c) => c.method === "SaveVenue" || c.method === "DeleteVenue",
      );
    await page.goto("/#/lighting/venues");
    await card(page, venue.name)
      .getByRole("button", { name: "Delete" })
      .click();
    await page
      .locator(".dialog-overlay")
      .getByRole("button", { name: "Confirm" })
      .click();
    await expect.poll(() => writes().length).toBe(1);
    expect(writes()[0].method).toBe("DeleteVenue");
    expect(rpc.calls(LightingService.method.deleteVenue)[0].name).toBe(
      venue.name,
    );
  });
});

/** The plan on the Venues page with `venue` live, its file served. */
async function openPlan(page: Page, venue: ReturnType<typeof richVenue>) {
  let current = structuredClone(venue);
  const puts: Record<string, unknown>[] = [];
  const rpc = await mockRpc(page);
  rpc.unary(LightingService.method.saveVenue, (request) => {
    const body = savedVenueRequest(request);
    current = {
      ...current,
      ...body,
      fixtures: Object.fromEntries(body.fixtures.map((f) => [f.name, f])),
    } as unknown as typeof current;
    puts.push({
      fixtures: body.fixtures,
      focus_points: body.focus_points,
      source: body.source,
    });
    return venueSavedReply({ name: request.name });
  });
  rpc.unary(LightingService.method.getVenue, () =>
    venueDetailReply({ venue: current, dsl: "", version: "v1" }),
  );
  await page.goto(`/?wsId=${venue.name}#/lighting/venues`);
  await expect(page.locator(".item-card").first()).toBeVisible();
  await sendStage(page, venue.name, {
    type: "metadata",
    fixtures: Object.fromEntries(
      Object.values(venue.fixtures).map((f) => [
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
    venue: { name: venue.name, focus_points: venue.focus_points },
  });
  await expect(page.locator(".inspector")).toBeVisible();
  return puts;
}

/** Every fixture of a PUT, keyed by name. */
const byName = (put: Record<string, unknown>) =>
  Object.fromEntries(
    (put.fixtures as { name: string }[]).map((f) => [f.name, f]),
  );

test.describe("The Venues plan", () => {
  test("Apply in the inspector with the fields as shown changes nothing", async ({
    page,
  }) => {
    const venue = richVenue(unique("plan"));
    const puts = await openPlan(page, venue);
    await page
      .locator(".inspector")
      .getByLabel("Spot", { exact: true })
      .check();
    await expect(page.locator("#insp-name")).toHaveValue("Spot");
    await page
      .locator(".inspector")
      .getByRole("button", { name: "Apply" })
      .click();
    await expect.poll(() => puts.length).toBe(1);
    const fixtures = byName(puts[0]);
    expect(fixtures.Spot).toEqual(venue.fixtures.Spot);
    expect(fixtures.Par).toEqual({ ...venue.fixtures.Par, mode: null });
    expect(puts[0].focus_points).toEqual(venue.focus_points);
    expect(puts[0].source).toEqual(venue.source);
  });

  test("+ Focus point adds the point and keeps every fixture as it was", async ({
    page,
  }) => {
    const venue = richVenue(unique("plan"));
    const puts = await openPlan(page, venue);
    await page.getByRole("button", { name: "+ Focus point" }).click();
    await expect.poll(() => puts.length).toBe(1);
    const points = puts[0].focus_points as Record<string, unknown>;
    expect(Object.keys(points).sort()).toHaveLength(2);
    expect(points.drummer).toEqual(venue.focus_points.drummer);
    const fixtures = byName(puts[0]);
    expect(fixtures.Spot).toEqual(venue.fixtures.Spot);
    expect(fixtures.Par).toEqual({ ...venue.fixtures.Par, mode: null });
  });
});

test.describe("Groups", () => {
  test("Add a group and a constraint with their defaults: both are written", async ({
    page,
  }) => {
    const yaml = `songs: songs
profiles:
  - hostname: test-host
    dmx:
      universes: []
      lighting:
        groups:
          wash:
            name: wash
            constraints: []
`;
    const rpc = await mockRpc(page);
    rpc.unary(
      PlayerService.method.getConfig,
      getConfigReply({ yaml, checksum: "groups" }),
    );
    rpc.unary(
      PlayerService.method.updateProfile,
      savedReply({ yaml, checksum: "after" }),
    );
    await page.goto("/#/lighting/groups");
    await expect(page.locator(".group-card")).toHaveCount(1);
    const section = (title: string) =>
      page
        .locator(".subsection")
        .filter({ has: page.locator(".subsection-title", { hasText: title }) });
    // Fixtures are patched in venue files: the profile has no inline ones.
    await expect(section("Logical Groups")).toHaveCount(1);
    await expect(section("Inline Fixtures")).toHaveCount(0);
    await expect(page.getByText(/inline fixture/i)).toHaveCount(0);
    await section("Logical Groups")
      .getByRole("button", { name: "Add" })
      .first()
      .click();
    await expect(page.locator(".group-card")).toHaveCount(2);
    // The new group opens; give it its first constraint as offered.
    await page
      .locator(".group-card")
      .nth(1)
      .getByRole("button", { name: "Add" })
      .click();
    await page.getByRole("button", { name: "Save", exact: true }).click();
    await expect
      .poll(() => rpc.calls(PlayerService.method.updateProfile).length)
      .toBe(1);
    const lighting = JSON.parse(
      rpc.calls(PlayerService.method.updateProfile)[0].profileJson,
    ).dmx.lighting;
    expect(Object.keys(lighting.groups).sort()).toEqual(["new_group", "wash"]);
    expect(lighting.groups.new_group).toEqual({
      name: "new_group",
      constraints: [{ AllOf: [] }],
    });
    expect(lighting.fixtures).toBeUndefined();
  });
});

test.describe("MVR import", () => {
  test("the wizard with every default sends the name and origin it showed", async ({
    page,
  }) => {
    const imports: Request[] = [];
    await page.route("**/api/lighting/mvr/inspect", (r) =>
      r.fulfill(json(MVR_INSPECTION)),
    );
    await page.route("**/api/lighting/mvr/import", (r) => {
      imports.push(r.request());
      const write = /name="write"\r\n\r\ntrue/.test(
        r.request().postData() ?? "",
      );
      return r.fulfill(
        json(
          write
            ? {
                write: true,
                report: {
                  ...MVR_INSPECTION.report,
                  written: ["lighting/venues/kellys.venue"],
                  distillation_warnings: {},
                },
                reloaded: false,
                venue_error: null,
              }
            : { write: false, plan: MVR_INSPECTION.report },
        ),
      );
    });
    await page.goto("/#/lighting/import");
    await page.getByTestId("mvr-file").setInputFiles({
      name: "Kellys.mvr",
      mimeType: "application/octet-stream",
      buffer: Buffer.from("PK-mock"),
    });
    const name = await page.getByTestId("mvr-venue-name").inputValue();
    await page.getByTestId("mvr-continue").click();
    const origin = [
      await page.getByTestId("mvr-origin-x").inputValue(),
      await page.getByTestId("mvr-origin-y").inputValue(),
      await page.getByTestId("mvr-origin-z").inputValue(),
    ].join(",");
    await page.getByTestId("mvr-to-review").click();
    await page.getByTestId("mvr-do-import").click();
    await expect(page.getByTestId("mvr-done")).toBeVisible();
    const written = imports.at(-1)!.postData() ?? "";
    expect(written).toContain('name="write"\r\n\r\ntrue');
    expect(written).toContain(`name="name"\r\n\r\n${name}\r\n`);
    expect(written).toContain(`name="origin"\r\n\r\n${origin}\r\n`);
    expect(written).toContain('filename="Kellys.mvr"');
  });
});

test.describe("Song lighting", () => {
  test("+ DSL: Confirm waits for a name, then the new file is written", async ({
    page,
  }) => {
    const rpc = await mockRpc(page);
    rpc.unary(LightingService.method.saveLightShow, {});
    await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
    await expect(page.locator(".lighting-section")).toBeVisible();
    await page.getByRole("button", { name: /\+ DSL/i }).click();
    const dialog = page.locator('[role="dialog"]');
    // No name is offered, and none can be invented: Confirm waits for one.
    await expect(dialog.locator(".dialog-input")).toHaveValue("");
    const confirm = dialog.getByRole("button", { name: "Confirm" });
    await expect(confirm).toBeDisabled();
    await dialog.locator(".dialog-input").press("Enter");
    await expect(dialog).toBeVisible();
    await dialog.locator(".dialog-input").fill("encore");
    await confirm.click();
    await page.getByRole("button", { name: "Save" }).click();
    await expect
      .poll(() =>
        rpc
          .calls(LightingService.method.saveLightShow)
          .some((w) => w.name.includes("encore.light")),
      )
      .toBe(true);
  });
});
