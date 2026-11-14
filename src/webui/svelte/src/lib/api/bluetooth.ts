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

/**
 * The Bluetooth controller's pairing: whether it runs, whether the pairing
 * window is open, and the phones paired. `ConfigService`'s Bluetooth calls.
 */

import type { BluetoothStatus } from "../../gen/rules/mtrack_rules";
import { configClient } from "../grpc/client";
import { mapBluetooth } from "../grpc/bluetooth";
import { callError } from "../grpc/library";

export async function fetchBluetooth(
  signal?: AbortSignal,
): Promise<BluetoothStatus> {
  try {
    return mapBluetooth(await configClient.getBluetooth({}, { signal }));
  } catch (e) {
    throw callError(e, "Failed to read Bluetooth");
  }
}

/** Opens the pairing window for `seconds`, or closes it. The answer is the
 *  controller as it is after. */
export async function setBluetoothPairing(
  open: boolean,
  seconds?: number,
): Promise<BluetoothStatus> {
  try {
    return mapBluetooth(
      await configClient.setBluetoothPairing({
        open,
        seconds: open ? seconds : undefined,
      }),
    );
  } catch (e) {
    throw callError(e, "Failed to change pairing");
  }
}

/** Forgets a paired phone, which must pair again. */
export async function forgetBluetoothPhone(
  address: string,
): Promise<BluetoothStatus> {
  try {
    return mapBluetooth(await configClient.forgetBluetoothDevice({ address }));
  } catch (e) {
    throw callError(e, "Failed to forget the phone");
  }
}
