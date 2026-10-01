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
 * Whether this tab runs an older UI than the server now serves. A single-page
 * app keeps its bundle until it is reloaded, so after mtrack is replaced and
 * restarted an open tab talks to the new server with old code.
 */

/** The server's build as `/api/status` reports it. */
export interface ServerBuild {
  build_time: string;
  /** The UI the server serves, when its build stamped one. */
  ui_build?: string | null;
}

/**
 * Two rules, the exact one when it can be asked:
 *
 * - The UI's build identity (the hash of its frontend inputs, compiled into
 *   the bundle) against the one the server reports for the UI it serves:
 *   different means this tab runs other code than a reload would load. Not
 *   the git hash — `make build-ui` skips rebuilding unchanged frontend
 *   sources, so the bundle's commit lags HEAD after a Rust-only change and
 *   would read as outdated when it is not.
 * - Without both (the dev server, a plain `npm run build`, an older server):
 *   the server's build time against the one seen when the page loaded. A
 *   change means the binary was replaced under the page.
 */
export function uiOutdated(
  uiBuild: string,
  firstSeen: ServerBuild | null,
  current: ServerBuild | null,
): boolean {
  if (!current) return false;
  const served = current.ui_build ?? "";
  if (uiBuild && served) return uiBuild !== served;
  return !!firstSeen && firstSeen.build_time !== current.build_time;
}
