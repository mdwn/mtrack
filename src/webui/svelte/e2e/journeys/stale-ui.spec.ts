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

import { expect, status, test } from "./harness";

// The "mtrack was updated" notice against the real binary. A journey cannot
// swap the binary or the shared UI build under a page, so this checks the
// other half: a restart of the same build — what a crash or a reboot looks
// like to an open tab — must not tell the user to reload. (Both rules agree
// here: the same binary keeps its build time, and the same UI its build.)

test("a restart of the same build shows no update notice", async ({
  page,
  project,
}) => {
  await page.goto("/#/");
  await expect(page.locator(".topnav__conn")).toBeVisible();
  const before = await status(project);
  // The server says which UI it serves: a string from `make build-ui`, null
  // from a plain `npm run build`.
  expect(
    before.build.ui_build === null || typeof before.build.ui_build === "string",
  ).toBe(true);

  await project.restart();
  const after = await status(project);
  expect(after.build).toEqual(before.build);
  // The page's Watch stream reopens on the restarted player and is told its
  // status as it opens; give it a moment past that.
  await expect(page.locator(".topnav__conn--off")).toHaveCount(0);
  await page.waitForTimeout(3000);
  await expect(page.getByTestId("ui-updated")).toHaveCount(0);
});
