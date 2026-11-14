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

import { expect, synthGdtf, test, venueList, venueOf } from "./harness";
import { card } from "../support/locators";
import { lightingClientAt } from "../support/player-client";

// Fixture types and venues kept in subdirectories: the engine always read
// them, and the Lighting area must see the same project — listed, opened
// and saved back to the file they live in.

const PAR = `fixture_type "Par" {
  channels: 3
  channel_map: { "red": 1, "green": 2, "blue": 3 }
}
`;
const BRICK = `fixture_type "Brick" from gdtf("lighting/library/synth.gdtf") {
}
`;
const HOUSE = `venue "house" {
  fixture "Par 1" Par @ 1:1
  fixture "Brick 1" Brick mode "8: RGBS" @ 1:10
}
`;

test.describe("Project files in subdirectories", () => {
  test.use({
    files: {
      "lighting/library/synth.gdtf": synthGdtf(),
      "lighting/fixture_types/rig/par.light": PAR,
      "lighting/fixture_types/rig/gdtf/brick.fixture": BRICK,
      "lighting/venues/tour/house.light": HOUSE,
    },
  });

  test("are listed, open, and save back to their own files", async ({
    page,
    project,
  }) => {
    await page.goto("/#/lighting/fixtures");
    for (const name of ["Par", "Brick"])
      await expect(card(page, name)).toBeVisible();

    const lighting = lightingClientAt(project.url);
    const list = await lighting.listFixtureTypes({});
    expect(list.errors).toEqual([]);
    expect(list.fixtureTypes.Par.file).toBe("rig/par.light");
    expect(list.fixtureTypes.Brick.file).toBe("rig/gdtf/brick.fixture");
    await lighting.getFixtureType({ name: "Par" });
    await lighting.getFixtureTypeGdtfDetails({ name: "Brick" });

    const venues = await venueList(project);
    expect(Object.keys(venues.venues)).toEqual(["house"]);
    await venueOf(project, "house");

    await lighting.saveFixtureType({
      name: "Par",
      body: {
        case: "dsl",
        value: PAR.replace('"blue": 3', '"blue": 3, "white": 4').replace(
          "channels: 3",
          "channels: 4",
        ),
      },
    });
    expect(project.read("lighting/fixture_types/rig/par.light")).toContain(
      '"white": 4',
    );
    expect(project.exists("lighting/fixture_types/par.light")).toBe(false);
  });
});
