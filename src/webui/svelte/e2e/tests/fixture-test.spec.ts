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

// "Test this fixture" on a fixture's page: does the light answer when
// mtrack sends to it, and if not, why not. Every test records what the page
// sends with `page.route` and lets the mock answer.

let counter = 0;
const wsId = () =>
  `ftest-${test.info().parallelIndex}-${++counter}-${Date.now()}`;

interface Sent {
  method: string;
  body: Record<string, unknown> | null;
}

/** Records every POST and DELETE to the test endpoint, in order. */
async function record(page: Page): Promise<Sent[]> {
  const sent: Sent[] = [];
  await page.route(/\/api\/lighting\/fixture-test(\?.*)?$/, async (route) => {
    const method = route.request().method();
    sent.push({
      method,
      body: method === "POST" ? route.request().postDataJSON() : null,
    });
    await route.fallback();
  });
  return sent;
}

/** Rewrites the options the mock answers. */
async function options(
  page: Page,
  change: (o: Record<string, unknown>) => void,
): Promise<void> {
  await page.route(/\/api\/lighting\/fixture-test\/options/, async (route) => {
    const res = await route.fetch();
    const body = await res.json();
    change(body);
    await route.fulfill({ response: res, json: body });
  });
}

async function playback(page: Page, id: string, extra: object) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: {
      type: "playback",
      is_playing: false,
      elapsed_ms: 0,
      song_name: "Test Song Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
      ...extra,
      _wsId: id,
    },
  });
}

/** Opens a fixture's page and its test section. */
async function openTest(page: Page, type = "pixelbrick", id = wsId()) {
  await page.goto(`/?wsId=${id}#/lighting/fixtures/${type}`);
  const section = page.getByTestId("fixture-test");
  await section.locator("summary").first().click();
  await expect(page.getByTestId("ftest-send")).toBeVisible();
  await expect(page.getByTestId("ftest-send")).toBeEnabled();
  return id;
}

const posts = (sent: Sent[]) => sent.filter((s) => s.method === "POST");
const lastPost = (sent: Sent[]) => posts(sent).at(-1)?.body ?? null;

test("the mode drives the controls: strobe is there in RGBS and not in RGB", async ({
  page,
}) => {
  await openTest(page);
  await expect(page.getByTestId("ftest-mode")).toHaveValue("8: RGBS");
  await expect(page.getByTestId("ftest-span")).toHaveText("Uses addresses 1–4");
  await expect(page.getByTestId("ftest-strobe-on")).toBeVisible();
  await page.getByTestId("ftest-mode").selectOption("1: RGB");
  await expect(page.getByTestId("ftest-strobe-na")).toHaveText(
    "not in this mode",
  );
  await expect(page.getByTestId("ftest-span")).toHaveText("Uses addresses 1–3");
});

test("nothing is sent until Send is on; Send alone sends full white", async ({
  page,
}) => {
  const sent = await record(page);
  await openTest(page);
  await page.waitForTimeout(600);
  expect(sent).toEqual([]);

  await page.getByTestId("ftest-send").check();
  await expect.poll(() => posts(sent).length).toBeGreaterThan(0);
  expect(posts(sent)[0].body).toEqual({
    fixture_type: "pixelbrick",
    mode: "8: RGBS",
    universe: 1,
    address: 1,
    controls: { color: "#ffffff", dimmer: 1, strobe: null },
    raw: {},
  });
  await expect(page.getByTestId("ftest-live")).toHaveText(
    "Test output is live on universe 1, addresses 1–4",
  );
  // What mtrack is sending, from the answer.
  await expect(page.getByTestId("ftest-sending")).toHaveText(
    "mtrack is sending 255, 255, 255, 0 to universe 1, addresses 1–4.",
  );
  // The heartbeat keeps it alive with nothing touched.
  const before = posts(sent).length;
  await expect
    .poll(() => posts(sent).length, { timeout: 5000 })
    .toBeGreaterThan(before);
});

