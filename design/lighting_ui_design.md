# Lighting in the web UI: its own area

*Design doc, draft 1 — 2026-09-29. Follows the UX pitch canvas
(seven mockups: hub, fixture types, venue editor, MVR import, show fit,
preview, MVR export). Decided with Mike: lighting leaves Config for a
top-level area of its own. This doc fixes the information architecture and
the build order, and specifies phase L1 in detail.*

## 1. Why

Lighting grew from a corner of Config into a feature set of its own: fixture
types (hand-written or from GDTF), venues with positions and focus points,
logical groups, show files, the stage plot, Stage 3D, and MVR import and
export. Today those are spread across four places:

| Where | What |
|---|---|
| Config → a profile → Lighting | fixture types, venues, groups, current venue, GDTF import |
| Dashboard | the stage plot, which is also the position and focus-point editor |
| `#/stage` | Stage 3D, reached from a button on the dashboard's stage card |
| CLI / MCP only | MVR import and export |

A user has to know how these connect before a show lights anything, and when
one link is missing — an untagged venue, a universe with no output — the show
plays to a dark rig with warnings in a log. The pitch's answer: one Lighting
area organised around the question *will my show reach the lights?*

## 2. Goals and non-goals

**Goals.**

- One top-level **Lighting** area holding everything lighting, reachable in one
  click from the nav, on desktop and phone.
- Every piece of lighting state is edited in exactly one place.
- The build is incremental: each phase ships on its own and leaves the UI
  coherent.

**Non-goals.**

- No change to files, formats, the DSL or the engine. This is navigation and
  presentation over what exists.
