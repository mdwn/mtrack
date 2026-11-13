# The web UI on gRPC

*Design doc, draft 1 — 2026-11-12. Follows `design/remote_phase3_design.md`
§5, which left open how far the web UI moves to gRPC. Mike has since decided
the direction: "I'm not worried about cost. Consistency is a big thing here,
but only if we can get the whole thing consistent. gRPC has a lot of
advantages w/r/t versioning." The light view moved first (#730). This doc
says what "the whole thing" is, how to get there without a broken web UI on
the way, and what is his to decide (§10).*

## 1. The end state

- **Every call and every live feed is gRPC.** The web UI speaks gRPC-Web to
  the web server's own port, as it does today for the transport buttons and
  the stage. The phone app speaks gRPC on the gRPC port, or the same
  messages over Bluetooth.
- **HTTP carries the page and file bytes, nothing else.** The static page,
  and the bodies of uploads and downloads at paths gRPC hands out (§5).
- **The REST API and the JSON websocket are deleted.** No new REST route is
  added on the way.
- **One description of the API.** The `.proto` files. One lock policy, one
  way to say "someone else changed this", one set of generated types for
  the web UI, the app, the harness and anyone's script.

## 2. What is there today

Counted on main `f8d84727`.

| | Count | Notes |
|---|---|---|
| REST routes under `/api` | 85 | 75 called by the web UI, 10 by nothing in it |
| Websocket message types | 5 | `playback`, `logs`, `waveform`, `config_changed`, `reload`. Server to browser only |
| `PlayerService` RPCs | 44 | 3 of them streams. The web UI calls 14 |
| Playwright spec files | 136 | 52 of them stub `/api` routes; 17 use the websocket helper |
| Routes the test mock server answers | 75 | Plus the websocket |

REST routes by area:

| Area | Routes | Carries files? |
|---|---|---|
| Config and profiles | 18 | No |
| Songs | 8 | The waveform answer is large JSON |
| Uploads, import, browse | 6 | Audio and MIDI uploads, no size limit |
| Samples | 1 | Audio upload, no size limit |
| Playlists | 5 | No |
| Lighting shows, validate, evaluate | 9 | No |
| Fixture types and GDTF | 8 | GDTF import, up to 272 MB |
| Fixture test | 3 | No |
| Venues, MVR, assets | 13 | MVR import and export; 3D models and thumbnails |
| Bluetooth pairing | 3 | No |
| Devices and calibration | 7 | No |
| Status, lock, restart | 4 | No |

Three facts shape the plan:

- **REST handlers do not go through gRPC.** They call the player, its
  config store or the filesystem directly. So about fifteen RPCs that already
  exist (playlists, config sections, profiles, the lock, the song list)
  have a REST twin with its own code and its own lock table.
- **The MCP server links the REST modules** in process, for the same
  writes. Whatever replaces a REST handler has to serve it too.
- **Who else uses REST:** the hardware harness (ten routes), the systemd
  test (two `curl`s), the docs (a "REST API" section with no endpoint list
  and no promise). The phone app uses none.

## 3. What a browser can and cannot do

- **Unary calls and server streams work** over gRPC-Web. That is every RPC
  the player has.
- **A browser cannot stream a request.** So an upload cannot be a client
  stream. This is why file bytes stay on HTTP (§5).
- **Six connections per host on HTTP/1.1.** Every open stream holds one.
  Five separate feeds would leave one connection for everything else, and
  a second tab would hang. So the feeds share one stream (§4).
- **`<img>` and the 3D loader want a URL.** Models and thumbnails stay
  plain `GET`s.

## 4. One stream for what changes

```proto
rpc Watch(WatchRequest) returns (stream WatchEvent);

message WatchRequest {
  repeated Topic topics = 1;   // what this page shows
}

message WatchEvent {
  oneof event {
    Playback playback = 1;         // what the websocket's `playback` says
    LogLines logs = 2;
    Waveform waveform = 3;
    ConfigChanged config_changed = 4;
    ShowReload reload = 5;
    HardwareStatus hardware = 6;   // replaces the 5 s poll of /api/status
  }
}
```

- **One stream per tab**, opened with the topics the tab shows. A page that
  needs another topic reopens it; that costs one round trip.
- **Sent on change**, with the cadences the websocket has today (`playback`
  five times a second while playing).
- **`WatchStage` stays its own stream.** It is heavy, only the lighting
  pages want it, and the phone already reads it. A tab holds two streams at
  most, of six connections.