test("a swatch sends its colour; Blackout sends dark", async ({ page }) => {
  const sent = await record(page);
  await openTest(page);
  await page.getByTestId("ftest-send").check();
  await page.getByTestId("ftest-swatch-red").click();
  await expect
    .poll(() => (lastPost(sent)?.controls as { color?: string })?.color)
    .toBe("#ff0000");
  await expect(page.getByTestId("ftest-sending")).toContainText(
    "sending 255, 0, 0, 0",
  );
  await page.getByTestId("ftest-strobe-on").check();
  await expect
    .poll(() => (lastPost(sent)?.controls as { strobe?: number })?.strobe)
    .toBe(5);
  await page.getByTestId("ftest-blackout").click();
  await expect
    .poll(() => lastPost(sent)?.controls)
    .toEqual({ color: "#000000", dimmer: 0, strobe: null });
});

test("a raw channel is sent by hand and marked manual until reset", async ({
  page,
}) => {
  const sent = await record(page);
  await openTest(page);
  await page.getByTestId("ftest-send").check();
  await page.getByTestId("ftest-raw").locator("summary").click();
  const row = page.locator('[data-testid="ftest-raw-row"][data-offset="3"]');
  await row.getByTestId("ftest-raw-number").fill("128");
  await row.getByTestId("ftest-raw-number").press("Enter");
  await expect.poll(() => lastPost(sent)?.raw).toEqual({ "3": 128 });
  await expect(row.getByTestId("ftest-raw-manual")).toHaveText("manual");
  // The other rows show what the answer said.
  await expect(
    page
      .locator('[data-testid="ftest-raw-row"][data-offset="1"]')
      .getByTestId("ftest-raw-number"),
  ).toHaveValue("255");
  await row.getByTestId("ftest-raw-reset").click();
  await expect.poll(() => lastPost(sent)?.raw).toEqual({});
  await expect(row.getByTestId("ftest-raw-manual")).toHaveCount(0);
});

test("Stop releases, and so does leaving the page", async ({ page }) => {
  const sent = await record(page);
  await openTest(page);
  await page.getByTestId("ftest-send").check();
  await expect.poll(() => posts(sent).length).toBeGreaterThan(0);
  await page.getByTestId("ftest-stop").click();
  await expect.poll(() => sent.at(-1)?.method).toBe("DELETE");
  await expect(page.getByTestId("ftest-live")).toHaveCount(0);
  await expect(page.getByTestId("ftest-send")).not.toBeChecked();

  await page.getByTestId("ftest-send").check();
  await expect.poll(() => sent.at(-1)?.method).toBe("POST");
  await page
    .locator(".lighting__tabs")
    .getByRole("link", { name: "Venues" })
    .click();
  await expect.poll(() => sent.at(-1)?.method).toBe("DELETE");
});

test("moving the address while live releases the old one first", async ({
  page,
}) => {
  const sent = await record(page);
  await openTest(page);
  await page.getByTestId("ftest-send").check();
  await expect.poll(() => posts(sent).length).toBeGreaterThan(0);
  const at = sent.length;
  await page.getByTestId("ftest-address").fill("9");
  await page.getByTestId("ftest-address").press("Enter");
  await expect
    .poll(() => sent.slice(at).find((s) => s.method === "POST")?.body?.address)
    .toBe(9);
  const after = sent.slice(at);
  const release = after.findIndex((s) => s.method === "DELETE");
  const resend = after.findIndex(
    (s) => s.method === "POST" && s.body?.address === 9,
  );
  expect(release).toBeGreaterThanOrEqual(0);
  expect(release).toBeLessThan(resend);
  expect(after.some((s) => s.method === "POST" && s.body?.address === 1)).toBe(
    false,
  );
  await expect(page.getByTestId("ftest-live")).toHaveText(
    "Test output is live on universe 1, addresses 9–12",
  );
});

test("a hand-written type sends no mode, and its own dimmer", async ({
  page,
}) => {
  const sent = await record(page);
  await openTest(page, "par");
  await expect(page.getByTestId("ftest-mode")).toHaveCount(0);
  await page.getByTestId("ftest-send").check();
  await expect.poll(() => posts(sent).length).toBeGreaterThan(0);
  expect(posts(sent)[0].body).toMatchObject({
    fixture_type: "par",
    mode: null,
    controls: { color: "#ffffff", dimmer: 1 },
  });
});

