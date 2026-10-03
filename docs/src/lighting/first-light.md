# First Light: From a GDTF to a Show

This page takes one light from nothing to a cue in a song, all in the web UI: import the
fixture's GDTF, put it in a venue in the mode the unit is set to, make that venue current, check
that the light answers, and write a show for it. There is no translation step: the GDTF is the
fixture, and nothing about its channels is ever written by hand.

You need mtrack running with the web UI open, a DMX interface that olad can see, and the
fixture's GDTF file. Most manufacturers publish one; [GDTF Share](https://gdtf-share.com/) has
thousands. Unlock the player first (the lock icon in the top nav), since every step here edits
the project.

## 1. Give the profile a DMX output

On **Config**, open the profile for this machine and turn on its **Lighting** section. Add a
universe: the OLA universe number your interface is patched to, and a name. This is the part of
the setup that belongs to the machine rather than the project; see
[Hardware Configuration](../getting-started/hardware-config.md).

olad sends nothing for a universe that has no output port patched to it, and says nothing about
it either. Open olad's own page (`http://<the machine>:9090`) and patch your interface's output
port to that universe. mtrack checks this for you and warns when a universe it drives has no port
patched ([The universe must exist in olad](configuration.md#the-universe-must-exist-in-olad)).

## 2. Import the GDTF

Go to **Lighting → Fixture types** and press **Import GDTF**. Choose the `.gdtf` file. That is
the whole import: the file is copied into `lighting/library/`, the fixture is listed under the
name inside the file, and its page opens. The page shows the fixture in 3D and every one of its
DMX modes, each with its channels and what a show can do in it.

![The Fixture types page: a GDTF fixture's card with its model, maker, mode count and the modes
in use, beside two hand-written types](../images/lighting-fixture-types.png)

![A GDTF fixture's page: the 3D model, the facts from the GDTF, the mode list with 8: RGBS
selected and its four channels, and the fixture's settings with the strobe
curve](../images/lighting-fixture-page.png)

From the command line, `mtrack import-gdtf <file>` does the same, and
`mtrack import-gdtf <file> --list-modes` lists the modes without importing anything.

## 3. Find the unit's mode and address

A fixture's mode (its personality: "8: RGBS", "13: DIM RGBAWS" …) and its DMX address are set on
the unit itself, usually in its menu. mtrack cannot read them from the light, so look. The
fixture page lists the same modes by the names the manufacturer uses.

## 4. Test the light

Still on the fixture's page, open **Test this fixture**. Choose the mode the unit is set to, the
universe its cable is on and its DMX address, then turn on **Send to lights**. A working light
comes up full white at once; the swatches and sliders let you check its colours, strobe and, on
a mover, pan and tilt.

If nothing happens, the panel lists what to check, in order: whether olad is answering, whether
the universe has an output patched, then the unit's own address, mode and cable. It also shows
exactly what mtrack is sending, so "mtrack is not sending" can be told from "the light is not
listening". See [Testing a fixture](web-ui.md#testing-a-fixture).

![Test this fixture, live: mode 8: RGBS at address 5, Send to lights on, the red swatch chosen,
the line saying mtrack is sending 255, 0, 0, 0, and the "Nothing happened?"
checks](../images/lighting-fixture-test.png)

Turn **Send to lights** off when you are done, or just leave the page: the test lets go by
itself.

## 5. Put it in a venue

A venue is the rig at one place: which fixtures there are, in which modes, at which addresses.
On **Lighting → Venues**, press **New Venue** and give it a name. **Add Fixture** adds a row;
choose your fixture as its type, and the **Mode** select lists the GDTF's modes with how many
addresses each uses. Pick the mode the unit is set to, the universe, and the address you just
tested. Give the fixture a **tag** that says what it is for, such as `wash` or `front`: shows
reach fixtures by tags, never by name. Save.

**Add Fixture** again continues the patch: the next row starts straight after the last one's
addresses. A row that would run into another's addresses is marked before you save.

![The venue editor: three fixtures, two PixelBricks in different modes with the addresses each
uses, and a warning where the second runs into the third](../images/lighting-venue-editor.png)

## 6. Make the venue current and give the show a group

On **Lighting → Groups**, choose the profile this machine runs (it is chosen for you when the
profile's hostname matches). Set **Current Venue** to your venue: the current venue is the one
the engine lights. Then add a logical group, say `front_wash`, with an **AllOf** constraint on
the tag you gave the fixture. Save.

![The Groups page: the running profile, its current venue "house", and the logical groups, one
open to show its constraints](../images/lighting-groups.png)

The **Overview** tab now answers "will your show reach the lights?": the fixture type, venue
and output checks should read **Ready**, and anything that is not says what to fix and links
there ([Overview](web-ui.md#overview-will-your-show-reach-the-lights)).

![The Overview: five numbered checks, then what needs attention — an overlap in the patch, a
group that finds no fixtures, and a universe olad has no port patched to](../images/lighting-overview.png)

## 7. Write a show

Open a song, go to its **Lighting** tab and press **+ DSL** to give it a light show file. In the
timeline, add a cue on the group, for example a static red, and play the song: the fixture
follows. The [Timeline Editor](../interfaces/web-ui.md#timeline-editor) covers the editing, and
the [Effects Reference](effects.md) every effect. The same cue, written as text:

```light
show "Main" {
    @00:00.000
    front_wash: static color: "red", dimmer: 100%, duration: 8s
}
```

The show names the group, not the fixture, so the same song plays at another venue whose
fixtures carry the same tags.

## What the files look like

None of this needs editing by hand, but it is all plain text in the project:

```text
lighting/library/robin_esprite.gdtf       the GDTF, as imported
lighting/venues/house.venue               the venue
songs/My Song/main.light                  the show
mtrack.yaml                               the profile: DMX universes, groups, current venue
```

```light
venue "house" {
  fixture "Front 1" "Robin Esprite" mode "Mode 1" @ 1:1 tags ["wash", "front"]
}
```

A GDTF fixture gets a small record in `lighting/fixture_types/` only once you set something the
GDTF does not say: its name in your venues, a mover's speed limits, or its strobe curve (see
[GDTF fixture types](configuration.md#gdtf-fixture-types)).

## Where next

- A rig in a console or pre-viz tool comes in whole as an MVR: [Import an
  MVR](web-ui.md#import-an-mvr), then [Fit shows](web-ui.md#fit-shows)
  to tag it for your groups.
- Place fixtures on the stage plot, aim them and see the rig in 3D:
  [Venues](web-ui.md#venues) and [Plot and 3D](web-ui.md#plot-and-3d).
- A fixture with no GDTF is written by hand: [Fixture Type
  Definitions](configuration.md#fixture-type-definitions-lightingfixture_types).
