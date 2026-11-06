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

// Paths as the file browsers show them.

/** The directory above `path`; the root is its own. */
export function parentPath(path: string): string {
  const parts = path.replace(/\/+$/, "").split("/");
  if (parts.length <= 2) return "/";
  parts.pop();
  return parts.join("/");
}

/** Each directory from the root down to `path`, with its own path. */
export function breadcrumbs(path: string): { name: string; path: string }[] {
  const crumbs = [{ name: "/", path: "/" }];
  let acc = "";
  for (const part of path.split("/").filter(Boolean)) {
    acc += "/" + part;
    crumbs.push({ name: part, path: acc });
  }
  return crumbs;
}

/** A file's name without its extension: a track's default name. */
export function withoutExtension(file: string): string {
  return file.replace(/\.[^.]+$/, "");
}