- The song-level lighting editor (a song's timeline and cues) stays with the
  song; Lighting links to it.
- DMX *hardware* settings (`dmx.universes`, olad ports, playback delay) stay in
  Config with the other hardware. Lighting reads them to say whether output
  reaches the lights (§5, L2) and links to Config to change them.

## 3. What belongs to whom

Lighting state splits along a line that already exists in the code:

- **Project files** — fixture types (`lighting/fixture_types/`) and venues
  (`lighting/venues/`). One set per project, whatever the profile. Lighting
  owns them outright.
- **Profile settings** — `dmx.lighting` in a hardware profile: the logical
  groups and `current_venue`. They differ per machine (the rig at home and the
  rig on tour select different venues). Lighting edits them *for a chosen
  profile*, defaulting to the profile the player is running (the one whose
  `hostname` matches, as `/api/status` reports), with a picker for the others.
  Saving goes through the same API Config uses (a profile file under
  `profiles_dir`, or the inline profile in `mtrack.yaml`).

Config's profile editor keeps the profile's hardware and shows lighting as a
read-only summary — venue, group count — with a link into Lighting for that
profile. There is never a second editor for the same state, so two unsaved
edits of one profile cannot race.

## 4. Information architecture

```
Lighting                    #/lighting
├── Overview                #/lighting            (L1: summary + links; L2: readiness hub)
├── Fixture types           #/lighting/fixtures
├── Venues                  #/lighting/venues     (list, editor, stage plot)
├── Groups                  #/lighting/groups     (per profile: groups, current venue)
└── 3D                      #/lighting/stage      (Stage 3D)
```

- The nav gains **Lighting** between Playlists and Config.
- `#/stage` redirects to `#/lighting/stage`, so bookmarks and the dashboard's
  3D button keep working.
- The dashboard keeps its stage card as a live view; editing positions and
  focus points moves to Venues, and the card links there.

## 5. Phases

Each phase maps to screens in the pitch canvas.

| Phase | What | Pitch screen |
|---|---|---|
| **L1** | Relocate: the Lighting area and its routes, today's editors moved in, Config and dashboard link across. No new capability. | — |
| **L2** | Overview becomes the readiness hub: fixture types → venue → groups → shows → output, each a check mtrack already runs (lint, load reports, the olad patch probe). | Lighting hub |
| **L3** | Fit your shows: groups the shows use vs. fixtures each resolves, tag suggestions, focus points the shows aim at, missing universe outputs. | Fit your shows |
| **L4** | Venue editor: multi-select, arrange (align, space, mirror), aim (a direction or a focus point, tilt) with the rotation computed. | Venue editor |
| **L5** | MVR import and export in the browser: origin picked on a plan; download with the pre-viz warning. | MVR import, Export |
| **L6** | Preview: Stage 3D with a timeline scrubber and honest caveats; the plain-language mode picker for GDTF import. | Preview, Add a fixture type |

L3 is the highest-value step after L1: an imported venue is untagged, so every
group a show uses finds nothing, and today nothing says so where a user looks.

## 6. L1 in detail

**Routes and nav.** As §4. `App.svelte` routes `#/lighting*` to a new
`pages/Lighting.svelte` holding the sub-navigation; `#/stage` rewrites to
`#/lighting/stage`. The page title follows the sub-page ("Lighting - Venues").

**Components.** `components/config/LightingSection.svelte` (≈2,000 lines, three
sub-tabs) splits into:

- `components/lighting/FixtureTypesPanel.svelte` — list, editor, GDTF import;
  project files.
- `components/lighting/VenuesPanel.svelte` — list and editor; project files.
- `components/lighting/ProfileLightingPanel.svelte` — groups and
  `current_venue`, bound to one profile's `dmx.lighting`.

`Lighting.svelte` mounts them: Fixture types and Venues directly; Groups with a
profile picker and its own load and save; 3D mounts `pages/Stage3D.svelte`. The
Venues sub-page also shows the stage plot (`StageView`) in its editing mode.
The profile loading and saving the Groups page needs is the logic
`ProfileEditor` has today, extracted to `lib/` so both use one path.

**Config.** `ProfileEditor` drops `LightingSection` for a summary card: the
profile's current venue and group count, and "Edit in Lighting" linking to
`#/lighting/groups?profile=<name>`.

**Dashboard.** The stage card keeps its live view. Its edit controls move to
Venues; the card links there.

**Dead code.** `pages/LightingEditor.svelte` is not routed anywhere (its last
changes were #295, #388 and #406) and is removed.

**Tests.** The Playwright specs that reach lighting through `#/config`
(`config-lighting`, `fixture-types`, `venues`, `stage-3d`, `stage-cells`, and
any other that navigates there) move to the new routes. New specs cover the
nav item, each sub-route, the `#/stage` redirect, the profile picker's default,
and that Config shows the summary and link rather than an editor.

**Docs.** `docs/src/interfaces/web-ui.md` gains a Lighting section; the Config
section loses its lighting subsection and points there.

## 7. Decisions for review

1. **Lighting is a top-level area** — decided (2026-09-29).
2. **Profile-scoped lighting (groups, current venue) is edited in Lighting**,
   for a chosen profile defaulting to the running one; Config shows a summary
   and a link. *Alternative:* keep groups in Config and move only project
   files — rejected, because groups are what a show resolves through, and the
   L3 fit screen edits tags and groups together.
3. **DMX hardware stays in Config.** Lighting reads it and links to it.
4. **The dashboard's stage card stops being an editor** and links to Venues.
   *Alternative:* keep editing in both — rejected by §2's one-place rule.
5. **`#/stage` redirects** rather than being removed.

## 8. L2 in detail: the readiness hub (draft 1, 2026-09-29)

The Overview becomes the pitch's hub: five checks in the order a show needs
them, each saying what is wrong in plain words and linking to where it is fixed.
Every check is one mtrack already runs somewhere; L2 gathers them in one place.

### 8.1 The five checks

| Check | Ready when | Reads | Fixed at |
|---|---|---|---|
| **Fixture types** | every fixture in the current venue has a type that loaded (a GDTF type's archive expanded) | the loaded lighting system | Fixture types |
| **Venue** | a current venue is selected and loaded | the loaded lighting system | Groups (choose), Venues (edit) |
| **Groups** | every group a show uses finds at least one fixture in the current venue | show files + group resolution, as lint's `empty-group` | Groups; L3 later |
| **Shows** | every song's light shows parse, and lint finds nothing that stops a cue doing what it says (`capability-gap`, `unbound-focus-point`) | each song's shows through the same lint MCP's `validate_lighting` runs | the song's lighting editor |
| **Output** | every universe the venue's fixtures use has an output under `dmx.universes`, and olad reports an output port patched to each | lint's `unconfigured-universe`, the olad patch probe (#456) | Config → DMX; olad (`ola_patch`) |

A check is **ready**, **needs attention** (the show plays but something will not
do what it says), **blocked** (nothing reaches the lights: no venue, no DMX, a
show that does not load), or **unknown** (its input is missing, e.g. olad's web
server does not answer). Lint's advisory kinds (`unused-parameter`,
`tempo-grid-mismatch`, `past-end-of-song` …) are listed under the song but do
not change a check's state.

When a check's input is missing it is **unknown**, not ready: fixture types,
groups and output without a venue, and the venue-shaped checks without DMX.
The venue check itself is **blocked** when none is selected. An unresolved
fixture type is **blocked**, not merely needs attention, because the engine
registers a venue's fixtures all or nothing: one fixture whose type did not
load stops the whole venue registering. A check that mixes severities says
how many of each ("1 song won't load, 1 to check").

### 8.2 `GET /api/lighting/readiness`

Facts, not verdicts — the UI turns them into the states above, so wording and
rules live in one place and are translatable.

```json
{
  "dmx": true,
  "venue": {"name": "built-in", "fixtures": 8, "placed": 8, "focus_points": ["center"]},
  "fixture_types": {"in_use": ["Astera-PixelBrick"], "unresolved": [{"fixture": "Brick9", "type": "Nope", "reason": "…"}]},
  "groups": [{"name": "front_wash", "fixtures": 4, "songs": ["Legions of Decay"]}],
  "shows": [{"song": "Esaweg", "files": ["show.light"], "error": "Effect 'static' requires a 'duration' …",
             "warnings": [{"kind": "capability-gap", "message": "…"}]}],
  "output": {"universes": [1], "unconfigured": [], "olad": {"reachable": true, "unpatched": []}}
}
```

- `venue` is `null` when none is current; `dmx` is `false` when the running
  profile has no DMX. Without DMX, `venue`, `fixture_types`, `groups` and
  `output` are empty; `shows` is not (see below).
- `shows` lists every song with lighting, with or without DMX: a show that
  does not load is as broken on a laptop as on the rig, and a laptop is where
  shows are written. Without a venue only the lint checks that need none run.
  A song whose show does not parse never loads (`Song::new` fails as a whole),
  so it is not among the loaded songs: its entry comes from the player's
  `SongLoadFailure`, with the parser's own message as `error` and `files: []`.
- A song's `warnings` do not include `unconfigured-universe`: that finding is
  about the venue, not the song, and the `output` section carries it. (MCP's
  `validate_lighting` still reports it, since it validates one show.)
- `fixture_types.in_use` lists only the types that loaded.
- `olad` is `null` without DMX, and also when there is nothing to ask about (no
  venue, no configured universes). The probe covers the universes the venue's
  fixtures use that have a configured output, else every configured universe.
  `reachable: false` when its web server does not answer within the probe's
  deadline (the same two seconds as the startup probe).

The lint context that MCP's `validate_lighting` builds (group counts and
capabilities, focus points, universe coverage) moves into one function both
call, so the hub and MCP can never disagree. The olad probe gains a function
that returns what it found instead of only logging it; the startup warning
keeps using it.

### 8.3 The page

The mockup's hub: the five checks as a numbered strip, then a **Needs
attention** list — one entry per finding, grouped by check, each with its fix
link — and the live stage card (not editable). A blocked or attention check
shows its count; ready shows a one-line summary ("8 fixtures, all placed").
The page refreshes when the venue or config reloads (the existing websocket
metadata broadcast), not on a timer.

## 9. L3 in detail: fit your shows (draft 1, 2026-09-30)

An imported venue arrives with no tags, so every group a show uses finds
nothing; a console's focus points have the console's names; and its universes
have no output on this profile. L2 says so. L3 fixes it in one place, on a page
at `#/lighting/fit`, reached from the hub's Groups, Shows and Output findings
(and, in L5, as the last step of an MVR import).

### 9.1 What the page shows

Three columns, as in the pitch's "Fit your shows" screen:

1. **Groups your shows use** — every group any song's shows target, with the
   tags its constraints need (`AllOf`, `AnyOf`; `Prefer` shown as "prefers"),
   how many fixtures it finds in the current venue, and which songs use it.
   Empty groups first. Selecting a group drives the other two columns.
2. **The plan** — the stage plot, with the selected group's members
   highlighted; clicking a fixture toggles it in the pending selection.
3. **Fixes** — for the selected group, a **suggestion** (below) with
   **Apply** and **Pick others**; then the **focus points the shows aim at**
   that the venue lacks, each with **Place on plan** (a click on the plot
   creates the point with that name); then **Output**: universes the venue's
   fixtures use that the running profile has no output for, with
   **Add to profile**, and olad's unpatched ports with the `ola_patch` line
   for each universe (the device and port are the user's to fill in).

A footer repeats L2's Groups check ("2 of 6 groups find fixtures").

### 9.2 Suggestions

A suggestion is a set of fixtures and the tags that would put them in the
group, with a reason a user can check. It is computed on the server so the MCP
tools can offer the same one, and it is explainable or it is not offered:

- **Needs:** the tags the group's `AllOf` require, plus one of `AnyOf`.
- **Candidates:** fixtures whose capabilities fit what the shows ask of the
  group: a `move` needs pan and tilt, a colour cue needs colour, a `strobe` a
  strobe channel, `per: cell` cells. Capabilities come from
  `FixtureInfo::capabilities()`, needs from the same rules as lint's
  `capability-gap`.
- **Cluster:** candidates are grouped by fixture type, then by where they
  hang: the same height band (deck, low, truss: z < 0.5, < 2.5, above) and
  the same depth band (downstage, mid, upstage by thirds of the venue's
  y-extent) when the venue places them. The largest cluster is the
  suggestion; the rest are offered under **Pick others**.
- **Reason:** "the 11 MAC Viper AirFX on the upstage truss can move and
  colour, which `movers` needs".

No cluster fits → no suggestion, and the page says which need no fixture
meets ("nothing here has a strobe channel").

### 9.3 `GET /api/lighting/fit`

Facts plus suggestions; the actions reuse what exists.

```json
{
  "venue": {"name": "basic-festival", "fixtures": [{"name": "…", "type": "…", "tags": [], "position": [x,y,z], "capabilities": ["color","pan_tilt"]}], "focus_points": ["…"]},
  "groups": [{"name": "movers", "needs": {"all_of": ["moving_head"], "any_of": [], "prefer": []},
              "fixtures": [], "songs": ["…"], "wants": ["move", "color"],
              "suggestion": {"fixtures": ["…"], "tags": ["moving_head"], "reason": {"count": 11, "type": "MAC Viper AirFX", "where": "upstage truss", "can": ["move", "color"]}},
              "others": [{"fixtures": ["…"], "type": "Robin Esprite", "where": "…"}]}],
  "focus_points_wanted": [{"name": "drummer", "songs": ["…"]}],
  "output": {"unconfigured": [11, 12], "unpatched": [1], "ola_http_port": 9090}
}
```

- **Apply** writes the tags to the venue file through the existing venue save
  (positions, rotations, focus points and provenance preserved, as the Venues
  editor does), then the engine reloads it as it does after any venue save.
- **Place on plan** creates a focus point through the same path the Venues
  editor's **+ Focus point** uses, with the wanted name preset.
- **Add to profile** appends `{universe, name: "u<N>"}` entries to the running
  profile's `dmx.universes` through `lib/profileStore.ts`, then reloads the
  profile the way Config's save does.

The `reason` is structured so the page can word and translate it; the MCP
tool (`suggest_group_tags`, L3 too) prints the same fields.

### 9.4 Out of scope

Editing constraints, creating groups, renaming a console's focus points in
bulk, and any automatic tagging without a click: a suggestion is applied by
the user, never by the import.

### 9.5 As built

Where the build settled a point §9.1–§9.4 left open or got wrong:

- **Wants follow lint exactly.** A `move` wants pan **or** tilt (lint's
  `capability-gap` rule, so applying a suggestion clears the gap, not "pan and
  tilt"). A `dimmer` or `pulse` cue wants a **dimmer** (`wants` value `dimmer`),
  met by a dimmer channel or colour channels. Colour, strobe and `per: cell`
  are as in §9.2. A candidate satisfies every want.
- **Several `AnyOf` constraints are flattened** into one list; the suggestion
  applies the first tag of it, plus every `AllOf` tag.
- **Suggestions are computed only for groups the profile defines that find no
  fixtures.** An undefined group, a group with no `AllOf`/`AnyOf`, and a group
  that already finds fixtures get none.
- **Fields added to §9.3:** per group `defined`, `unmet` (the wants no fixture
  meets) and `unmet_together` (each want is met by some fixture, none meets
  them all); `output.reachable` (whether olad answered; `null` when nothing was
  asked); and structured `height` (`deck`/`low`/`truss`) and `depth`
  (`downstage`/`mid`/`upstage`) beside `where` in `reason` and `others`, so the
  page words and translates the place itself. The depth band is absent when the
  venue's y-extent is nil.
- **"+ Focus point" lives in `StageView`,** not the Venues editor; Place on
  plan reuses its save path (`createFocusPoint`).
- **A profile write reloads on its own.** Add to profile writes through
  `profileStore` and the server reloads; the page does nothing extra.
- **Test hook:** when the plan is used for choosing fixtures, `StageView` puts
  their pixel positions on the canvas as `data-positions`, so tests can click a
  fixture without re-deriving the layout. The page also lists every fixture as
  a checkbox, so selection does not depend on the canvas.

## 10. L4 in detail: arrange and aim in the venue editor (draft 1, 2026-09-30)

Placing a rig by hand today means typing coordinates and solving rotations
from the formula in the docs. This week's house rig took four rounds of that:
aim each brick at a point, then face each side inward, then align each side
on a line, then space the front evenly and mirror the sides. L4 makes each of
those a control on the Venues page, working on a selection on the plan.

### 10.1 Selection

The Venues plot (StageView, `editable`) gains multi-select: click selects,
shift-click extends, drag on empty deck marquee-selects, Escape clears. The
fixture list beside the plot mirrors the selection (and selects from it). An
inspector panel replaces the per-fixture form for whatever is selected: one
fixture shows its fields; several show the shared ones (type, tags in
common) and the Arrange and Aim tools.

### 10.2 Arrange (two or more selected)

All operations write positions through the existing venue save and reload,
as dragging does; each is one undoable save.

- **Align on a line** — the selection's dominant spread decides the line:
  spread more along y than x → align x to the selection's mean x (a side
  column); otherwise align y to the mean y (a row). Z untouched.
- **Space evenly** — along the same dominant axis, the two extreme fixtures
  stay and the rest are spread evenly between them, in their current order.
- **Mirror across centre** — each selected fixture's x becomes −x, about the
  stage centre line x = 0. (The venue's origin is downstage-centre by
  definition, so the centre line needs no setting.)
- **Nudge** — arrow keys move the selection 0.1 m, shift for 1 m.

### 10.3 Aim (fixed fixtures)

For fixtures with no pan or tilt, a rotation is the only aim they have. The
Aim section offers, for the selection:

- **Face a direction** — a direction across the deck (stage left, stage
  right, upstage, downstage, or a bearing in degrees) and a **tilt up from
  the floor** (0° level, 90° straight up; negative aims down, for a hung
  fixture). Rotation = `(90 + tilt, 0, bearing)` in the convention of §3 of
  the docs' "Mounting and pose convention": bearing −90 faces +x, 0 faces
  +y, 90 faces −x, 180 faces −y.
- **At a focus point** — pick one of the venue's focus points; each selected
  fixture gets the rotation that points its rest beam at it:
  `a = acos(−d.z)`, `b = atan2(−d.x, d.y)`, `rotation (a, 0, b)`, with
  `d` the unit vector from the fixture to the point (the docs' worked
  example). A fixture at the point itself is skipped and said so.
- **Rotation** — the raw triple, shown always, editable as the escape hatch.

For movers (pan and tilt), the section offers the two mountings the docs
name — **hung facing downstage** `(0, 0, 180)` and **standing on the deck**
`(180, 0, 0)` — and the raw triple. Aiming a mover is the show's job.

The plan draws each fixed fixture's beam from its rotation (as it does now),
so an aim is checked at once; Stage 3D shows the same.

### 10.4 Where the math lives

Both rotation solvers are pure functions in `src/lib/stage/aim.ts`, unit
tested against the docs' worked example (brick at `(0.09, −0.1, 0)` aimed at
`(0.7, 1.9, 1.5)` → `(125.7, 0, −17.0)`) and against the perimeter rig
(`(110, 0, ∓90)` for the sides, `(110, 0, 0)` for the front). Arrange
operations are pure functions in `src/lib/stage/arrange.ts`, tested on the
house rig's measured positions (the same numbers as the venue in
backing-tracks: align → x = −5.345 / 6.855, space → 2.46 m, mirror about 0).

### 10.5 Out of scope

Rotating a whole selection as a body, snapping to scenery, and any change to
the venue file format. Aiming through a GDTF rig's own geometry stays a
`move` concern; a fixed fixture with a pitched lens is drawn where it really
points, as Stage 3D already does.

### 10.6 As built

Where the build settled a point §10.1–§10.5 left open or got wrong:

- **The inspector applies to the plot's venue,** the current one the engine
  has loaded and the websocket describes. `VenuesPanel`'s editor still edits
  any venue in the list, and creates and removes fixtures; it was not
  replaced. One selected fixture shows the same fields there (name, type,
  universe, channel, tags) with an **Apply**.
- **Stage left is bearing −90,** toward +x, as the venue files' comments say
  (+x is stage-left); stage right is 90, upstage 0, downstage 180.
- **Mirror mirrors the aim.** Reflecting x also reflects the beam, so a
  rotation `(rx, ry, rz)` becomes `(rx, −ry, −rz)` in the same save; the
  perimeter's left side `(110, 0, −90)` becomes `(110, 0, 90)`. A fixture with
  no rotation stays without one.
- **Capabilities come from the metadata.** Each fixture in the websocket
  metadata carries `capabilities` (`color`, `pan_tilt`, `strobe`, `cells`),
  named as the fit endpoint names them. A fixture with `pan_tilt` is a mover,
  so a GDTF-referential type, which lists no channels in the fixture-types
  API, is still one.
- **Plot fixes the build needed.** The plot redraws on a `ResizeObserver`,
  because the inspector narrows it after the first layout. Focusing the plot
  on a press uses `preventScroll` and reads the press position first, since
  focusing scrolled the page and put the hit test off the fixture. A click
  that does not move a fixture saves nothing, and arrow-key nudges are
  debounced so a burst is one save.
