# Lighting in the Web UI

The **Lighting** item in the top navigation holds everything about lights, in one place. It has
five pages, shown as tabs across the top, and an import wizard that belongs to Venues. Each has
its own address, so you can bookmark it. New to it? [First Light](first-light.md)
goes from a GDTF to a show through these pages.

| Page | Address | What it holds |
|---|---|---|
| Overview | `#/lighting` | The readiness checks: will your show reach the lights? |
| Fixture types | `#/lighting/fixtures` | The kinds of fixture in your rig: GDTFs and hand-written types |
| Venues | `#/lighting/venues` | Where fixtures sit, and the stage plot |
| Groups | `#/lighting/groups` | Logical groups and the current venue, for one hardware profile |
| Fit shows | `#/lighting/fit` | Fit your shows to the venue and the rig: tags, focus points, outputs |
| Import an MVR | `#/lighting/import` | The MVR import wizard (from Venues) |

What a page has open is part of its address too: a fixture's page is `#/lighting/fixtures/<name>`,
the venue selected for the stage card is `#/lighting/venues/<name>` (`?edit` when its form is open,
`?view=3d` when the card shows 3D, with `&mode=preview&song=<name>&t=<seconds>` for a previewed
moment), and a new form is `?new=venue`, `?new=light` or `?new=fixture`. Groups takes
`?profile=<name>` and Fit shows `?group=<name>`. The browser's Back, a reload and a
shared link land where they say, and a page's tab always returns to its list.

Two kinds of state live here, and they are stored differently.

