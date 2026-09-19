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

//! Golden checks from files to bytes.
//!
//! Everything else in the lighting tests verifies one stage against the
//! next: the pointing math against its own round trip, the resolver
//! against hand-built channel defs, the engine against the pointing math.
//! A consistent mistake shared by two stages passes all of them. These
//! tests start from what a user writes — a GDTF archive, `.fixture` and
//! `.venue` files, a `.light` show — and end at the DMX frame, compared
//! with numbers worked out by hand from the documented conventions and
//! written down here, not computed by the code under test.
//!
//! The expected values were derived independently (a few lines of
//! trigonometry, the rotation order and pan/tilt signs taken from the
//! `pointing` module's documentation and the stage coordinate
//! definition in `FixtureInfo`). The working is shown beside each one so
//! a reader can redo it on paper.

use std::collections::HashMap;
use std::time::Duration;

use crate::config::lighting::{Directories, GroupConstraint, Lighting, LogicalGroup};
use crate::lighting::effects::DmxCommand;
use crate::lighting::evaluate::{apply_timeline_update, evaluate_show, Evaluation};
use crate::lighting::gdtf::{build_zip, SYNTHETIC_DESCRIPTION};
use crate::lighting::parser::parse_light_shows;
use crate::lighting::system::LightingSystem;
use crate::lighting::timeline::LightingTimeline;
use crate::lighting::EffectEngine;

/// A three-cell RGB bar with one dimmer on the body: the pixel-mode shape
/// the distiller expands from template channels (design §17.2). Cells
/// P1..P3 sit 0.1 m apart along the bar's x axis; their Breaks place
/// them at DMX 2-4, 5-7 and 8-10 after the dimmer at 1.
const PIXEL_GDTF: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<GDTF DataVersion="1.2"><FixtureType Name="Golden Bar" Manufacturer="mtrack golden">
  <Geometries>
    <Geometry Name="Base" Position="{1,0,0,0}{0,1,0,0}{0,0,1,0}{0,0,0,1}">
      <Geometry Name="Body"/>
      <GeometryReference Name="P1" Geometry="Lens" Position="{1,0,0,-0.1}{0,1,0,0}{0,0,1,0}{0,0,0,1}"><Break DMXBreak="1" DMXOffset="1"/></GeometryReference>
      <GeometryReference Name="P2" Geometry="Lens" Position="{1,0,0,0}{0,1,0,0}{0,0,1,0}{0,0,0,1}"><Break DMXBreak="1" DMXOffset="4"/></GeometryReference>
      <GeometryReference Name="P3" Geometry="Lens" Position="{1,0,0,0.1}{0,1,0,0}{0,0,1,0}{0,0,0,1}"><Break DMXBreak="1" DMXOffset="7"/></GeometryReference>
    </Geometry>
    <Geometry Name="Lens"/>
  </Geometries>
  <DMXModes><DMXMode Name="Pixel" Geometry="Base"><DMXChannels>
    <DMXChannel Offset="1" Geometry="Body"><LogicalChannel Attribute="Dimmer"><ChannelFunction Name="D" Attribute="Dimmer" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
    <DMXChannel Offset="2" Geometry="Lens"><LogicalChannel Attribute="ColorAdd_R"><ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
    <DMXChannel Offset="3" Geometry="Lens"><LogicalChannel Attribute="ColorAdd_G"><ChannelFunction Name="G" Attribute="ColorAdd_G" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
    <DMXChannel Offset="4" Geometry="Lens"><LogicalChannel Attribute="ColorAdd_B"><ChannelFunction Name="B" Attribute="ColorAdd_B" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
  </DMXChannels></DMXMode></DMXModes>
</FixtureType></GDTF>"#;

/// The GDTF mover: pan 1/2 over -270..270°, tilt 3/4 over -135..135°
/// (from the synthetic description's "Mover 16bit" mode).
const MOVER_FIXTURE: &str = "fixture_type \"Mover\"\n  from gdtf(\"lighting/library/mover.gdtf\", mode \"Mover 16bit\")\n{ }\n";

/// A hand-written mover with the same travel and a slew limit, so both
/// sources of a degree range are on the wire.
const CHEAP_FIXTURE: &str = "fixture_type \"Cheap\" {\n  \
  channel \"pan\"    @ 1 fine 2 range -270deg..270deg\n  \
  channel \"tilt\"   @ 3 fine 4 range -135deg..135deg\n  \
  channel \"dimmer\" @ 5\n  \
  movement { max_pan_speed: 240deg/s }\n}\n";

