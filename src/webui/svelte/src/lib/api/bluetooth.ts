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
 * window is open, and the phones paired.
 */

import type { BluetoothStatus } from "../../gen/rules/mtrack_rules";
import { apiError, del, get, post } from "./rest";

export async function fetchBluetooth(
  signal?: AbortSignal,
): Promise<BluetoothStatus> {
  const res = await get("/bluetooth", signal);
  if (!res.ok) throw await apiError(res, "Failed to read Bluetooth");
  return res.json();
}

/** Opens the pairing window for `seconds`, or closes it. The answer is the
 *  controller as it is after. */
export async function setBluetoothPairing(
  open: boolean,
  seconds?: number,
): Promise<BluetoothStatus> {
  const res = await post(
    "/bluetooth/pairing",
    JSON.stringify(open ? { open, seconds } : { open }),
  );
  if (!res.ok) throw await apiError(res, "Failed to change pairing");
  return res.json();
}

/** Forgets a paired phone, which must pair again. */
export async function forgetBluetoothPhone(
  address: string,
): Promise<BluetoothStatus> {
  const res = await del(`/bluetooth/devices/${encodeURIComponent(address)}`);
  if (!res.ok) throw await apiError(res, "Failed to forget the phone");
  return res.json();
}
