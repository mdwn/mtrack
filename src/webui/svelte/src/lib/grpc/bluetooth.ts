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

// The Bluetooth controller's pairing over gRPC: the answer of `GetBluetooth`,
// `SetBluetoothPairing` and `ForgetBluetoothDevice` (all a `BluetoothStatus`),
// mapped into the shape the Bluetooth card holds. Only the mapping lives here,
// so a unit test can run it without a page; the calls are in
// src/lib/api/bluetooth.ts.

import type { BluetoothStatus as StatusMessage } from "../../gen/player/v1/player_pb";
import type { BluetoothStatus } from "../../gen/rules/mtrack_rules";

/** The Bluetooth controller as the card shows it: whether it runs, why it
 *  does not, the pairing window and the phones paired. */
export function mapBluetooth(r: StatusMessage): BluetoothStatus {
  return {
    configured: r.configured,
    running: r.running,
    error: r.error ?? null,
    pairing: {
      open: r.pairing?.open ?? false,
      seconds_left: r.pairing?.secondsLeft ?? 0,
    },
    devices: r.devices.map((d) => ({
      address: d.address,
      name: d.name ?? null,
      connected: d.connected,
    })),
  };
}