const BAR_FIXTURE: &str =
    "fixture_type \"Bar\"\n  from gdtf(\"lighting/library/pixel.gdtf\", mode \"Pixel\")\n{ }\n";

/// The same bar in the hand-written cell form, cells 0.3 m apart.
const HAND_BAR_FIXTURE: &str = "fixture_type \"HandBar\" {\n  channel \"dimmer\" @ 1\n  \
  cell \"1\" at (-0.3, 0, 0) { channel \"red\" @ 2  channel \"green\" @ 3  channel \"blue\" @ 4 }\n  \
  cell \"2\" at (0, 0, 0) { channel \"red\" @ 5  channel \"green\" @ 6  channel \"blue\" @ 7 }\n  \
  cell \"3\" at (0.3, 0, 0) { channel \"red\" @ 8  channel \"green\" @ 9  channel \"blue\" @ 10 }\n}\n";

/// Stage coordinates: meters, right-handed Z-up, origin downstage-centre
/// on the deck, +x stage-left, +y upstage. Rotation is degrees about X, Y,
/// Z applied in that order. M1/M2 hang on the rear truss turned to face
/// downstage; M3 is on the front pipe pitched 30° about X; M4 is off
/// stage-left with a yaw and a pitch, so the rotation order matters.
/// Bar2 is hung backwards, so its cell "1" ends up furthest stage-left.
const VENUE: &str = "venue \"golden\" {\n  \
  fixture \"M1\" Mover @ 1:1 tags [\"movers\", \"rear\"] position (-2, 3.5, 4.2) rotation (0, 0, 180)\n  \
  fixture \"M2\" Mover @ 1:11 tags [\"movers\", \"rear\"] position (2, 3.5, 4.2) rotation (0, 0, 180)\n  \
  fixture \"M3\" Cheap @ 1:21 tags [\"movers\", \"front\"] position (0, 0.5, 3.0) rotation (30, 0, 0)\n  \
  fixture \"M4\" Cheap @ 1:31 tags [\"movers\", \"front\"] position (3, 2, 4) rotation (0, 20, 90)\n  \
  fixture \"Bar1\" Bar @ 1:101 tags [\"bars\"] position (-1, 1, 0.5)\n  \
  fixture \"Bar2\" HandBar @ 1:121 tags [\"bars\"] position (1, 1, 0.5) rotation (0, 0, 180)\n  \
  focus \"drummer\" (0, 2.8, 1.4)\n  \
  focus \"singer\" (0.4, 0.9, 1.6)\n  \
  focus \"wing\" (-3, 1.5, 1.0)\n}\n";

const SHOW: &str = "show \"golden\" {\n    \
    @00:00.000\n    bars: static color: \"white\", dimmer: 100%, duration: 12s\n    \
    @00:00.000\n    bars: rainbow speed: 0.0, spread: 360deg, per: cell, duration: 12s, layer: midground\n    \
    @00:00.000\n    movers: move focus: \"drummer\", duration: 100ms\n    \
    @00:02.000\n    movers: move focus: \"singer\", duration: 500ms\n    \
    @00:05.000\n    front: move focus: \"wing\", duration: 500ms\n    \
    @00:05.000\n    rear: move pan: 45deg, tilt: -20deg, duration: 500ms\n}\n";

/// The instants the frame is judged at: each one well after the cue
/// before it has finished travelling (M3's 240°/s limit needs 0.6 s for
/// its longest leg).
const AT: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(4),
    Duration::from_secs(7),
];

