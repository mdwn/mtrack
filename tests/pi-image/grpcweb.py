#!/usr/bin/env python3
# Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
#
# This program is free software: you can redistribute it and/or modify it under
# the terms of the GNU General Public License as published by the Free Software
# Foundation, version 3.
#
# This program is distributed in the hope that it will be useful, but WITHOUT
# ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
# FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License along with
# this program. If not, see <https://www.gnu.org/licenses/>.
#
"""The two gRPC-Web calls the boot check makes, for a shell with only curl.

The player's web port answers gRPC-Web, and in its text form
(`application/grpc-web-text`) a call is base64 in and base64 out, which curl
carries without help. This builds the two request bodies and reads the one
answer the check needs, with no protobuf library: the image under test has
none, and neither does the runner.

    grpcweb.py request watch-hardware   the body for PlayerService/Watch
    grpcweb.py request unlock           the body for PlayerService/SetLock
    grpcweb.py status < answer          the first hardware event, as JSON
    grpcweb.py ok < answer              exit 0 if the call's grpc-status is 0

Field numbers are those of src/proto/player/v1/player.proto.
"""

import base64
import json
import sys


def frame(message: bytes) -> str:
    """One gRPC-Web data frame, in the text form."""
    return base64.b64encode(b"\x00" + len(message).to_bytes(4, "big") + message).decode()


REQUESTS = {
    # WatchRequest { topics: [WATCH_TOPIC_HARDWARE (6)] }, packed.
    "watch-hardware": frame(b"\x0a\x01\x06"),
    # SetLockRequest { locked: false }: a false is not written.
    "unlock": frame(b""),
}


def unbase64(text: str) -> bytes:
    """A text-form body: base64 pieces end to end, each padded on its own, and
    the last perhaps cut short by a timeout."""
    text = "".join(text.split())
    out = b""
    for i in range(0, len(text) - len(text) % 4, 4):
        out += base64.b64decode(text[i : i + 4])
    return out


def frames(body: bytes):
    """Each whole frame of a body: (is it the trailers, its bytes)."""
    at = 0
    while at + 5 <= len(body):
        length = int.from_bytes(body[at + 1 : at + 5], "big")
        if at + 5 + length > len(body):
            return
        yield bool(body[at] & 0x80), body[at + 5 : at + 5 + length]
        at += 5 + length


def varint(data: bytes, at: int):
    value = shift = 0
    while True:
        byte = data[at]
        at += 1
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return value, at
        shift += 7


def fields(data: bytes) -> dict:
    """A message's fields by number, the last of each: an int for a varint,
    bytes for anything length-delimited."""
    found = {}
    at = 0
    while at < len(data):
        key, at = varint(data, at)
        number, wire = key >> 3, key & 7
        if wire == 0:
            found[number], at = varint(data, at)
        elif wire == 2:
            length, at = varint(data, at)
            found[number] = data[at : at + length]
            at += length
        elif wire == 1:
            at += 8
        elif wire == 5:
            at += 4
        else:
            raise ValueError(f"wire type {wire}")
    return found


def status(body: bytes) -> dict:
    """The first WatchEvent that is a HardwareStatus (field 6), as the three
    facts the check reads."""
    for trailers, message in frames(body):
        if trailers:
            continue
        hardware_status = fields(message).get(6)
        if hardware_status is None:
            continue
        top = fields(hardware_status)
        build = fields(top.get(1, b""))
        hardware = fields(top.get(2, b""))
        return {
            "build": {"version": build.get(1, b"").decode()},
            "hardware": {"hostname": hardware.get(2, b"").decode()},
            "locked": bool(top.get(4, 0)),
        }
    raise ValueError("no hardware event in the answer")


def call_ok(body: bytes) -> bool:
    for trailers, message in frames(body):
        if trailers:
            lines = message.decode(errors="replace").lower().split("\r\n")
            return any(line.replace(" ", "") == "grpc-status:0" for line in lines)
    return False


def main() -> int:
    args = sys.argv[1:]
    if len(args) == 2 and args[0] == "request" and args[1] in REQUESTS:
        print(REQUESTS[args[1]])
        return 0
    if args == ["status"]:
        print(json.dumps(status(unbase64(sys.stdin.read()))))
        return 0
    if args == ["ok"]:
        return 0 if call_ok(unbase64(sys.stdin.read())) else 1
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # An empty or cut answer: say so, print nothing.
        print(f"grpcweb.py: {error}", file=sys.stderr)
        sys.exit(1)
