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
"""The gRPC-Web calls the boot check makes, for a shell with only curl.

The player's web port answers gRPC-Web, and in its text form
(`application/grpc-web-text`) a call is base64 in and base64 out, which curl
carries without help. This builds the request bodies and reads the answers
the check needs, with no protobuf library: the image under test has none, and
neither does the runner.

    grpcweb.py request watch-hardware   the body for PlayerService/Watch
    grpcweb.py request unlock           the body for PlayerService/SetLock
    grpcweb.py request save-fixture-type NAME DSL
                                        the body for LightingService/SaveFixtureType:
                                        a fixture type of that name from that text
    grpcweb.py request save-venue NAME FIXTURE TYPE UNIVERSE ADDRESS
                                        the body for LightingService/SaveVenue:
                                        a venue of that name holding one
                                        fixture of that type at that patch
    grpcweb.py request venue-patch NAME the body for LightingService/GetVenuePatch
    grpcweb.py status < answer          the first hardware event, as JSON
    grpcweb.py ok < answer              exit 0 if the call's grpc-status is 0
    grpcweb.py saved-fixture-type < answer
                                        the name a SaveFixtureType answer says it
                                        saved; fails if the call was refused
    grpcweb.py saved-venue < answer     the name a SaveVenue answer says it
                                        saved; fails if the call was refused
    grpcweb.py venue-footprint < answer the footprint of the first fixture of a
                                        GetVenuePatch answer (empty when its
                                        type or mode did not load); fails if
                                        the call was refused

Field numbers are those of src/proto/player/v1/player.proto.
"""

import base64
import json
import sys


def frame(message: bytes) -> str:
    """One gRPC-Web data frame, in the text form."""
    return base64.b64encode(b"\x00" + len(message).to_bytes(4, "big") + message).decode()


def varint_bytes(value: int) -> bytes:
    """A number as a varint."""
    out = b""
    while True:
        low = value & 0x7F
        value >>= 7
        if value:
            out += bytes([low | 0x80])
        else:
            return out + bytes([low])


def string_field(number: int, text: str) -> bytes:
    """A string field: its key (wire type 2), its length, its UTF-8."""
    data = text.encode()
    return varint_bytes(number << 3 | 2) + varint_bytes(len(data)) + data


def save_fixture_type(name: str, dsl: str) -> str:
    """SaveFixtureTypeRequest { name: 1, body: oneof { dsl: 6 } }."""
    return frame(string_field(1, name) + string_field(6, dsl))


def varint_field(number: int, value: int) -> bytes:
    """A varint field: its key (wire type 0), then the number."""
    return varint_bytes(number << 3) + varint_bytes(value)


def message_field(number: int, message: bytes) -> bytes:
    """A message field: its key (wire type 2), its length, its bytes."""
    return varint_bytes(number << 3 | 2) + varint_bytes(len(message)) + message


def save_venue(name: str, fixture: str, fixture_type: str, universe: int, address: int) -> str:
    """SaveVenueRequest { name: 1, body: oneof { form: 6 } } with a
    LightingVenueForm { fixtures: 1 } of one LightingFixture { name: 1,
    fixture_type: 2, universe: 3, start_channel: 4 }."""
    row = (
        string_field(1, fixture)
        + string_field(2, fixture_type)
        + varint_field(3, universe)
        + varint_field(4, address)
    )
    return frame(string_field(1, name) + message_field(6, message_field(1, row)))


def venue_patch(name: str) -> str:
    """GetVenuePatchRequest { name: 1 }."""
    return frame(string_field(1, name))


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


def saved_fixture_type(body: bytes) -> str:
    """The name in a SaveFixtureTypeResponse (field 1), of a call that was
    not refused."""
    if not call_ok(body):
        raise ValueError("the call was refused")
    for trailers, message in frames(body):
        if not trailers:
            return fields(message).get(1, b"").decode()
    raise ValueError("no answer in the body")


def saved_venue(body: bytes) -> str:
    """The name in a SaveVenueResponse (field 1), of a call that was not
    refused."""
    if not call_ok(body):
        raise ValueError("the call was refused")
    for trailers, message in frames(body):
        if not trailers:
            return fields(message).get(1, b"").decode()
    raise ValueError("no answer in the body")


def venue_footprint(body: bytes) -> str:
    """The footprint (field 4 of a PatchedFixture) of the first span (field 1
    of a VenuePatch) of a call that was not refused; empty for a fixture whose
    type or mode did not load, which has none."""
    if not call_ok(body):
        raise ValueError("the call was refused")
    for trailers, message in frames(body):
        if trailers:
            continue
        span = fields(message).get(1)
        if span is None:
            raise ValueError("the venue has no fixture")
        footprint = fields(span).get(4)
        return "" if footprint is None else str(footprint)
    raise ValueError("no answer in the body")


def main() -> int:
    args = sys.argv[1:]
    if len(args) == 2 and args[0] == "request" and args[1] in REQUESTS:
        print(REQUESTS[args[1]])
        return 0
    if len(args) == 4 and args[:2] == ["request", "save-fixture-type"]:
        print(save_fixture_type(args[2], args[3]))
        return 0
    if len(args) == 7 and args[:2] == ["request", "save-venue"]:
        print(save_venue(args[2], args[3], args[4], int(args[5]), int(args[6])))
        return 0
    if len(args) == 3 and args[:2] == ["request", "venue-patch"]:
        print(venue_patch(args[2]))
        return 0
    if args == ["saved-venue"]:
        print(saved_venue(unbase64(sys.stdin.read())))
        return 0
    if args == ["venue-footprint"]:
        print(venue_footprint(unbase64(sys.stdin.read())))
        return 0
    if args == ["saved-fixture-type"]:
        print(saved_fixture_type(unbase64(sys.stdin.read())))
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
