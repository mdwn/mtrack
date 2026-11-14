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
import { BluetoothStatusSchema } from "../../gen/player/v1/player_pb";
import { mapBluetooth } from "./bluetooth";

describe("the Bluetooth controller", () => {
  it("is as the card holds it: a name or null, an error or null", () => {
    expect(
      mapBluetooth(
        create(BluetoothStatusSchema, {
          configured: true,
          running: true,
          pairing: { open: true, secondsLeft: 87 },
          devices: [
            { address: "AA:BB:CC:DD:EE:01", name: "Pixel", connected: true },
            { address: "AA:BB:CC:DD:EE:02", connected: false },
          ],
        }),
      ),
    ).toEqual({
      configured: true,
      running: true,
      error: null,
      pairing: { open: true, seconds_left: 87 },
      devices: [
        { address: "AA:BB:CC:DD:EE:01", name: "Pixel", connected: true },
        { address: "AA:BB:CC:DD:EE:02", name: null, connected: false },
      ],
    });
  });

  it("says why it is not running, closed and empty", () => {
    expect(
      mapBluetooth(
        create(BluetoothStatusSchema, {
          configured: true,
          error: "the radio is blocked",
        }),
      ),
    ).toEqual({
      configured: true,
      running: false,
      error: "the radio is blocked",
      pairing: { open: false, seconds_left: 0 },
      devices: [],
    });
  });
});