**Project files** — fixture types and venues — are the same whichever hardware profile is
running. Fixture Types and Venues edit them directly, in `lighting/fixture_types/` and
`lighting/venues/` unless the running profile's lighting settings move them (`directories`).
The engine reads the same places, so a new project needs no directories setting: what you make
here is what the show uses ([Where the lighting files live](configuration.md#where-the-lighting-files-live)).

**Profile settings** — logical groups and the current venue — belong to one hardware profile,
because the rig at home and the rig on tour select different venues. The Groups page edits them
for the profile you pick. It starts on the profile the player is running (the one whose
`hostname` matches the Status page). When no profile matches this host, or the status cannot be
read, nothing is selected: the page says so, and Save stays disabled until you choose a profile
from the list. (It never falls back to the first profile, which could be another machine's.) A
link such as `#/lighting/groups?profile=my-host` opens it on another profile. Saving goes through
the same API as the Config page: a profile file when the config sets `profiles_dir`, otherwise the
profile inside `mtrack.yaml`. A profile with no DMX section has nothing to edit here; the page
links to Config to enable lighting first.

Saving rewrites the profile. **Comments in the YAML are not kept**, because the YAML library
does not preserve them. A profile *file* keeps the keys mtrack does not model (they are copied
across from the existing file); the profile inside `mtrack.yaml` is rewritten whole, so it loses
comments and unknown keys alike. Keep notes you care about somewhere other than a profile the web
UI saves. A profile file saved from two pages or tabs at once is protected: the page reads the
file's version, and a save made after the file changed is refused. The page reloads the profile
and asks you to make the change again.

## Overview: will your show reach the lights?

The Overview answers one question, and says what to fix when the answer is no. It shows five
checks, numbered in the order a show needs them, then a **Needs attention** list, then the live
stage. Every check is something mtrack already works out; the Overview gathers them in one place.

![The Overview: Fixture types and Shows ready, Venue and Groups needing attention, Output blocked;
below, the overlap, the group that finds no fixtures and the unpatched universe, each with a link
to fix it](../images/lighting-overview.png)

| Check | Ready when |
|---|---|
| 1. Fixture types | Every fixture in the current venue has a type that loaded. A type made from a GDTF file counts only if the file could be read. |
| 2. Venue | A current venue is chosen and loaded. |
| 3. Groups | Every group your songs' shows use finds at least one fixture in the current venue. |
| 4. Shows | Every song's light shows load, and nothing stops a cue from doing what it says (for example, a strobe sent to fixtures that cannot strobe, or a cue aimed at a focus point the venue does not have). |
| 5. Output | Every universe the venue's fixtures use has an output under `dmx.universes`, and olad has an output port patched to each. |

Each check is in one of four states, always written out in words and not only shown by colour:

- **Ready**: nothing to do.
- **Needs attention**: the show plays, but something will not do what it says.
- **Blocked**: nothing reaches the lights. There is no venue, no DMX output, a fixture type that
  did not load, a show that does not load, or a universe with no output.
- **Unknown**: mtrack could not find out. This happens when a check depends on another that is
  blocked (a fixture type cannot be checked with no venue), and when olad's web server does not
  answer, so the patch cannot be read.

A ready check shows a one-line summary, such as "8 fixtures, all placed". A check that is not
ready shows how many things need fixing.

The **Needs attention** list has one entry per finding, grouped by check, and each entry links
to where it is fixed: Fixture types, Venues or Groups in this area, the song's lighting editor
for a show, and the running profile's DMX settings in Config for output. Some notes from a
song's show checks, such as a cue that starts after the song ends, are listed under the song
marked **Note**. They are worth reading, but they do not change a check's state.

The page checks again when the venue or the configuration reloads, so after you fix something,
the check turns ready without a refresh. A profile with no DMX output shows only the Output check
as blocked, with a link to enable it.

## Fixture types

A fixture you have a GDTF for needs nothing from you but the GDTF. **Import GDTF** takes a `.gdtf`
and that is all: the fixture is listed at once, its page opens, and every one of its modes is
available when you add it to a venue. The result says what you now have ("Imported PB15
PixelBrick (Astera LED Technology) — 30 modes"). Importing the same file again changes nothing and
says so. If another fixture already has its name, it is named after its file as well
("PB15 PixelBrick (pb15)") and the result says so. A different GDTF with the same file name as
one already imported is refused, because other fixtures may use the one that is there. Copying a
`.gdtf` into `lighting/library/` by hand does exactly the same as importing it.

![The Fixture types list: a GDTF fixture's card with its model, maker, mode count and the modes
in use, and two hand-written types with their files and channels](../images/lighting-fixture-types.png)

A fixture from a GDTF has a card that says what it is: its picture (once a 3D model has been made
for it), the manufacturer and fixture name, how many modes it has and its beam, and one pill per
mode the venues use with how many fixtures use it (the three most-used, then "+N more"). Opening it
shows the fixture: a 3D view (drag to turn it; the GDTF's thumbnail stands in when the browser
cannot draw 3D) beside what the GDTF states about it — its file, its mode count, the beam, light
output, power, which venues use it, and the manufacturer's own description. A figure the GDTF does
not state is left out. Below that is every mode, with a filter; a mode a fixture uses says
**N in use**. Choose a mode to see what your shows could do in it, its channels, and which fixtures
use it. A mode mtrack cannot drive is listed greyed with the reason. A venue fixture whose mode is
not in the GDTF is named in red under the facts.

![A GDTF fixture's page: the 3D model, the facts, the mode list with 8: RGBS selected and its
channels, the collapsed Test this fixture panel, and Your settings for this fixture with the
strobe curve](../images/lighting-fixture-page.png)

**Your settings for this fixture** holds what is yours about it: its **name** in venues and shows,
and, for a fixture that can pan or tilt in any mode, its **movement limits** (max pan and tilt
speed, in degrees per second; blank is no limit). A rename rewrites every venue line that names the
fixture, in every venue file, keeping comments and layout, and **Save settings** says first how
many lines in which venues. If any venue file cannot be rewritten (it does not parse, or it changed
meanwhile), nothing is written and the message names the file. A fixture's mode is not a setting
of the fixture: each fixture in a venue names its own.

For a fixture whose GDTF gives its strobe a rate in hertz, the settings also hold its **Strobe
curve**: how a strobe rate becomes a DMX value. Left on **Automatic**, it follows the GDTF — its
own table of steps when it has one ("as the GDTF declares (12 steps)"), else linear in Hz between
its two ends. **Period** is for a unit whose firmware is linear in the flash length, as the Astera
PixelBrick's is: set it there, or a 10 Hz strobe flashes about once every one and a half seconds.
**Linear in Hz** ignores a table. If a 2 Hz strobe does not flash twice a second when you test the
fixture, try another curve. See
[Strobe curve](configuration.md#strobe-curve-strobe_curve).

Deleting a fixture from a GDTF removes its GDTF too, unless another fixture uses the same file.
If a venue uses it, the confirmation says how many fixtures in which venues, and that those venues
will stop loading.

### Testing a fixture

**Test this fixture**, on every fixture's page (from a GDTF or written by hand), answers "does this
light respond when mtrack sends to it?" — and, when it does not, says why. Choose the **mode** the
fixture is set to (a GDTF fixture's first mode mtrack can drive is chosen for you; a fixture written
by hand has no modes), the **universe** its cable is on (one of the running profile's DMX
universes; one with no output patched in olad says so) and its **DMX address**; the panel shows the
addresses it will use ("Uses addresses 1–4"). Nothing is sent until you turn **Send to lights** on,
and then a working fixture lights up full white at once (colour white, brightness full, strobe off;
a moving head stays at its centre pose). Then, in order: quick swatches (red, green, blue, white,
off) and **Blackout**; sliders for brightness, strobe in flashes a second, extra colour channels
(white, amber, …) and pan and tilt in degrees; and, under **Every channel**, a 0–255 slider per
channel. A raw channel you move is marked **manual** and stays where you put it until you reset
it. A control the chosen mode has no channel for is shown as "not in this mode", so switching
between, say, RGB and RGBS shows what the strobe channel adds.

![Test this fixture, live: mode, universe and address, Send to lights on with the live line and
Stop, the red swatch chosen, the sliders, and the "mtrack is sending" line over the "Nothing
happened?" checks](../images/lighting-fixture-test.png)

The values go through the same code a show's effects do — a fixture without a dimmer channel
scales its colour by the brightness, a strobe rate lands in the fixture's strobe range, pan and
tilt in degrees become the coarse and fine bytes of its range — so what the panel sends is what a
show would send. The fixture's 3D view mirrors it.

**What "live" means.** While Send is on, mtrack lays these channels over everything else it sends
to that universe — the show, MIDI-DMX — and says so in the panel and in a banner on every page
("Test output is live on universe 1, addresses 1–4", with **Stop**). The page keeps the test
alive; if nothing does (the tab closed, the laptop slept), it lets go by itself within about five
seconds and the lights go back to whatever the show is doing. Changing the mode, universe or
address, or leaving the page, lets go of the old addresses first. A fixture of the current venue
on those addresses is named: the test overrides it while it runs. The stage views show it doing so: the
plot and the 3D view (on the dashboard and on the Venues page) draw each venue fixture whose
addresses the test covers with the test's values — its colour and, for a mover whose pan or tilt
is covered, where it points — rather than the show's. The plot marks it with a **TEST** badge,
3D tints its label amber, and the Venues page's stage card carries the same "Test output is live"
line as the banner, with **Stop**. Once the test lets go, they show the show again.

![The Venues page's stage card while Brick 2 is under test: the "Test output is live" line with
Open and Stop, and Brick 2 drawn red with a TEST badge and a dashed ring](../images/lighting-under-test.png)

**Nothing happened?** While live, the panel lists the likely causes, the ones mtrack can check
first: olad (the program that sends DMX out of the computer) not answering; the universe having no
output patched in olad, with a link to olad's own page; then questions for the fixture itself — is
its DMX address set to the address you chose, is it set to this mode (named, with its channel
count), is the cable in and the line terminated. One line says exactly what mtrack is sending
("mtrack is sending 255, 255, 255, 0 to universe 1, addresses 1–4"), so "mtrack is not sending"
can be told from "the light is not listening".

**When it is unavailable.** Testing needs the running profile to have DMX output with at least one
universe (the panel links to where that is set), the player to be unlocked (it offers to unlock),
and no song playing. Starting a song or locking the player stops a running test.

**Define a fixture by hand** is for a fixture with no GDTF: it opens the channel-map form (a
`.light` file) or the text of a `.fixture` file, which are your own definitions; see
[the configuration reference](configuration.md).

## Venues

Lists the venues and edits their fixtures: name, type, universe, start channel and tags. Saving
a venue keeps everything the form does not show — fixture positions and rotations, focus points,
and where an MVR import came from.

A fixture from a GDTF has a **Mode** select on its row listing every mode with its footprint
(modes mtrack cannot drive are shown but cannot be chosen), so one venue can mix, say, RGBS and
RGBWS bricks; nothing needs setting up on the Fixture types page first. A new row takes the previous
row's mode when it is the same fixture, otherwise the first mode mtrack can drive, so **Add
Fixture** then **Save** always writes a line with its mode. Changing a row's fixture picks that
fixture's first drivable mode. A line read from a file without a mode is marked **Choose a mode for
this fixture.** and stops the save until one is chosen (the venue does not load without it). A mode
saved by hand that the GDTF does not have is shown as such and kept unless you change it. Each row says which addresses it
occupies ("Addresses 5–9"). A row whose addresses run into another row's (rows on exactly the
same addresses are ganged, which is fine) or past 512 is marked with what it runs into; it does
not stop you typing, and Save asks "Save anyway?" naming them, so a hand-made venue with overlaps
stays editable. Nothing is ever renumbered for you. A save shows at once everywhere on the page:
the venue's card, the plot (its fixtures and count) and the inspector read the saved file, and
the next edit starts from it.

![The venue form: two PixelBricks in 8: RGBS and 13: DIM RGBAWS, each with its Mode select and
the addresses it uses, and a hand-written par; the second brick and the par are marked where they
run into each other](../images/lighting-venue-editor.png)

**Add Fixture** continues the patch from the last row: the new fixture is named `Fixture N` (the
first number not taken), takes the last row's type and universe, and starts straight after it
(its address plus its footprint in its own mode, so a 3-channel fixture at 1 is followed at 4). A fixture
that would run past address 512 starts the next universe at 1. Changing an earlier row never
renumbers the rows after it. Nothing you added is dropped on save: a row with no name, no type,
a name another row has, or a universe or address below 1 stops the save, is outlined with what
to fix, and gets the focus. The fixture type editor treats channel rows the same way, a tag with
no allowed character stays in its box marked, and a group or focus point rename
that is refused (empty, or a name already taken) says so instead of quietly reverting. When the
current venue did not load, the plot on this page shows its file instead of an empty stage, so
the fixture at fault can be selected and fixed in the inspector.

Click a venue in the list to select it; **Edit** opens its fixture form. The plot below shows
the selected venue. When it is the current venue (marked *current*, or when none is selected),
the plot is the live stage. When it is any other venue, for example one you just created, the
plot is a plain view of that venue's file, labelled *not live* and without live fixture colour.
Placing, arranging and aiming work the same way and save to that venue.

Two tabs or pages can edit one venue. Every venue save carries the version of the file it was
based on (a hash of its bytes, sent as `If-Match`). If the file changed since it was loaded, the
save is refused and the page reloads, and you make the change again. Saves never regenerate an
existing file: the file that defines the venue is found by scanning the venue files for its
block (its name need not match the file name) and patched in place, keeping comments and `# TODO`
lines (`#` and `//` comments both). A file that no longer parses is refused with the file's name
so you can fix it by hand; nothing is overwritten.

Beneath the list is the stage plot, the same view as the dashboard's stage card but editable.
Drag a fixture from the tray onto the stage to place it, or drag a placed fixture or a focus pin
to move it. Each drag writes the new coordinates to the venue file, the running engine reloads
the venue, and every open stage view redraws from the file. The **+ Focus point** button adds a
pin, and the list beneath the plot renames or deletes them. Focus points are the stage points a
show aims at, so name them for what they are ("drummer", "center-stage"). A fixture dragged from
the tray is hung at 3 m; edit the venue file to correct its height.

**Selecting.** Click a fixture on the plot to select it, shift-click to add or remove one, or
drag a box on the empty deck to select what it encloses (shift adds the box to the selection).
**Escape** clears it. The inspector beside the plot (below it on a phone) lists every fixture with
a checkbox that mirrors the selection and drives it. Dragging a fixture moves it as before, and a
click that does not move saves nothing.

The inspector shows what is selected:

- **One fixture** shows its own fields: name, type, universe, start channel and tags, saved by
  **Apply**. A fixture from a GDTF also has a **Mode** select listing every mode with its
  footprint (modes mtrack cannot drive are shown but cannot be chosen). Choosing a mode saves it
  at once; a line read without one shows **Choose a mode** until it is fixed here. Below it a
  strip shows the fixture's universe around it, one cell per address: this fixture, the other
  fixtures (hover a cell for who), and overlaps in red. A mode or an address that would run into
  another fixture's addresses is refused before anything is saved, naming the fixture in the way
  and the largest mode that fits — for example *Not saved. 13: DIM RGBAWS needs addresses 9 to
  15, and Brick4 already uses some of them. Move Brick4 or pick a mode of 4 addresses or fewer.*
  Fixtures patched to exactly the same addresses (ganged) are not in each other's way. The check
  reads the venue's files, so it works with no DMX output running. A file that already has
  overlaps still saves; the Overview lists them.
- **Several** show the type and tags they share, and the two tools below.

![The live stage plot with Brick 5 selected: the inspector's fixture list, its name, type and
Mode select, and the patch strip with its addresses and an overlap in red](../images/lighting-venue-plot.png)

**Arrange** (two or more placed fixtures) writes positions. **Align on a line** puts a side
column at its mean x, or a row at its mean y, whichever way the selection spreads more.
**Space evenly** keeps the two extreme fixtures and spreads the rest evenly between them along
the same line, in their current order. **Mirror across centre** turns each x into its negative,
about the centre line at x = 0. While the plot has keyboard focus, the arrow keys nudge the
selection 0.1 m (1 m with shift); a burst of key presses is one save.

**Aim** writes rotations. A fixture with pan or tilt channels is a mover; everything else is
fixed. For fixed fixtures, **Face a direction** takes stage left, stage right, upstage,
downstage or a bearing in degrees, and a **tilt up from the floor** (0 is level, negative aims
down), and writes the rotation that does it. **At a focus point** picks one of the venue's focus
points and writes, for each selected fixture, the rotation that points its beam at it (a
fixture standing on the point is skipped, and the inspector says so). For movers, **Hung,
facing downstage** and **Standing on the deck** write the two mountings the
[pose convention](configuration.md#mounting-and-pose-convention) names; aiming a
mover is the show's job. The raw rotation is always shown and editable. The plot draws each
fixed fixture's beam from its rotation, so an aim is checked at once.

**Beam angle** (one or more fixtures, movers and fixed alike) is the angle in degrees the 3D view
draws the fixture's beam at. Set it when a diffuser or filter makes a unit wider than its fixture
type says; it only changes the drawing and never reaches the lights. Several fixtures with
different angles show as mixed. Clearing the field and pressing **Set beam angle** removes the
override, and the fixture goes back to its type's angle. The angle must be more than 0 and at
most 180.

Every operation is one save of the venue file, and keeps the focus points, provenance and every
field the operation did not change, a fixture's mode included. When a save leaves the current
venue unable to load, the save still happens and the message where you saved says so, naming the
fixture and why.

### Plot and 3D

The stage card has a **Plot | 3D** switch. **3D** swaps the picture in the card for the venue as a
room — the deck, every fixture from its GDTF's model at its position and rotation, beams, focus
points, and an MVR venue's scenery — and **Plot** swaps it back. Nothing else moves: the venue, the
selected fixtures, the inspector beside the picture, the card's header and **+ Focus point** stay.
The switch is part of the address (`?view=3d`), so Back, a reload and a shared link keep it, and
selecting another venue in the list keeps the card in 3D. three.js and the 3D code load the first
time 3D is pressed, not before. Drag to orbit, scroll to zoom, right-drag to pan; moving fixtures
by dragging stays a plot action.

The card's **maximize** button, beside the switch, fills the browser window with the whole card
under the navigation — the picture, the inspector and the focus points — for a bigger plot to
place fixtures on or a bigger room to watch. Press it again, or Escape, to put the card back
(Escape clears a selection first). Plot or 3D, the selection and the 3D camera stay as they were;
a reload shows the page as laid out. A playback bar shows along the bottom while a card is
maximized.

![The stage card in 3D: the current venue as a room, the pars' beams lighting the deck, the
bricks' beams in their colours, with Live | Preview under the picture and the inspector
beside it](../images/stage-3d.png)

The [dashboard](../interfaces/web-ui.md#dashboard)'s stage card has the same switch for the current venue. There it is a live view
only: nothing to select, no show preview, and the switch is not part of the address.

The selection is the plot's: click a fixture in 3D to select it (shift-click adds or removes one,
a click on empty space clears; a drag orbits and selects nothing), and the inspector shows it. A
selected fixture has a cyan box around it and a larger, cyan label. Whatever the inspector applies
— a rotation from **Aim**, a position, a mode — is redrawn in 3D where you are, without moving the
camera; only choosing another venue reframes it. That is the way to check an aim: a fixture
tipped up 30° looks the same from above as one hung straight down.

- **The current venue** is drawn live: the engine's colour, levels and pan and tilt, and a pixel
  fixture's cells each in its own colour. Under the picture, **Live | Preview** switches to
  [previewing a song](#previewing-a-song).
- **Any other venue** is drawn from its file, labelled *not live* as the plot is: every fixture
  at rest with its own type and mode's model, lit a plain white so the lenses and beam directions
  read. Under the picture, one line says a show can only be previewed on the current venue, with a
  link to the Groups page to make this one current. Saving the venue or a fixture type redraws it.
- **Fixtures with no position yet** are drawn in a row in front of the stage, and a note in the
  picture says how many, with a link back to the plot to place them.
- A line under the picture names fixtures drawn generically (a type with no GDTF model) and the
  scenery drawn and not drawn (`.3ds` meshes are skipped).

Sizes are true: the world is in meters (the grid is 1 m, noted in the picture's corner), fixtures
sit at their venue positions, and a fixture's model is drawn at the size its GDTF declares — the
GDTF standard has the mesh "explicitly scaled to this dimension", whatever size the mesh file has.
Two things are drawing conventions, not measurements: a beam's **angle** is the GDTF's beam angle
(or the fixture's own beam angle, when the venue sets one),
but its **length** is capped by beam type (a spot throws up to 24 m, a wash 8 m, an LED tile or
glow a short 0.9 m haze at the lens); and the **deck** is just big enough to hold the fixtures and
focus points (at least 8 × 6 m, with 1 m to spare, wider when the MVR's scenery is) — it is not a
stored stage size. A browser that cannot draw 3D (no WebGL) says so in the card, and **Plot**
still works.

### Previewing a song

On the current venue's 3D view, **Preview** draws what a song's show would do at a moment you
choose, without playing anything: pick a song (any song with lighting that loads) and drag the
scrubber, or step it with the arrow keys; the song's sections are drawn along it as the
dashboard's timeline draws them. The show is worked out offline, so a song that is playing keeps
playing and nothing reaches the lights. **Live** goes back to what the engine is sending.

Beneath the scrubber, **At this moment** says in words what each group is doing (for example
"movers: move, 50% done (2.0 s of 4.0 s)"), **Untouched by this show** counts the fixtures no
cue targets and names them when expanded, and **Open this cue in the timeline** opens the
song's lighting editor with its cursor at the scrubbed time (`#/songs/<name>/lighting?t=<seconds>`).
Small notes over the picture appear only when they apply: fixtures whose colour comes from a
wheel are drawn white and, in Live, that nothing has been received from the engine yet. The moment is kept in the address —
`#/lighting/venues/<venue>?view=3d&mode=preview&song=<name>&t=<seconds>` — so a reload or a
shared link opens at it.

An address of the form `#/stage` or `#/lighting/stage` (with or without
`?mode=preview&song=…&t=…`) opens the Venues page with the card in 3D, on the current venue when
the page knows it, keeping a preview's song and time.

## Import an MVR

**Import an MVR** on the Venues page (and a link on the Overview) opens a four-step wizard at
`#/lighting/import`, over the same library as `mtrack import-mvr` and the MCP tools. The
browser keeps the file and uploads it at each step; the server holds nothing in between, and
nothing is written until the last step.

1. **File.** Drop an `.mvr` or choose one. The page shows what it holds: fixtures, fixture
   types that have a GDTF mode and fixtures that do not, universes, focus points and scenery.
   The venue name defaults to the file's name; an existing venue of that name that came from
   this MVR is merged into. A file that is not an MVR, or is over the archive caps, is refused
   here with the reason.
2. **Stage origin.** A top-down plan of the file's fixtures, focus points and scenery, in the
   file's own millimeters (up the page is upstage). Click the front edge of the deck, in the
   middle: that point becomes the stage origin. A ring marks a **suggestion**, and **Use the
   suggestion** applies it: the middle of the deck's front edge, at the deck's top, when the
   scenery carries a stage floor; otherwise the middle of the fixtures' front edge at the floor.
   The X, Y and Z fields hold the chosen point and can be typed in; on the focused plan the arrow
   keys move it 100 mm (1 m with shift).

   mtrack finds the deck by name: a plain scene object (not a truss, support or screen) whose
   name or mesh file says *stage*, *deck*, *floor*, *podium*, *riser*, *platform* or the like,
   sized from the bounds of its glTF (`.glb`) meshes. A file whose stage floor is named
   otherwise, or drawn only as `.3ds`, has no deck to suggest and falls back to the fixtures.
3. **Review.** What the import will do, without writing it: whether this seeds a new venue or
   merges into an existing one (a merge keeps tags, focus names and fixtures you added by hand),
   fixtures to seed (each with its own mode), the fixtures' GDTFs to add and which are already in
   the library, fixtures that become `# TODO` lines with the reason for each, and how much scenery
   the 3D view can draw. On a merge it also lists the fields you edited by hand that the MVR would
   overwrite, each with a **Keep my edits** checkbox: positions, rotations and focus points start
   checked, patch, mode and type start unchecked.
4. **Import.** Writes the files and reports them. Two buttons follow: **Fit your shows**, since a
   seeded venue is untagged, and **Open in Venues**. Importing does not make the venue current;
   pick it on the Groups page.

![The MVR import wizard at Review: a first import seeding a venue, the fixture type to import and
the mode of each fixture, a fixture that becomes a TODO line, and a scenery
warning](../images/lighting-mvr-review.png)

## Export an MVR

**Export an MVR** on a venue's card opens a dialog. It takes the file name (default
`<venue>.mvr`, made file-name-safe), **One layer per first tag**, and shows a summary computed
without writing anything: fixtures and how many have positions, GDTF files embedded from the
library and generated for native types, focus points, and how many fixed fixtures are linked to
a focus point.

Pre-viz tools aim a fixed fixture (one with no pan or tilt) at its linked focus point and,
without one, at the origin. When some are unlinked the dialog says so and offers **Add an aim
point per fixture**: for each one, a focus point named `<fixture> aim` is added to the venue
file where its rest beam meets the deck, or 3 m along the beam when it does not meet the deck
within 50 m. Fixtures with no position are skipped, and the rest of the file, including
positions, rotations and comments, is left as it was. The summary is read again and the export
links them. **Export as is** downloads without adding any. **Download** sends the `.mvr` to the
browser; nothing is written under `lighting/export/` unless **Keep a copy in the project** is
ticked.

## Groups

Directories, the current venue and logical groups with their constraints. Pick
the profile at the top. **Save** writes that profile; leaving the page with unsaved edits asks
first.

![The Groups page: the running profile, the directories, the current venue and the logical groups,
one open with its constraints](../images/lighting-groups.png)

The **current venue** is the one the engine lights; shows play on it, and the live stage views
draw it. Choosing another one here is how you move the rig from one place to another.

A profile that still carries inline fixtures (`dmx.lighting.fixtures`) is refused at load;
fixtures are patched in venue files, and `mtrack migrate --apply` moves inline ones into one (see
the [command-line reference](../reference/cli.md#mtrack-migrate)).

## Fit shows

An imported venue arrives with no tags, so every group a show uses finds no fixtures; a console's
focus points carry the console's names; and its universes have no output on this profile. The
Overview says so, and its findings for groups, unbound focus points and outputs link here. Fit
shows fixes them in one place, in three columns (stacked on a phone):

![Fit shows with the movers group selected: the groups your shows use, the plan with the three
suggested fixtures ringed, and the suggestion with Apply, a focus point to place and the
output fixes](../images/lighting-fit.png)

- **Groups your shows use** lists every group any song's shows target, with the tags its
  constraints need, how many fixtures it finds in the current venue and which songs use it.
  Groups that find nothing come first. Select one to drive the other two columns.
- **The plan** is the stage plot. A solid ring marks the fixtures the selected group finds; a
  dashed ring marks the pending selection. Click a fixture to add it to the selection or take it
  out.
- **Fixes** starts with a **suggestion** for the selected group: the fixtures that can do what
  the shows ask of it (a `move` needs pan or tilt, a colour cue colour channels, a `strobe` a
  strobe channel, `per: cell` cells), grouped by fixture type and, when the venue places them,
  by where they hang (deck, low rig or truss; downstage, midstage or upstage). The largest set is
  the suggestion, with the reason in words; if nothing fits it says which need nothing meets.
  **Apply** adds the group's tags to those fixtures in the venue file, keeping positions,
  rotations, focus points and the MVR provenance, and the engine reloads the venue. **Pick
  others** offers the other sets, or lets you choose fixtures by hand on the plan or from a list;
  **Tag N fixtures** then applies the group's tags to your selection. Nothing is tagged without a
  click.

  Below that, **focus points your shows aim at** that the venue lacks each have **Place on
  plan**: the next click on the plan creates a point with that name (pressing Enter on the
  focused plan puts it at the center). **Output** lists universes the venue uses that the running
  profile has no output for, with **Add to profile**, which appends `{universe, name: "u<N>"}`
  to the profile's DMX universes and saves it as Config does; and universes olad has no port
  patched to, with the `ola_patch` line to run (fill in your device and port).

A footer repeats the Overview's Groups check ("2 of 6 groups find fixtures"). The same
suggestion is available to agents as the MCP tool `suggest_group_tags`. A link such as
`#/lighting/fit?group=movers` opens the page with that group selected.

