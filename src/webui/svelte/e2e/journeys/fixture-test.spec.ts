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
import { fromBinary } from "@bufbuild/protobuf";
import { createClient } from "@connectrpc/connect";
import { createGrpcWebTransport } from "@connectrpc/connect-web";
import {
  FixtureTestAnswerSchema,
  PlayerService,
} from "../../src/gen/player/v1/player_pb";
import type { StageState } from "../../src/gen/rules/mtrack_rules";
import { StageReader } from "../../src/lib/stage/feed";
import { readFrames } from "../support/grpc-web";
import { lightingClientAt } from "../support/player-client";
import { expect, status, synthGdtf, test, type Project } from "./harness";

// Testing a fixture from its page, against the real binary with its DMX
// engine on the null client: Send lights it full white, a swatch changes
// the frame the engine lays over its output, Stop releases it, and a test
// nobody keeps alive goes dark by itself. The expiry is shortened through
// MTRACK_TEST_OUTPUT_EXPIRY_MS so the journey need not wait 5 s.

const EXPIRY_MS = 1500;
test.use({ env: { MTRACK_TEST_OUTPUT_EXPIRY_MS: String(EXPIRY_MS) } });

type Answer = ReturnType<typeof answerOf>;

const answerOf = (body: Uint8Array) =>
  fromBinary(
    FixtureTestAnswerSchema,
    readFrames(body).find((f) => f.flag === 0)!.payload,
  );

const byName = (answer: Answer) =>
  Object.fromEntries(answer.frame.map((c) => [c.name, c.value]));

/** The next start of the test (with this colour, when given: a heartbeat
 *  can come first), and what the player answered it. */
const posted = async (page: Page, color?: string): Promise<Answer> => {
  const res = await page.waitForResponse(
    (r: Response) =>
      r.url().includes("/player.v1.LightingService/StartFixtureTest") &&
      (!color || (r.request().postDataBuffer()?.includes(color) ?? false)),
  );
  return answerOf(new Uint8Array(await res.body()));
};

const testOutput = async (project: Project) =>
  (await status(project)).hardware.test_output;

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
  const white = byName(await first);
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
  expect(byName(await second)).toMatchObject({
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
  const body = await lightingClientAt(project.url).startFixtureTest({
    fixtureType: "Synth Brick",
    mode: "8: RGBS",
    universe: 1,
    address: 1,
    controls: { color: "#ffffff", dimmer: 1, strobe: {} },
  });
  expect(body.expiresInSecs).toBeCloseTo(EXPIRY_MS / 1000, 1);
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

  test("the stage stream shows the test's bytes on it, and the show again after Stop", async ({
    project,
  }) => {
    // The stage as the web UI reads it: the player's gRPC stream
    // `WatchStage`, over gRPC-Web, into the stage's state.
    const states: StageState[] = [];
    const reader = new StageReader({
      metadata: () => {},
      state: (state) => states.push(state),
    });
    const stream = new AbortController();
    const client = createClient(
      PlayerService,
      createGrpcWebTransport({ baseUrl: project.url }),
    );
    void (async () => {
      try {
        for await (const update of client.watchStage(
          {},
          { signal: stream.signal },
        )) {
          reader.take(update);
        }
      } catch {
        // Ended with the test.
      }
    })();
    await expect.poll(() => states.length).toBeGreaterThan(0);
    const latest = () => states.at(-1);

    try {
      const lighting = lightingClientAt(project.url);
      const post = (color: string) =>
        lighting.startFixtureTest({
          fixtureType: "Par",
          universe: 1,
          address: 1,
          controls: { color, dimmer: 1 },
        });
      const body = await post("#ff0000");
      // The test says it overrides the venue's fixture there.
      const overlap = body.warnings.find(
        (w) => w.kind.case === "venueOverlap",
      )?.kind;
      expect(
        overlap?.case === "venueOverlap" ? overlap.value.fixtures : undefined,
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
      await lighting.stopFixtureTest({});
      await expect.poll(() => latest()?.under_test).toEqual([]);
      expect(latest()!.fixtures.Front?.green ?? 0).toBe(0);
      expect(latest()!.fixtures.Front?.red ?? 0).toBe(0);
    } finally {
      stream.abort();
    }
  });
});
