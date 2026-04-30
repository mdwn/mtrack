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

import { readable, type Readable } from "svelte/store";

/**
 * A readable boolean store that reflects whether `mediaQuery` currently
 * matches. Updates as the viewport / orientation / OS theme changes.
 *
 * Returns `false` during SSR and before the browser MQL is available.
 */
export function matchMedia(mediaQuery: string): Readable<boolean> {
  return readable(false, (set) => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const mql = window.matchMedia(mediaQuery);
    set(mql.matches);
    const onChange = (e: MediaQueryListEvent) => set(e.matches);
    mql.addEventListener("change", onChange);
    return () => mql.removeEventListener("change", onChange);
  });
}

/** Phone-sized viewport (matches our 720px breakpoint). */
export const isPhone = matchMedia("(max-width: 720px)");
