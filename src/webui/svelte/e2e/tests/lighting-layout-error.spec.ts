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

// A file that does not lay out -- here one whose include cannot be
// fetched -- shows the player's reason over the timeline, which keeps
// drawing the last layout that did lay out.

import { test, expect } from "@playwright/test";
import { Code, ConnectError } from "@connectrpc/connect";
import { LightingService } from "../../src/gen/player/v1/player_pb";
import { lightShowReply } from "../support/lighting-rpc";
import { mockRpc } from "../support/rpc";

// The show the mock server serves for any file.
const DEFAULT_SHOW =
  'show "Main" {\n    @00:00.000\n    front: static, color: "blue", duration: 5s\n}\n';
test("a failing include shows the reason and keeps the last good layout", async ({
  page,
}) => {
  const rpc = await mockRpc(page);
  rpc.unary(LightingService.method.getLightShow, (req) => {
    if (req.name !== "missing.light") return lightShowReply(DEFAULT_SHOW);
    throw new ConnectError(
      "Lighting file not found: missing.light",
      Code.NotFound,
    );
  });
  await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
  await expect(page.locator(".lighting-section")).toBeVisible();
  // The mock's show lays out: lanes, no banner.
  await expect(page.locator(".show-group")).toHaveCount(1);
  await expect(page.getByTestId("layout-error")).toHaveCount(0);

  // Add an include of a file the server does not have.
  await page.locator(".tab-btn", { hasText: "Raw DSL" }).click();
  const textarea = page.locator(".raw-textarea");
  await expect(textarea).toBeVisible();
  const source = await textarea.inputValue();
  await textarea.fill(`include "missing.light"\n${source}`);
  await page.locator(".tab-btn", { hasText: "Timeline" }).click();

  // The player's words for it, over the lanes of the last good layout.
  const banner = page.getByTestId("layout-error");
  await expect(banner).toBeVisible();
  await expect(banner).toContainText('include "missing.light"');
  await expect(banner).toContainText("cannot read");
  await expect(page.locator(".show-group")).toHaveCount(1);
});
