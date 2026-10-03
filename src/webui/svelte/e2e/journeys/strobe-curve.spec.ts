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

import { CONFIG, expect, synthGdtf, test, type Project } from "./harness";

// The strobe curve, end to end on the real binary: the synthetic GDTF's
// strobe function has only its endpoints (0.4–25 Hz over DMX 7–255), so by
// the GDTF rule a 10 Hz strobe is linear in Hz (~104). Setting the curve to
// period on the fixture's page (as the Astera PixelBrick needs) moves the
// same show's strobe to 248. Read through POST /api/lighting/evaluate, the
// show preview's offline evaluation of the song with the loaded venue.

/** A mono 16-bit WAV of `samples` silent samples at 44.1 kHz. */
function silentWav(samples: number): Buffer {
  const data = samples * 2;
  const wav = Buffer.alloc(44 + data);
  wav.write("RIFF", 0);
  wav.writeUInt32LE(36 + data, 4);
  wav.write("WAVEfmt ", 8);
  wav.writeUInt32LE(16, 16);
  wav.writeUInt16LE(1, 20);
  wav.writeUInt16LE(1, 22);
  wav.writeUInt32LE(44100, 24);
  wav.writeUInt32LE(88200, 28);
  wav.writeUInt16LE(2, 32);
  wav.writeUInt16LE(16, 34);
  wav.write("data", 36);
  wav.writeUInt32LE(data, 40);
  return wav;
}

test.use({
  config: `${CONFIG}    groups:
      strobes:
        name: strobes
        constraints:
          - AllOf: ["strobe"]
`,
  files: {
    "lighting/library/synth.gdtf": synthGdtf(),
    "lighting/venues/house.light":
      'venue "house" {\n  fixture "B1" "Synth Brick" mode "8: RGBS" @ 1:1 tags ["strobe"]\n}\n',
    "songs/Strobe/kick.wav": silentWav(44100 * 6),
    "songs/Strobe/show.light":
      'show "S" {\n    @00:00.000\n    strobes: strobe frequency: 10, duration: 5s\n}\n',
    "songs/Strobe/song.yaml":
      "kind: song\nname: Strobe\ntracks:\n  - name: kick\n    file: kick.wav\nlighting:\n  - file: show.light\n",
  },
});

/** The strobe byte the show gives B1 one second in. */
async function strobeAt1s(project: Project): Promise<number | undefined> {
  const res = await fetch(`${project.url}/api/lighting/evaluate`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ song: "Strobe", times: [1] }),
  });
  if (!res.ok) throw new Error(`evaluate: ${res.status} ${await res.text()}`);
  const body = (await res.json()) as {
    evaluations: { fixtures: Record<string, Record<string, number>> }[];
  };
  return body.evaluations[0]?.fixtures.B1?.strobe;
}

test("the strobe curve set on the fixture's page changes what a show sends", async ({
  page,
  project,
}) => {
  // Nothing stated: as the GDTF declares, linear in Hz between its ends.
  await expect.poll(() => strobeAt1s(project)).toBeGreaterThanOrEqual(103);
  expect(await strobeAt1s(project)).toBeLessThanOrEqual(104);

  await page.goto("/#/lighting/fixtures/Synth%20Brick");
  const curve = page.getByTestId("ft-set-strobe");
  await expect(curve).toHaveValue("");
  await expect(curve.locator("option").first()).toHaveText(
    "Automatic: as the GDTF declares (linear in Hz)",
  );
  await curve.selectOption("period");
  await page.getByTestId("ft-set-save").click();
  await expect(page.getByTestId("ft-set-msg")).toContainText("Saved");

  // mtrack's record now says so, and the engine follows it.
  expect(
    project
      .read("lighting/fixture_types/synth_brick.fixture")
      .includes("strobe_curve: period"),
  ).toBe(true);
  await expect.poll(() => strobeAt1s(project)).toBe(248);

  // Back to automatic: the statement goes, and the strobe is linear again.
  await page.reload();
  await expect(curve).toHaveValue("period");
  await curve.selectOption("");
  await page.getByTestId("ft-set-save").click();
  await expect(page.getByTestId("ft-set-msg")).toContainText("Saved");
  expect(
    project
      .read("lighting/fixture_types/synth_brick.fixture")
      .includes("strobe_curve"),
  ).toBe(false);
  await expect.poll(() => strobeAt1s(project)).toBeLessThanOrEqual(104);
});