test("help while live lists the unpatched universe first, with olad's page", async ({
  page,
}) => {
  await options(page, (o) => {
    (o.universes as { patched: boolean }[])[0].patched = false;
  });
  await openTest(page);
  await expect(page.getByTestId("ftest-unpatched")).toBeVisible();
  await page.getByTestId("ftest-send").check();
  const causes = page.getByTestId("ftest-help").locator("li");
  await expect(causes.first()).toHaveAttribute(
    "data-testid",
    "ftest-cause-unpatched",
  );
  await expect(causes.first().getByRole("link")).toHaveAttribute(
    "href",
    /:9090$/,
  );
  await expect(page.getByTestId("ftest-cause-address")).toContainText(
    "set to 1",
  );
  await expect(page.getByTestId("ftest-cause-mode")).toContainText(
    '"8: RGBS" (4 channels)',
  );
});

test("the nav says a test is live on every page, and stops it", async ({
  page,
}) => {
  const sent = await record(page);
  let live = true;
  await page.route("**/api/status", async (route) => {
    const res = await route.fetch();
    const body = await res.json();
    body.hardware.test_output = live
      ? {
          universe: 1,
          address: 5,
          footprint: 4,
          fixture_type: "pixelbrick",
          mode: "8: RGBS",
          expires_in_secs: 4,
        }
      : null;
    await route.fulfill({ response: res, json: body });
  });
  await page.goto(`/?wsId=${wsId()}#/`);
  const banner = page.getByTestId("test-output-banner");
  await expect(banner).toContainText(
    "Test output is live on universe 1, addresses 5–8 (pixelbrick).",
  );
  await expect(banner.getByRole("link", { name: "Open" })).toHaveAttribute(
    "href",
    "#/lighting/fixtures/pixelbrick",
  );
  live = false;
  await page.getByTestId("test-output-stop").click();
  await expect.poll(() => sent.at(-1)?.method).toBe("DELETE");
  await expect(banner).toHaveCount(0);
});

test.describe("when testing cannot happen, the section says why", () => {
  test("locked: unlock", async ({ page }) => {
    const id = await openTest(page);
    await playback(page, id, { locked: true });
    const why = page.getByTestId("ftest-unavailable");
    await expect(why).toHaveAttribute("data-reason", "locked");
    await expect(why).toContainText("The player is locked");
    await expect(why.getByRole("button", { name: "Unlock" })).toBeVisible();
    await expect(page.getByTestId("ftest-send")).toBeDisabled();
  });

  test("playing: stop the song; a live test stops too", async ({ page }) => {
    const sent = await record(page);
    const id = await openTest(page);
    await page.getByTestId("ftest-send").check();
    await expect.poll(() => posts(sent).length).toBeGreaterThan(0);
    await playback(page, id, { is_playing: true });
    const why = page.getByTestId("ftest-unavailable");
    await expect(why).toHaveAttribute("data-reason", "playing");
    await expect(why).toContainText("A song is playing");
    await expect(page.getByTestId("ftest-send")).not.toBeChecked();
    await expect(page.getByTestId("ftest-send")).toBeDisabled();
  });

  for (const reason of ["no_dmx", "no_universes"] as const) {
    test(`${reason}: a link to the profile's DMX settings`, async ({
      page,
    }) => {
      await options(page, (o) => {
        o.available = false;
        o.unavailable = reason;
        if (reason === "no_universes") o.universes = [];
      });
      await page.goto(`/?wsId=${wsId()}#/lighting/fixtures/pixelbrick`);
      await page.getByTestId("fixture-test").locator("summary").first().click();
      const why = page.getByTestId("ftest-unavailable");
      await expect(why).toHaveAttribute("data-reason", reason);
      await expect(why).toContainText(
        reason === "no_dmx" ? "no DMX output" : "no universes",
      );
      await expect(
        why.getByRole("link", { name: "Open the profile's DMX settings" }),
      ).toHaveAttribute("href", "#/config/test-host/lighting");
      await expect(page.getByTestId("ftest-send")).toBeDisabled();
    });
  }
});
