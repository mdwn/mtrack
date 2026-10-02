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

import { type Page, type Response } from "@playwright/test";
import { api, expect, synthGdtf, test, type Project } from "./harness";

// Testing a fixture from its page, against the real binary with its DMX
// engine on the null client: Send lights it full white, a swatch changes
// the frame the engine lays over its output, Stop releases it, and a test
// nobody keeps alive goes dark by itself. The expiry is shortened through
// MTRACK_TEST_OUTPUT_EXPIRY_MS so the journey need not wait 5 s.

const EXPIRY_MS = 1500;
test.use({ env: { MTRACK_TEST_OUTPUT_EXPIRY_MS: String(EXPIRY_MS) } });

interface Frame {
  frame: { name: string; value: number; address: number }[];
}
interface Status {
  hardware: {
    test_output: {
      universe: number;
      address: number;
      footprint: number;
    } | null;
  };
}

const byName = (body: Frame) =>
  Object.fromEntries(body.frame.map((c) => [c.name, c.value]));

/** The next POST of the test (with this colour, when given: a heartbeat
 *  can come first). */
const posted = (page: Page, color?: string) =>
  page.waitForResponse(
    (r: Response) =>
      r.url().includes("/api/lighting/fixture-test") &&
      r.request().method() === "POST" &&
      (!color || (r.request().postData() ?? "").includes(color)),
  );

const testOutput = async (project: Project) =>
  (await api<Status>(project, "/status")).hardware.test_output;

test("a fixture lights full white on Send, follows a swatch, and stops", async ({
  page,
  project,
}) => {
  // Import the GDTF and land on its page.
  await page.goto("/#/lighting/fixtures");
  await page.locator('input[type="file"]').setInputFiles({
    name: "synth.gdtf",
    mimeType: "application/octet-stream",
    buffer: synthGdtf(),
  });
  await expect(page.getByTestId("ft-title")).toHaveText("Synth Brick");

  const section = page.getByTestId("fixture-test");
  await section.locator("summary").click();
  await expect(page.getByTestId("ftest-mode")).toHaveValue("8: RGBS");
  await expect(page.getByTestId("ftest-span")).toContainText("1–4");
  // Nothing is sent until Send is on.
  expect(await testOutput(project)).toBeNull();

  // Send alone: full white, strobe off.
  const first = posted(page);
  await page.getByTestId("ftest-send").check();
  const white = byName((await (await first).json()) as Frame);
  expect(white).toMatchObject({ red: 255, green: 255, blue: 255 });
  await expect(page.getByTestId("ftest-live")).toContainText(
    "universe 1, addresses 1–4",
  );
  await expect(page.getByTestId("ftest-sending")).toContainText(
    "255, 255, 255",
  );
  await expect
    .poll(() => testOutput(project))
    .toMatchObject({ universe: 1, address: 1, footprint: 4 });

  // A swatch: the engine's frame follows.
  const second = posted(page, "#ff0000");
  await page.getByTestId("ftest-swatch-red").click();
  expect(byName((await (await second).json()) as Frame)).toMatchObject({
    red: 255,
    green: 0,
    blue: 0,
  });

  // Stop: released.
  await page.getByTestId("ftest-stop").click();
  await expect.poll(() => testOutput(project)).toBeNull();
});

test("a test nobody keeps alive goes dark by itself", async ({ project }) => {
  project.write("lighting/library/synth.gdtf", synthGdtf());
  const res = await fetch(`${project.url}/api/lighting/fixture-test`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      fixture_type: "Synth Brick",
      mode: "8: RGBS",
      universe: 1,
      address: 1,
      controls: { color: "#ffffff", dimmer: 1, strobe: null },
    }),
  });
  expect(res.status).toBe(200);
  const body = (await res.json()) as { expires_in_secs: number };
  expect(body.expires_in_secs).toBeCloseTo(EXPIRY_MS / 1000, 1);
  expect(await testOutput(project)).not.toBeNull();
  // No heartbeat: gone soon after the expiry.
  await expect
    .poll(() => testOutput(project), { timeout: EXPIRY_MS + 3000 })
    .toBeNull();
});

test.describe("a venue fixture under test", () => {
  test.use({
    files: {
      "lighting/fixture_types/par.light":
        'fixture_type "Par" {\n  channels: 3\n  channel_map: { "red": 1, "green": 2, "blue": 3 }\n}\n',
      "lighting/venues/house.light":
        'venue "house" {\n  fixture "Front" Par @ 1:1\n  fixture "Back" Par @ 1:10\n}\n',
    },
  });

  interface StateMsg {
    type: string;
    fixtures: Record<string, Record<string, number>>;
    under_test?: string[];
  }

  test("the state stream shows the test's bytes on it, and the show again after Stop", async ({
    project,
  }) => {
    const states: StateMsg[] = [];
    const ws = new WebSocket(`${project.url.replace(/^http/, "ws")}/ws`);
    ws.onmessage = (e) => {
      const msg = JSON.parse(String(e.data)) as StateMsg;
      if (msg.type === "state") states.push(msg);
    };
    await new Promise((resolve, reject) => {
      ws.onopen = resolve;
      ws.onerror = reject;
    });
    const latest = () => states.at(-1);

    try {
      const post = async (color: string) => {
        const res = await fetch(`${project.url}/api/lighting/fixture-test`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            fixture_type: "Par",
            mode: null,
            universe: 1,
            address: 1,
            controls: { color, dimmer: 1 },
          }),
        });
        expect(res.status).toBe(200);
        return (await res.json()) as {
          warnings: { kind: string; fixtures?: string[] }[];
        };
      };
      const body = await post("#ff0000");
      // The test says it overrides the venue's fixture there.
      expect(
        body.warnings.find((w) => w.kind === "venue_overlap")?.fixtures,
      ).toEqual(["Front"]);

      await expect.poll(() => latest()?.under_test).toEqual(["Front"]);
      expect(latest()!.fixtures.Front).toMatchObject({
        red: 255,
        green: 0,
        blue: 0,
      });
      // Back, outside the span, is the show's (dark: nothing is playing).
      expect(latest()!.fixtures.Back?.red ?? 0).toBe(0);

      // A new colour follows.
      await post("#00ff00");
      await expect.poll(() => latest()?.fixtures.Front?.green).toBe(255);

      // Stop: back to the show's values, nothing marked.
      await fetch(`${project.url}/api/lighting/fixture-test`, {
        method: "DELETE",
      });
      await expect.poll(() => latest()?.under_test).toEqual([]);
      expect(latest()!.fixtures.Front?.green ?? 0).toBe(0);
      expect(latest()!.fixtures.Front?.red ?? 0).toBe(0);
    } finally {
      ws.close();
    }
  });
});
