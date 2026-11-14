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

// The bytes of a file, the half of an upload that is HTTP: "calls are gRPC;
// bytes are HTTP" (src/lib/grpc/uploads.ts has the rest).

import { configClient } from "../grpc/client";
import type { UploadPurpose } from "../../gen/player/v1/player_pb";
import { ApiError } from "./rest";

/** What `PUT /uploads/{id}` said when it refused: the server's `error`, else
 *  the status. */
async function putError(res: Response): Promise<ApiError> {
  const body: { error?: string } = await res.json().catch(() => ({}));
  return new ApiError(body.error || `Upload failed: ${res.status}`);
}

/** Announces `file` with `BeginUpload` and sends its bytes to the path that
 *  answers; the upload id, for the call that uses it. A name or size the
 *  server will not take is refused here, before any bytes are sent; a
 *  failed call is `fail`'s error (the caller's wording of it). */
export async function sendUpload(
  begin: {
    purpose: UploadPurpose;
    song?: string;
  },
  file: Blob & { name: string },
  fail: (e: unknown) => Error,
): Promise<string> {
  let begun;
  try {
    begun = await configClient.beginUpload({
      purpose: begin.purpose,
      fileName: file.name,
      size: BigInt(file.size),
      song: begin.song,
    });
  } catch (e) {
    throw fail(e);
  }
  const res = await fetch(begun.path, { method: "PUT", body: file });
  if (!res.ok) throw await putError(res);
  return begun.uploadId;
}
