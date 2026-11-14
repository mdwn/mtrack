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

// The player's lighting layers (`dmx.lighting.layers`): the timeline draws a
// lane for each, labelled by name, the effect form offers them, and the
// Groups page edits the list.

import { test, expect, type Page } from "@playwright/test";

const FIVE = ["floor", "back", "wash", "accents", "blinders"];

/** The mock player reports `layers` with its lighting groups, and a show
 *  that puts an effect on the fifth layer's neighbour. */
async function playerWithLayers(page: Page, layers: string[]) {
  await page.route("**/api/lighting/groups", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        groups: [{ name: "front_wash", fixtures: ["par1"] }],
        layers,
      }),
    }),
  );
  await page.route("**/api/lighting/show.light", (route) => {
    if (route.request().method() !== "GET") return route.continue();
    return route.fulfill({
      status: 200,
      contentType: "text/plain",
      body: 'show "Main" {\n    @00:00.000\n    front_wash: static, color: "blue", duration: 5s, layer: accents\n}\n',
    });
  });
}

test("a show on a custom fifth layer draws a lane for each layer, top first", async ({
  page,
}) => {
  await playerWithLayers(page, FIVE);
  await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
  await expect(page.locator(".show-group")).toHaveCount(1);
  // No layout error: the player's layers are the ones the show was read by.
  await expect(page.getByTestId("layout-error")).toHaveCount(0);
  const labels = page.locator(".show-group .sublane-type-label");
  await expect(labels).toHaveText([
    "blinders",
    "accents",
    "wash",
    "back",
    "floor",
    "Cmds",
    "Seqs",
  ]);
  // The BG/MG/FG initials are gone.
  await expect(
    page.locator(".show-group .sublane-type-label", { hasText: /^[BMF]G$/ }),
  ).toHaveCount(0);
  // The effect is drawn on its layer's lane.
  await expect(page.locator(".show-group .cue-block")).not.toHaveCount(0);
});

test("the default three layers are laid out top first by name", async ({
  page,
}) => {
  await playerWithLayers(page, ["background", "midground", "foreground"]);
  await page.route("**/api/lighting/show.light", (route) => {
    if (route.request().method() !== "GET") return route.continue();
    return route.fulfill({
      status: 200,
      contentType: "text/plain",
      body: 'show "Main" {\n    @00:00.000\n    front_wash: static, color: "blue", duration: 5s\n}\n',
    });
  });
  await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
  await expect(page.locator(".show-group")).toHaveCount(1);
  await expect(page.locator(".show-group .sublane-type-label")).toHaveText([
    "foreground",
    "midground",
    "background",
    "Cmds",
    "Seqs",
  ]);
});

test("the Groups page edits the layers and saves them bottom to top", async ({
  page,
}) => {
  const yaml = `songs: songs
profiles:
  - hostname: test-host
    dmx:
      universes: []
      lighting:
        current_venue: v
`;
  await page.route("**/api/config/store", (r) =>
    r.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ yaml, checksum: "one" }),
    }),
  );
  let put: {
    profile: { dmx: { lighting: Record<string, unknown> } };
  } | null = null;
  await page.route("**/api/config/profiles/*", (r) => {
    put = r.request().postDataJSON();
    return r.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ yaml, checksum: "two" }),
    });
  });
  await page.goto("/#/lighting/groups");
  const layers = page.getByTestId("lighting-layers");
  await expect(layers).toBeVisible();
  const names = () =>
    layers
      .locator(".layer-name")
      .evaluateAll((els) => els.map((e) => (e as HTMLInputElement).value));
  // Top first: the defaults as the player has them.
  await expect.poll(names).toEqual(["foreground", "midground", "background"]);

  // A new layer goes on top; name it, then move it down one place.
  await layers.getByTestId("layer-add").click();
  await expect
    .poll(names)
    .toEqual(["layer_4", "foreground", "midground", "background"]);
  const top = layers.getByTestId("layer-row").first();
  await top.locator(".layer-name").fill("accents");
  await top.locator(".layer-name").press("Enter");
  await top.getByRole("button", { name: "Move down" }).click();
  await expect
    .poll(names)
    .toEqual(["foreground", "accents", "midground", "background"]);

  // A name the parser cannot read is said so.
  await layers
    .getByTestId("layer-row")
    .nth(1)
    .locator(".layer-name")
    .fill("bad name");
  await layers
    .getByTestId("layer-row")
    .nth(1)
    .locator(".layer-name")
    .press("Enter");
  await expect(layers.getByTestId("layers-error")).toContainText(
    "A layer name is",
  );
  await layers
    .getByTestId("layer-row")
    .nth(1)
    .locator(".layer-name")
    .fill("accents");
  await layers
    .getByTestId("layer-row")
    .nth(1)
    .locator(".layer-name")
    .press("Enter");
  await expect(layers.getByTestId("layers-error")).toHaveCount(0);

  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect.poll(() => put).not.toBeNull();
  // Written bottom to top.
  expect(put!.profile.dmx.lighting.layers).toEqual([
    "background",
    "midground",
    "accents",
    "foreground",
  ]);
  expect(put!.profile.dmx.lighting.current_venue).toBe("v");
});

test("a save the server refuses for a layer a show uses says which shows", async ({
  page,
}) => {
  const yaml = `songs: songs
profiles:
  - hostname: test-host
    dmx:
      universes: []
      lighting:
        layers: [background, accents]
`;
  await page.route("**/api/config/store", (r) =>
    r.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ yaml, checksum: "one" }),
    }),
  );
  const message =
    "the lighting layers cannot be saved: removing or renaming a layer a show uses would leave that show unable to load; 'accents' (used by One (show.light, show \"main\")). Change those shows first.";
  await page.route("**/api/config/profiles/*", (r) =>
    r.fulfill({
      status: 409,
      contentType: "application/json",
      body: JSON.stringify({ error: message, layer_losses: [] }),
    }),
  );
  await page.goto("/#/lighting/groups");
  const layers = page.getByTestId("lighting-layers");
  await expect(layers).toBeVisible();
  await layers
    .getByTestId("layer-row")
    .first()
    .getByRole("button", { name: "Remove" })
    .click();
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    page.getByText("'accents' (used by One (show.light"),
  ).toBeVisible();
});

test("the effect form offers the player's layers", async ({ page }) => {
  await playerWithLayers(page, FIVE);
  await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
  const cue = page.locator(".cue-block").first();
  await expect(cue).toBeVisible();
  await cue.click({ force: true });
  const expand = page.locator(".effect-form .expand-toggle").first();
  await expect(expand).toBeVisible();
  await expand.click();
  const layer = page
    .locator(".effect-form label.param", { hasText: "Layer" })
    .locator("select");
  await expect(layer.locator("option")).toHaveText(["--", ...FIVE]);
  await expect(layer).toHaveValue("accents");
});
