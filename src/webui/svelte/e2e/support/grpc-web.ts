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

// The gRPC-Web wire format a Playwright route answers in: length-prefixed
// frames, a message frame (flag 0) and a trailers frame (flag 0x80), or, for
// a call that fails before it answers, the status in the response headers
// alone ("trailers-only"). Pure, so it is unit tested (grpc-web.test.ts); the
// route that uses it is ./rpc.ts.

/** The trailers frame's flag bit. */
const TRAILERS_FLAG = 0x80;
/** The message frame's compressed bit; the app's client never sets it. */
const COMPRESSED_FLAG = 0x01;

/** One frame: the flag, the payload's length (4 bytes, big endian), the
 *  payload. */
function frame(flag: number, payload: Uint8Array): Uint8Array {
  const out = new Uint8Array(5 + payload.length);
  out[0] = flag;
  new DataView(out.buffer).setUint32(1, payload.length);
  out.set(payload, 5);
  return out;
}

function concat(parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}

/** A message frame. */
export function messageFrame(message: Uint8Array): Uint8Array {
  return frame(0, message);
}

/** A trailers frame: `name: value` lines, CRLF ended. */
export function trailersFrame(trailers: Record<string, string>): Uint8Array {
  const text = Object.entries(trailers)
    .map(([name, value]) => `${name}: ${value}\r\n`)
    .join("");
  return frame(TRAILERS_FLAG, new TextEncoder().encode(text));
}

/** A unary call's successful body: its one message, then `grpc-status: 0`. */
export function unaryBody(message: Uint8Array): Uint8Array {
  return concat([messageFrame(message), trailersFrame({ "grpc-status": "0" })]);
}

/** The headers of a call that failed with `code` (a gRPC status code, 1 to
 *  16) and `message`: the status is in the headers and the body is empty. The
 *  message is percent encoded, as `grpc-message` is. */
export function errorHeaders(
  code: number,
  message: string,
): Record<string, string> {
  return {
    "content-type": "application/grpc-web+proto",
    "grpc-status": String(code),
    "grpc-message": encodeURIComponent(message),
  };
}

/** The message of a request body's first frame. A unary call has exactly
 *  one. Throws on a body that is not a whole, uncompressed message frame. */
export function requestMessage(body: Uint8Array): Uint8Array {
  if (body.length < 5) {
    throw new Error(
      `gRPC-Web request body is ${body.length} bytes, not a frame`,
    );
  }
  const flag = body[0];
  if (flag & TRAILERS_FLAG || flag & COMPRESSED_FLAG) {
    throw new Error(
      `gRPC-Web request frame has flag ${flag}, not a plain message`,
    );
  }
  const length = new DataView(body.buffer, body.byteOffset).getUint32(1);
  if (body.length < 5 + length) {
    throw new Error(
      `gRPC-Web request frame says ${length} bytes, the body holds ${body.length - 5}`,
    );
  }
  return body.subarray(5, 5 + length);
}

/** The frames of a response body, in order: each its flag and payload. For a
 *  test that checks what {@link unaryBody} wrote. */
export function readFrames(
  body: Uint8Array,
): { flag: number; payload: Uint8Array }[] {
  const frames: { flag: number; payload: Uint8Array }[] = [];
  let at = 0;
  while (at < body.length) {
    const length = new DataView(body.buffer, body.byteOffset).getUint32(at + 1);
    frames.push({
      flag: body[at],
      payload: body.subarray(at + 5, at + 5 + length),
    });
    at += 5 + length;
  }
  return frames;
}
