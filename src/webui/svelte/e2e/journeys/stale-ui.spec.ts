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

import { api, expect, test } from "./harness";

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
  const before = await api<{ build: Record<string, unknown> }>(
    project,
    "/status",
  );
  // The server says which UI it serves: a string from `make build-ui`, null
  // from a plain `npm run build`.
  expect(
    before.build.ui_build === null || typeof before.build.ui_build === "string",
  ).toBe(true);

  await project.restart();
  const after = await api<{ build: Record<string, unknown> }>(
    project,
    "/status",
  );
  expect(after.build).toEqual(before.build);
  // Two status polls (every 5 seconds) after the restart.
  await page.waitForTimeout(11000);
  await expect(page.getByTestId("ui-updated")).toHaveCount(0);
});
