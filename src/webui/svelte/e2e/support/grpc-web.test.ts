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

import { describe, expect, it } from "vitest";
import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import {
  PlayRequestSchema,
  SeekRequestSchema,
} from "../../src/gen/player/v1/player_pb";
import {
  errorHeaders,
  messageFrame,
  readFrames,
  requestMessage,
  trailersFrame,
  unaryBody,
} from "./grpc-web";

const text = (bytes: Uint8Array) => new TextDecoder().decode(bytes);

describe("gRPC-Web frames", () => {
  it("prefixes a message with a zero flag and its big endian length", () => {
    const framed = messageFrame(new Uint8Array([1, 2, 3]));
    expect([...framed]).toEqual([0, 0, 0, 0, 3, 1, 2, 3]);
  });

  it("frames an empty message", () => {
    expect([...messageFrame(new Uint8Array())]).toEqual([0, 0, 0, 0, 0]);
  });

  it("writes trailers as CRLF ended lines under the trailers flag", () => {
    const [frame] = readFrames(trailersFrame({ "grpc-status": "0", x: "y" }));
    expect(frame.flag).toBe(0x80);
    expect(text(frame.payload)).toBe("grpc-status: 0\r\nx: y\r\n");
  });

  it("answers a unary call with its message, then status 0", () => {
    const message = toBinary(
      SeekRequestSchema,
      create(SeekRequestSchema, { position: { seconds: 5n, nanos: 0 } }),
    );
    const frames = readFrames(unaryBody(message));
    expect(frames.map((f) => f.flag)).toEqual([0, 0x80]);
    expect([...frames[0].payload]).toEqual([...message]);
    expect(text(frames[1].payload)).toBe("grpc-status: 0\r\n");
  });

  it("answers an empty message with an empty message frame", () => {
    const frames = readFrames(unaryBody(new Uint8Array()));
    expect(frames[0].payload.length).toBe(0);
    expect(frames).toHaveLength(2);
  });

  it("puts an error's status in the headers, its message percent encoded", () => {
    expect(
      errorHeaders(9, "Player is locked. Unlock to make changes."),
    ).toEqual({
      "content-type": "application/grpc-web+proto",
      "grpc-status": "9",
      "grpc-message": "Player%20is%20locked.%20Unlock%20to%20make%20changes.",
    });
  });
});

describe("gRPC-Web requests", () => {
  it("reads the message back out of a request body", () => {
    const message = toBinary(
      SeekRequestSchema,
      create(SeekRequestSchema, { position: { seconds: 7n, nanos: 500 } }),
    );
    const decoded = fromBinary(
      SeekRequestSchema,
      requestMessage(messageFrame(message)),
    );
    expect(decoded.position?.seconds).toBe(7n);
    expect(decoded.position?.nanos).toBe(500);
  });

  it("reads an empty request", () => {
    const decoded = fromBinary(
      PlayRequestSchema,
      requestMessage(messageFrame(new Uint8Array())),
    );
    expect(decoded).toEqual(create(PlayRequestSchema));
  });

  it("reads a body that is a view into a larger buffer", () => {
    const framed = messageFrame(new Uint8Array([9, 8]));
    const padded = new Uint8Array(framed.length + 3);
    padded.set(framed, 3);
    expect([...requestMessage(padded.subarray(3))]).toEqual([9, 8]);
  });

  it("refuses a short, truncated, trailers or compressed body", () => {
    expect(() => requestMessage(new Uint8Array([0, 0]))).toThrow(/not a frame/);
    expect(() => requestMessage(new Uint8Array([0, 0, 0, 0, 4, 1]))).toThrow(
      /says 4 bytes/,
    );
    expect(() => requestMessage(trailersFrame({ a: "b" }))).toThrow(/flag 128/);
    expect(() => requestMessage(new Uint8Array([1, 0, 0, 0, 0]))).toThrow(
      /flag 1/,
    );
  });
});
