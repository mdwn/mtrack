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

/** Tells a load repeated as its inputs change (a directory edited, say)
 *  whether its answer is still wanted: each call starts a load and returns
 *  a check that holds only while no later load has started. An answer to
 *  an earlier load that arrives after a later one's is stale. */
export function latestLoad(): () => () => boolean {
  let latest = 0;
  return () => {
    const mine = ++latest;
    return () => mine === latest;
  };
}

/** A load awaited by callers that read what it loaded, repeated as its
 *  inputs change. `load` applies its answer only while `current()` holds
 *  (see [`latestLoad`]).
 *
 *  A call settles once the newest load started by then has settled, so a
 *  caller whose own answer went stale still finds the newest one applied,
 *  not nothing: it resolves if that newest load resolved, and rejects with
 *  its error if it rejected. A stale load's rejection is never seen. */
export function newestLoad(
  load: (current: () => boolean) => Promise<void>,
): () => Promise<void> {
  const start = latestLoad();
  let newest: Promise<void> = Promise.resolve();
  return async () => {
    let waited = load(start());
    newest = waited;
    for (;;) {
      let failed: { error: unknown } | undefined;
      try {
        await waited;
      } catch (error) {
        failed = { error };
      }
      if (waited === newest) {
        if (failed) throw failed.error;
        return;
      }
      waited = newest;
    }
  };
}
