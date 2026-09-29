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