- **`WatchStatus` stays for the phone.** It is the compact message
  Bluetooth can carry. `Playback` says more (tracks, sections, the
  waveform's song), and the app can move to `Watch` over Wi-Fi later if it
  wants what the web UI sees.
- **New topics are new `oneof` cases.** An older page ignores them.

## 5. Files

Calls are gRPC; bytes are HTTP.

**Uploads** (tracks, samples, GDTF, MVR):

1. The page calls an RPC that says what is coming: `BeginUpload` with the
   purpose (a track of this song, a sample, a GDTF to import), the file
   name and the size. The lock and the name are checked here.
2. The answer is a path on the web server and an upload id. The page `PUT`s
   the bytes there, with progress as today.
3. The page calls the RPC that uses the file (`AddTrackFiles`,
   `ImportGdtf`, `InspectMvr`, `ImportMvr`) with the upload id. That call
   answers what the REST route answers today.

An upload nobody claims is deleted after a few minutes. The id is good for
one file and one purpose.

**Downloads:**

- 3D models and thumbnails stay `GET /assets/…`, immutable and cached, the
  path the scene data already names. `GetLightingAsset` stays for the
  phone.
- MVR export: `ExportMvr` answers a download path; the browser fetches it.

**Not files:** song YAML, lighting DSL and venue text are strings in
messages. The waveform is a message (it is numbers, and it gzips).

## 6. Services

`PlayerService` has 44 RPCs and would pass 110. Split by area, all in
`player.v1`, all on both ports:

| Service | Holds |
|---|---|
| `PlayerService` | Performance: transport, sections, gains, ambient, the lock, status, the streams. What it has today stays, so nothing an app or script calls moves |
| `ConfigService` | Config sections, profiles, validation, the config store |
| `LibraryService` | Song files, song directories, browse and import, the editing side of playlists |
| `LightingService` | Shows, validation and evaluation, fixture types, venues, MVR, fixture test |
| `DeviceService` | Audio and MIDI devices, calibration, Bluetooth pairing, controller restart |

- **Bluetooth is unchanged.** Its allowlist names `PlayerService` methods.
  A new service is not reachable over Bluetooth unless a method is added to
  the list on purpose.
- **Over Wi-Fi the phone could call all of it**, as §4.1 of the phase 3 doc
  already accepts for the web UI's network.

## 7. One way to do the common things

| | REST today | gRPC |
|---|---|---|
| Someone else changed it | `If-Match`, `version`, `expected_checksum`, 409 | An `expected_revision` field on the request, `ABORTED` with the current revision. As `SavePlaylist` does |
| Locked | 423 from a middleware keyed by route | `FAILED_PRECONDITION` from the gRPC layer keyed by RPC. The web table in `lock_policy` is deleted at the end |
| Not valid | 400 with a JSON body | `INVALID_ARGUMENT`, with the per-line problems in the answer where the editor shows them (validate calls answer normally with a list) |
| Not found | 404 | `NOT_FOUND` |
| Versioning | None | Field numbers. A new field is ignored by an older peer; nothing is renumbered |

## 8. How to get there

The rule: the web UI works after every PR, and each area is deleted from
REST in the PR that moves it, with its mock routes and its lock entries.
No area is served both ways for longer than one PR.

Under the handlers, each area's logic moves into a plain function layer
(no axum types) that the RPC and the MCP server both call. The REST
handlers are mostly that already; the move is mechanical.

| Step | What | Why here |
|---|---|---|
| G0 | Delete the 10 routes nothing calls. The test helper: answer any RPC from a spec by method name, as `stage-stream.ts` does for `WatchStage` | Cheap, and every later step needs the helper |
| G1 | `Watch`. The websocket and the status poll are deleted | The feeds are the part users see, and it removes a transport |
| G2 | The RPCs that already exist: playlists, the lock, config sections, profiles, the song list. The web UI calls them; their REST twins are deleted | No new proto, and it removes the duplicated code first |
| G3 | `ConfigService`: what is left of config and profiles | Small, text only |
| G4 | `DeviceService`: devices, calibration, Bluetooth, restart | Small, and the harness moves with it |
| G5 | `LibraryService`: song files, browse, import, waveform, tempo guess | Medium |
| G6 | `LightingService`: shows, validate, evaluate, fixture test | Medium |
| G7 | `LightingService`: fixture types and venues | The largest handlers (8,800 lines with tests) |
| G8 | Uploads and downloads (§5): tracks, samples, GDTF, MVR | Needs the new upload path; last because it is the only new mechanism |
| G9 | Delete `/api`, `rest.ts`, the web lock table, the mock server's REST. Docs | The end state |

Size: about 9,900 lines of handler code get a new front, 1,300 lines of
web UI helper are rewritten, and the 52 spec files that stub REST are
touched. Around twenty PRs. The specs are the bulk of the work, not the
Rust.

## 9. What changes for someone outside

- **The REST API goes.** The docs call it comprehensive but list no
  endpoints. Anyone who scripted against it moves to `grpcurl` or a
  generated client. The changelog says so under Removed.
- **Health checks.** `curl /api/status` is how the systemd test and people
  check a player is up. Either `grpcurl … Status`, or one plain `GET
  /healthz` that answers 200 and nothing else. Recommended: `/healthz`. It
  is not an API.
- **Reverse proxies.** A proxy in front of the web UI must pass streamed
  responses unbuffered, as it must for the websocket today. One line in
  the docs.
- **The harness** already has a gRPC client; its ten REST calls move as
  their areas do.
- **Who may call what is unchanged.** No sign-in on the network, pairing on
  Bluetooth, the lock on both.

## 10. Decisions

1. **Services split by area** (§6), not one large `PlayerService`.
   Recommended: yes.
2. **One `Watch` stream beside `WatchStage`** (§4). Recommended: yes.
3. **File bytes over HTTP at paths gRPC hands out** (§5). Recommended: yes;
   a browser leaves no other way for uploads.
4. **Each area leaves REST in the PR that moves it** (§8), so a release cut
   mid-way has a REST API with holes. The alternative keeps every REST
   route until G9 and deletes them together: a cleaner release note, and
   two implementations to keep in step for the whole migration.
   Recommended: delete as we go, and cut no release until G9 unless one is
   needed.
5. **A `/healthz` route** (§9). Recommended: yes.
6. **Start with G0 and G1.** Recommended: yes.

## 11. What is not known

- Whether `Playback` five times a second over gRPC-Web costs more in the
  browser than JSON on a websocket. The stage, at twenty frames a second,
  did not (#730: median 50 ms between frames on both).
- How long the waveform message is for a long song, and whether it wants
  to be a stream of chunks.
- Whether Connect's browser client reconnects a server stream cleanly
  after the laptop sleeps. The stage feed has its own backoff; `Watch`
  would share it.