/// One expected pan/tilt on the wire: fixture, 16-bit pan, 16-bit tilt.
/// Bytes over a 16-bit range: `round((deg − from) / (to − from) × 65535)`.
struct Aimed(&'static str, u16, u16);

/// Hand-worked expectations, one list per instant.
///
/// Aiming: `d = target − position`, rotated into the mounting frame by
/// `(Rz·Ry·Rx)ᵀ`; `pan = atan2(dx, dy)`, `tilt = atan2(dz, hypot(dx, dy))`.
/// The turn is the one nearest the previous pan within ±270°.
///
/// t = 1 s, every mover on the drummer (0, 2.8, 1.4):
/// - M1 at (−2, 3.5, 4.2) yawed 180°: d = (2, −0.7, −2.8); yaw flips x
///   and y → (−2, 0.7, −2.8); pan = atan2(−2, 0.7) = −70.71°,
///   tilt = atan2(−2.8, 2.119) = −52.88°. Pan → 24186, tilt → 19932.
/// - M2 mirrors M1 in x: pan +70.71° → 41349, tilt the same.
/// - M3 at (0, 0.5, 3) pitched 30° about X: d = (0, 2.3, −1.6);
///   Rxᵀ(30°) gives y' = 2.3cos30 − 1.6sin30 = 1.192,
///   z' = −2.3sin30 − 1.6cos30 = −2.536; pan 0 → 32768,
///   tilt = atan2(−2.536, 1.192) = −64.82° → 17033.
/// - M4 at (3, 2, 4), Ry 20° then Rz 90°: d = (−3, 0.8, −2.6);
///   undo the yaw first (Rzᵀ 90°: (x,y) → (y,−x)) → (0.8, 3, −2.6), then
///   Ryᵀ 20°: x' = 0.8cos20 − (−2.6)sin20 = 1.641,
///   z' = 0.8sin20 + (−2.6)cos20 = −2.170; pan = atan2(1.641, 3) = 28.68°
///   → 36248, tilt = atan2(−2.170, 3.420) = −32.39° → 24905.
///
/// t = 4 s, every mover on the singer (0.4, 0.9, 1.6):
/// - M1: pan −42.71° → 27584, tilt −36.31° → 23955.
/// - M2: pan 31.61° → 36603, tilt −40.42° → 22957.
/// - M3: d = (0.4, 0.4, −1.4); Rxᵀ(30°): y' = 0.4cos30 + (−1.4)sin30
///   = −0.354, z' = −0.4sin30 + (−1.4)cos30 = −1.412. The singer is
///   behind the pitched frame's +y, so pan = atan2(0.4, −0.354) = 131.48°
///   → 48724; tilt = atan2(−1.412, 0.534) = −69.29° → 15948.
/// - M4: pan −4.68° → 32200, tilt −45.25° → 21785.
///
/// t = 7 s: the front pair on the wing (−3, 1.5, 1.0), the rear pair at
/// explicit angles.
/// - M3: principal pan −92.56°; the pan before was 131.48°, so the
///   nearest turn inside ±270° is 267.44° (136° away, not 224° back the
///   other way) → 65225; tilt −36.62° → 23878.
/// - M4: pan 5.30° → 33410, tilt −26.39° → 26362.
/// - M1, M2: pan 45° → round(315/540 × 65535) = 38229,
///   tilt −20° → round(115/270 × 65535) = 27913.
fn expected_pointing() -> [Vec<Aimed>; 3] {
    [
        vec![
            Aimed("M1", 24186, 19932),
            Aimed("M2", 41349, 19932),
            Aimed("M3", 32768, 17033),
            Aimed("M4", 36248, 24905),
        ],
        vec![
            Aimed("M1", 27584, 23955),
            Aimed("M2", 36603, 22957),
            Aimed("M3", 48724, 15948),
            Aimed("M4", 32200, 21785),
        ],
        vec![
            Aimed("M1", 38229, 27913),
            Aimed("M2", 38229, 27913),
            Aimed("M3", 65225, 23878),
            Aimed("M4", 33410, 26362),
        ],
    ]
}

/// The bars, at every instant: one rainbow spread over 360° across the
/// group's six cells with `per: cell`, hue 0 held (speed 0). Cells are
/// ordered stage-right to stage-left (ascending x): Bar1's P1, P2, P3 at
/// x = −1.1, −1.0, −0.9, then Bar2 hung backwards so its cell 3 lands at
/// x = 0.7, cell 2 at 1.0, cell 1 at 1.3. Six cells share the circle 60°
/// apart: 0° red, 60° yellow, 120° green, 180° cyan, 240° blue, 300°
/// magenta. The dimmer bed is full.
///
/// `(DMX channel, value)`: Bar1 at 101 (dimmer 101, cells 102-104,
/// 105-107, 108-110); Bar2 at 121 (dimmer 121, cell 1 at 122-124, cell 2
/// at 125-127, cell 3 at 128-130).
const EXPECTED_CELLS: [(u16, u8); 20] = [
    (101, 255),
    (102, 255),
    (103, 0),
    (104, 0), // P1 red
    (105, 255),
    (106, 255),
    (107, 0), // P2 yellow
    (108, 0),
    (109, 255),
    (110, 0), // P3 green
    (121, 255),
    (128, 0),
    (129, 255),
    (130, 255), // cell 3 cyan
    (125, 0),
    (126, 0),
    (127, 255), // cell 2 blue
    (122, 255),
    (123, 0),
    (124, 255), // cell 1 magenta
];

/// The same expectation by cell name, for the evaluator's view.
const EXPECTED_CELL_COLOURS: [(&str, &str, [u8; 3]); 6] = [
    ("Bar1", "P1", [255, 0, 0]),
    ("Bar1", "P2", [255, 255, 0]),
    ("Bar1", "P3", [0, 255, 0]),
    ("Bar2", "3", [0, 255, 255]),
    ("Bar2", "2", [0, 0, 255]),
    ("Bar2", "1", [255, 0, 255]),
];

struct Project {
    _dir: tempfile::TempDir,
    system: LightingSystem,
}

fn project() -> Project {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    for sub in ["lighting/library", "fixture_types", "venues"] {
        std::fs::create_dir_all(base.join(sub)).unwrap();
    }
    std::fs::write(
        base.join("lighting/library/mover.gdtf"),
        build_zip(&[("description.xml", SYNTHETIC_DESCRIPTION.as_bytes())]),
    )
    .unwrap();
    std::fs::write(
        base.join("lighting/library/pixel.gdtf"),
        build_zip(&[("description.xml", PIXEL_GDTF.as_bytes())]),
    )
    .unwrap();
    std::fs::write(base.join("fixture_types/mover.fixture"), MOVER_FIXTURE).unwrap();
    std::fs::write(base.join("fixture_types/cheap.fixture"), CHEAP_FIXTURE).unwrap();
    std::fs::write(base.join("fixture_types/bar.fixture"), BAR_FIXTURE).unwrap();
    std::fs::write(
        base.join("fixture_types/hand_bar.fixture"),
        HAND_BAR_FIXTURE,
    )
    .unwrap();
    std::fs::write(base.join("venues/golden.venue"), VENUE).unwrap();

    // The show's group names are the venue's tags, one group per tag —
    // the mtrack.yaml wiring a project would carry.
    let groups: HashMap<String, LogicalGroup> = ["movers", "rear", "front", "bars"]
        .into_iter()
        .map(|tag| {
            (
                tag.to_string(),
                LogicalGroup::new(
                    tag.to_string(),
                    vec![GroupConstraint::AllOf(vec![tag.to_string()])],
                ),
            )
        })
        .collect();
    let config = Lighting::new(
        Some("golden".to_string()),
        None,
        Some(groups),
        Some(Directories::new(
            Some("fixture_types".to_string()),
            Some("venues".to_string()),
        )),
    );
    let mut system = LightingSystem::new();
    system.load(&config, base).unwrap();
    Project { _dir: dir, system }
}

fn focus_points(system: &LightingSystem) -> HashMap<String, [f64; 3]> {
    system
        .get_current_venue()
        .unwrap()
        .focus_points()
        .iter()
        .map(|(n, p)| (n.clone(), *p))
        .collect()
}

/// The 512-byte universe after the engine has run the show live, frame
/// by frame at 10 ms, sampled at each instant in `AT`.
fn live_frames(project: &mut Project) -> Vec<[u8; 513]> {
    let fixtures = project.system.get_current_venue_fixtures().unwrap();
    let mut engine = EffectEngine::new();
    engine.set_focus_points(focus_points(&project.system));
    for fixture in fixtures {
        engine.register_fixture(fixture);
    }
    let shows = parse_light_shows(SHOW).unwrap().into_values().collect();
    let mut timeline = LightingTimeline::new(shows);
    let mut resolve = |mut effect: crate::lighting::effects::EffectInstance| {
        effect.target_fixtures = effect
            .target_fixtures
            .iter()
            .flat_map(|g| project.system.resolve_logical_group_graceful(g))
            .collect();
        effect
    };
    apply_timeline_update(&mut engine, timeline.start_at(Duration::ZERO), &mut resolve);

    let step = Duration::from_millis(10);
    let mut universe = [0u8; 513];
    let mut frames = Vec::new();
    let mut now = Duration::ZERO;
    let end = *AT.last().unwrap() + Duration::from_millis(100);
    while now <= end {
        apply_timeline_update(&mut engine, timeline.update(now), &mut resolve);
        let commands: Vec<DmxCommand> = engine.update(step, Some(now)).unwrap().to_vec();
        for command in commands {
            assert_eq!(command.universe, 1);
            universe[command.channel as usize] = command.value;
        }
        if AT.contains(&now) {
            frames.push(universe);
        }
        now += step;
    }
    assert_eq!(frames.len(), AT.len());
    frames
}

fn offline(project: &mut Project) -> Vec<Evaluation> {
    let fixtures = project.system.get_current_venue_fixtures().unwrap();
    let focus = focus_points(&project.system);
    let shows = parse_light_shows(SHOW).unwrap().into_values().collect();
    evaluate_show(shows, &fixtures, &focus, None, &AT, |mut effect| {
        effect.target_fixtures = effect
            .target_fixtures
            .iter()
            .flat_map(|g| project.system.resolve_logical_group_graceful(g))
            .collect();
        effect
    })
}

fn address(name: &str) -> u16 {
    match name {
        "M1" => 1,
        "M2" => 11,
        "M3" => 21,
        "M4" => 31,
        _ => unreachable!(),
    }
}

fn near(got: u16, want: u16) -> bool {
    got.abs_diff(want) <= 1
}

/// Every mover's pan and tilt on the wire match the hand-worked bytes at
/// each instant: focus points through two venue rotations that do not
/// commute, a range from a GDTF file and one from a `.fixture`, the
/// nearest turn after a previous pose, and an explicit-angle move.
#[test]
fn movers_aim_where_the_hand_calculation_says_on_the_wire() {
    let mut project = project();
    let frames = live_frames(&mut project);
    for (i, expected) in expected_pointing().iter().enumerate() {
        for Aimed(name, pan, tilt) in expected {
            let base = address(name) as usize;
            let got_pan = u16::from_be_bytes([frames[i][base], frames[i][base + 1]]);
            let got_tilt = u16::from_be_bytes([frames[i][base + 2], frames[i][base + 3]]);
            assert!(
                near(got_pan, *pan) && near(got_tilt, *tilt),
                "{name} at {:?}: wire pan/tilt {got_pan}/{got_tilt}, hand-worked {pan}/{tilt}",
                AT[i]
            );
        }
    }
}

/// The offline evaluator — what the `evaluate_show` MCP tool reports —
/// reaches the same bytes from the same files, so a user can check their
/// aim without a rig.
#[test]
fn the_evaluator_reports_the_same_pointing_as_the_wire() {
    let mut project = project();
    let evaluations = offline(&mut project);
    for (i, expected) in expected_pointing().iter().enumerate() {
        for Aimed(name, pan, tilt) in expected {
            let fixture = evaluations[i]
                .fixtures
                .iter()
                .find(|f| f.name == *name)
                .unwrap_or_else(|| panic!("{name} missing from the evaluation"));
            let byte = |channel: &str| {
                *fixture
                    .channels
                    .get(channel)
                    .unwrap_or_else(|| panic!("{name} at {:?} has no {channel}", AT[i]))
            };
            let got_pan = u16::from_be_bytes([byte("pan"), byte("pan_fine")]);
            let got_tilt = u16::from_be_bytes([byte("tilt"), byte("tilt_fine")]);
            assert!(
                near(got_pan, *pan) && near(got_tilt, *tilt),
                "{name} at {:?}: evaluated pan/tilt {got_pan}/{got_tilt}, hand-worked {pan}/{tilt}",
                AT[i]
            );
        }
    }
}

/// A `per: cell` rainbow spread across two bars lands each cell's colour
/// at the cell's own DMX address: template cells from a GDTF file, hand
/// written cells from a `.fixture`, ordered on the stage through the
/// venue's rotation.
#[test]
fn cells_take_their_own_colour_at_their_own_address_on_the_wire() {
    let mut project = project();
    let frames = live_frames(&mut project);
    for (i, frame) in frames.iter().enumerate() {
        for (channel, value) in EXPECTED_CELLS {
            assert_eq!(
                frame[channel as usize], value,
                "DMX {channel} at {:?}",
                AT[i]
            );
        }
    }
}

/// The evaluator reports the same per-cell colours under the cell names.
#[test]
fn the_evaluator_reports_the_same_cells_as_the_wire() {
    let mut project = project();
    let evaluations = offline(&mut project);
    for (i, evaluation) in evaluations.iter().enumerate() {
        for (fixture, cell, [r, g, b]) in EXPECTED_CELL_COLOURS {
            let snapshot = evaluation
                .fixtures
                .iter()
                .find(|f| f.name == fixture)
                .unwrap();
            let own = snapshot
                .cells
                .get(cell)
                .unwrap_or_else(|| panic!("{fixture}/{cell} missing at {:?}", AT[i]));
            let got = [own["red"], own["green"], own["blue"]];
            assert_eq!(got, [r, g, b], "{fixture}/{cell} at {:?}", AT[i]);
            assert_eq!(snapshot.channels["dimmer"], 255, "{fixture} dimmer");
        }
    }
}
