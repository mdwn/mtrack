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

// The errors the web UI's calls raise: a refusal the server explained, and a
// save refused because the file changed underneath it.

/** A refused request's message, and the `kind` of refusal when the server
 *  says which (an import's `venue`, `record`, `library`, ...), so a caller
 *  offers the right way on without guessing from the status. */
export class ApiError extends Error {
  kind: string | null;
  constructor(message: string, kind: string | null = null) {
    super(message);
    this.name = "ApiError";
    this.kind = kind;
  }
}

/** A save refused because the file changed since the client read it (gRPC
 *  `ABORTED`). Carries the file's current version. */
export class ConflictError extends Error {
  version: string | null;
  constructor(message: string, version: string | null = null) {
    super(message);
    this.name = "ConflictError";
    this.version = version;
  }
}
