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

import { describe, expect, it } from "vitest";
import { create } from "@bufbuild/protobuf";
import { Code, ConnectError } from "@connectrpc/connect";
import {
  LightingFitSchema,
  LightingReadinessSchema,
  StartFixtureTestRequestSchema,
} from "../../gen/player/v1/player_pb";
import { BadAnswerError } from "../lighting/answer";
import {
  mapFit,
  mapReadiness,
  startFixtureTestRequest,
  testCallError,
  TestRefused,
} from "./lighting";

describe("a fixture test's request", () => {
  it("splits the controls the page holds into typed ones and levels", () => {
    const request = create(
      StartFixtureTestRequestSchema,
      startFixtureTestRequest(
        {
          fixture_type: "Par",
          mode: null,
          universe: 1,
          address: 10,
          controls: {
            color: "#ff0000",
            dimmer: 0.5,
            strobe: 4,
            pan: 10,
            tilt: -5,
            amber: 0.25,
          },
          raw: { "3": 42, blue: 7 },
        },
        "rig/types",
        "rig/venues",
      ),
    );
    expect(request.fixtureType).toBe("Par");
    expect(request.mode).toBeUndefined();
    expect([request.universe, request.address]).toEqual([1, 10]);
    expect(request.controls?.color).toBe("#ff0000");
    expect(request.controls?.dimmer).toBe(0.5);
    expect(request.controls?.strobe?.hertz).toBe(4);
    expect([request.controls?.pan, request.controls?.tilt]).toEqual([10, -5]);
    expect(request.controls?.levels).toEqual({ amber: 0.25 });
    expect(request.raw).toEqual({ "3": 42, blue: 7 });
    expect([request.dir, request.venuesDir]).toEqual([
      "rig/types",
      "rig/venues",
    ]);
  });

  it("sends a strobe of null as one that is off, and none as untouched", () => {
    const off = create(
      StartFixtureTestRequestSchema,
      startFixtureTestRequest({
        fixture_type: "Par",
        universe: 1,
        address: 1,
        controls: { strobe: null },
      }),
    );
    expect(off.controls?.strobe).toBeDefined();
    expect(off.controls?.strobe?.hertz).toBeUndefined();
    const untouched = create(
      StartFixtureTestRequestSchema,
      startFixtureTestRequest({
        fixture_type: "Par",
        universe: 1,
        address: 1,
        controls: { dimmer: 1 },
      }),
    );
    expect(untouched.controls?.strobe).toBeUndefined();
    expect(untouched.dir).toBeUndefined();
  });
});

describe("a fixture test's refusal", () => {
  it("carries the reason the server gave", () => {
    const e = testCallError(
      new ConnectError("the address must be 1 to 512", Code.InvalidArgument, {
        "x-refusal-reason": "address",
      }),
      "Failed to send",
    );
    expect(e).toBeInstanceOf(TestRefused);
    expect((e as TestRefused).reason).toBe("address");
    expect(e.message).toBe("the address must be 1 to 512");
  });

  it("says locked for the lock layer's refusal, which names no reason", () => {
    const e = testCallError(
      new ConnectError(
        "Player is locked. Unlock to make changes.",
        Code.FailedPrecondition,
      ),
      "Failed to send",
    );
    expect((e as TestRefused).reason).toBe("locked");
  });

  it("has no reason for another failure, and the fallback without a message", () => {
    const e = testCallError(
      new ConnectError("", Code.Unavailable),
      "Failed to send",
    );
    expect((e as TestRefused).reason).toBeNull();
    expect(e.message).toContain("Failed to send");
  });

  it("passes an error that is not a failed call through", () => {
    const own = new TypeError("the page's own");
    expect(testCallError(own, "x")).toBe(own);
  });
});

describe("an answer the page cannot read", () => {
  it("is a bad answer, not a crash", () => {
    expect(() => mapReadiness(create(LightingReadinessSchema, {}))).toThrow(
      BadAnswerError,
    );
    expect(() => mapFit(create(LightingFitSchema, {}))).toThrow(BadAnswerError);
  });
});
