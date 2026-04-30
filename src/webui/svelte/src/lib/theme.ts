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

import { writable, get, type Writable } from "svelte/store";

/**
 * "system" follows OS preference; "light" / "dark" are explicit user
 * overrides persisted in localStorage.
 */
export type ThemeChoice = "system" | "light" | "dark";

/** Effective theme actually applied to the page. */
export type EffectiveTheme = "light" | "dark";

const STORAGE_KEY = "mtrack-theme";

function readChoice(): ThemeChoice {
  if (typeof localStorage === "undefined") return "system";
  const v = localStorage.getItem(STORAGE_KEY);
  if (v === "light" || v === "dark") return v;
  return "system";
}

function systemPrefersDark(): boolean {
  if (typeof window === "undefined" || !window.matchMedia) return false;
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

function resolve(choice: ThemeChoice): EffectiveTheme {
  if (choice === "light") return "light";
  if (choice === "dark") return "dark";
  return systemPrefersDark() ? "dark" : "light";
}

function applyToDocument(theme: EffectiveTheme) {
  if (typeof document === "undefined") return;
  document.documentElement.classList.toggle("nc--dark", theme === "dark");
}

export const themeChoice: Writable<ThemeChoice> = writable(readChoice());
export const effectiveTheme: Writable<EffectiveTheme> = writable(
  resolve(get(themeChoice)),
);

// Re-resolve and apply whenever the choice changes.
themeChoice.subscribe((choice) => {
  if (typeof localStorage !== "undefined") {
    if (choice === "system") localStorage.removeItem(STORAGE_KEY);
    else localStorage.setItem(STORAGE_KEY, choice);
  }
  const next = resolve(choice);
  effectiveTheme.set(next);
  applyToDocument(next);
});

// Track OS preference changes when the user is on "system".
if (typeof window !== "undefined" && window.matchMedia) {
  const mql = window.matchMedia("(prefers-color-scheme: dark)");
  const onChange = () => {
    if (get(themeChoice) === "system") {
      const next = resolve("system");
      effectiveTheme.set(next);
      applyToDocument(next);
    }
  };
  mql.addEventListener("change", onChange);
}

/**
 * Cycle: system → light → dark → system. Used by the topnav toggle so
 * three clicks bring the user back to default. We expose this as a
 * helper so the keyboard shortcut and the UI both go through the same
 * code path.
 */
export function cycleTheme() {
  const current = get(themeChoice);
  themeChoice.set(
    current === "system" ? "light" : current === "light" ? "dark" : "system",
  );
}
