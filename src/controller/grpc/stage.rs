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

//! The stage as the gRPC stage stream says it: the websocket's `metadata`
//! and `state` messages as a layout and frames. The layout names every
//! fixture and its channels once; a frame is then bytes in the layout's
//! order, with only what differs from the layout (poses) or is running
//! (strobes, per-cell values) beside them. Pure conversion; the stream
//! itself is in the service.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::proto::player::v1::{
    StageCell, StageCellValues, StageFixture, StageFixtureCells, StageFrame, StageLayout,
    StagePoint, StagePose, StagePoseChange, StageStrobe, StageVec3, StageVenue,
};
use crate::state::{PoseSnapshot, StateSnapshot};
use crate::webui::wire::messages as wire;

/// Geometry is sent as `f32`: a millimeter on a stage needs no more, and it
/// halves the poses.
fn vec3(v: [f64; 3]) -> StageVec3 {
    StageVec3 {
        x: v[0] as f32,
        y: v[1] as f32,
        z: v[2] as f32,
    }
}

fn pose(pose: &PoseSnapshot) -> StagePose {
    StagePose {
        pan: pose.pan as f32,
        tilt: pose.tilt as f32,
        aim: Some(vec3(pose.aim)),
        floor: pose.floor.map(|[x, y]| StagePoint {
            x: x as f32,
            y: y as f32,
        }),
    }
}

/// A map's keys, sorted: the order a fixture's or a cell's bytes are in.
fn sorted_names(channels: &HashMap<String, u8>) -> Vec<String> {
    let mut names: Vec<String> = channels.keys().cloned().collect();
    names.sort_unstable();
    names
}

/// A capability as the JSON names it, from the same serialization.
fn capability_name(capability: &crate::lighting::fit::Capability) -> String {
    match serde_json::to_value(capability) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

/// The layout for a snapshot: the venue's fixtures as `metadata` has them,
/// and with them every fixture the snapshot names (one it carries, one with
/// a pose, one under test), sorted by name. A fixture only in the snapshot
/// has channels and no placement; one only in the metadata has no channels.
/// Each takes its channel names, sorted, and its pose from the snapshot.
/// The layout's `id` is the stream's to give: it is 0 here.
pub(crate) fn stage_layout(
    metadata: &wire::StageMetadata,
    snapshot: &StateSnapshot,
) -> StageLayout {
    // By name, the last of a name winning, as the JSON's maps have it.
    let channels: HashMap<&str, &HashMap<String, u8>> = snapshot
        .fixtures
        .iter()
        .map(|f| (f.name.as_str(), &f.channels))
        .collect();
    let poses: HashMap<&str, &PoseSnapshot> = snapshot
        .poses
        .iter()
        .map(|p| (p.name.as_str(), p))
        .collect();
    let names: BTreeSet<&str> = metadata
        .fixtures
        .keys()
        .map(String::as_str)
        .chain(channels.keys().copied())
        .chain(poses.keys().copied())
        .chain(snapshot.under_test.iter().map(String::as_str))
        .collect();

    let fixtures = names
        .into_iter()
        .map(|name| {
            let mut fixture = StageFixture {
                name: name.to_string(),
                channels: channels
                    .get(name)
                    .map(|c| sorted_names(c))
                    .unwrap_or_default(),
                pose: poses.get(name).map(|p| pose(p)),
                ..Default::default()
            };
            if let Some(meta) = metadata.fixtures.get(name) {
                let placement = &meta.placement;
                fixture.r#type = placement.fixture_type.clone();
                fixture.tags = placement.tags.clone();
                fixture.position = placement.position.map(vec3);
                fixture.rotation = placement.rotation.map(vec3);
                fixture.beam_angle = placement.beam_angle.map(|degrees| degrees as f32);
                fixture.rig = placement.rig.clone();
                fixture.capabilities = meta.capabilities.iter().map(capability_name).collect();
                fixture.cells = meta
                    .cells
                    .iter()
                    .map(|cell| StageCell {
                        name: cell.name.clone(),
                        offset: Some(vec3(cell.offset)),
                    })
                    .collect();
            }
            fixture
        })
        .collect();

    StageLayout {
        id: 0,
        fixtures,
        // Without the venues directory: that is for saving edits back.
        venue: metadata.venue.as_ref().map(|venue| StageVenue {
            name: venue.venue.name.clone(),
            focus_points: venue
                .venue
                .focus_points
                .iter()
                .map(|(name, point)| (name.clone(), vec3(*point)))
                .collect(),
            scenery: venue.venue.scenery.clone(),
            scenery_error: venue.venue.scenery_error.clone(),
        }),
    }
}

/// Each layout fixture's index, by name.
fn indexes(layout: &StageLayout) -> HashMap<&str, usize> {
    layout
        .fixtures
        .iter()
        .enumerate()
        .map(|(index, fixture)| (fixture.name.as_str(), index))
        .collect()
}

/// Whether a frame for `snapshot` can be read against `layout`: every
/// fixture the snapshot names is in the layout, each one it carries has
/// exactly the layout's channels, and the layout gives channels to no
/// fixture the snapshot has dropped. When not, a fresh layout goes out
/// before the frame.
pub(crate) fn fits(snapshot: &StateSnapshot, layout: &StageLayout) -> bool {
    let indexes = indexes(layout);
    let carried: HashMap<&str, &HashMap<String, u8>> = snapshot
        .fixtures
        .iter()
        .map(|f| (f.name.as_str(), &f.channels))
        .collect();
    let named = snapshot
        .poses
        .iter()
        .map(|p| p.name.as_str())
        .chain(snapshot.under_test.iter().map(String::as_str))
        .all(|name| indexes.contains_key(name));
    named
        && carried.iter().all(|(name, channels)| {
            indexes.get(name).is_some_and(|&index| {
                let layout_channels = &layout.fixtures[index].channels;
                layout_channels.len() == channels.len()
                    && layout_channels.iter().all(|c| channels.contains_key(c))
            })
        })
        && layout
            .fixtures
            .iter()
            .all(|f| f.channels.is_empty() || carried.contains_key(f.name.as_str()))
}

/// The frame for `snapshot`, read against `layout`, which it must fit
/// ([`fits`]), and whose id it carries. Everything is in the layout's
/// fixture order, so two frames of the same stage are equal.
pub(crate) fn stage_frame(snapshot: &StateSnapshot, layout: &StageLayout) -> StageFrame {
    let indexes = indexes(layout);
    let index_of = |name: &str| indexes.get(name).map(|&index| index as u32);

    // By name, the last of a name winning, as the JSON's maps have it.
    let carried: HashMap<&str, &crate::state::FixtureSnapshot> = snapshot
        .fixtures
        .iter()
        .map(|f| (f.name.as_str(), f))
        .collect();
    let mut channels = Vec::new();
    let mut strobes = Vec::new();
    let mut cells = Vec::new();
    for (index, fixture) in layout.fixtures.iter().enumerate() {
        let now = carried.get(fixture.name.as_str());
        // Every channel the layout names takes its byte, so the bytes after
        // it stay where the layout says they are.
        channels.extend(fixture.channels.iter().map(|channel| {
            now.and_then(|now| now.channels.get(channel))
                .copied()
                .unwrap_or(0)
        }));
        let Some(now) = now else {
            continue;
        };
        if let Some(hertz) = now.strobe_hz {
            strobes.push(StageStrobe {
                fixture: index as u32,
                hertz: hertz as f32,
            });
        }
        cells.extend(fixture_cells(index as u32, &now.cells));
    }

    let current: HashMap<&str, StagePose> = snapshot
        .poses
        .iter()
        .map(|p| (p.name.as_str(), pose(p)))
        .collect();
    let poses = layout
        .fixtures
        .iter()
        .enumerate()
        .filter_map(|(index, fixture)| {
            let now = current.get(fixture.name.as_str());
            (now != fixture.pose.as_ref()).then(|| StagePoseChange {
                fixture: index as u32,
                pose: now.cloned(),
            })
        })
        .collect();

    let mut under_test: Vec<u32> = snapshot
        .under_test
        .iter()
        .filter_map(|name| index_of(name))
        .collect();
    under_test.sort_unstable();
    under_test.dedup();

    StageFrame {
        layout: layout.id,
        channels,
        strobes,
        poses,
        cells,
        active_effects: snapshot.active_effects.clone(),
        under_test,
    }
}

/// A fixture's per-cell values, its cells grouped by the channels they
/// have: one group, for every fixture whose cells are alike.
fn fixture_cells(
    fixture: u32,
    cells: &BTreeMap<String, HashMap<String, u8>>,
) -> Vec<StageFixtureCells> {
    let mut groups: BTreeMap<Vec<String>, Vec<StageCellValues>> = BTreeMap::new();
    for (name, values) in cells {
        let channels = sorted_names(values);
        let bytes = channels.iter().map(|channel| values[channel]).collect();
        groups.entry(channels).or_default().push(StageCellValues {
            name: name.clone(),
            values: bytes,
        });
    }
    groups
        .into_iter()
        .map(|(channels, cells)| StageFixtureCells {
            fixture,
            channels,
            cells,
        })
        .collect()
}

#[cfg(test)]
mod test {
    use prost::Message;

    use super::*;
    use crate::lighting::fit::Capability;
    use crate::state::FixtureSnapshot;

    fn fixture(name: &str, channels: &[(&str, u8)]) -> FixtureSnapshot {
        FixtureSnapshot {
            name: name.to_string(),
            channels: channels.iter().map(|(c, v)| (c.to_string(), *v)).collect(),
            cells: Default::default(),
            strobe_hz: None,
        }
    }

    fn pointing(name: &str, pan: f64, floor: Option<[f64; 2]>) -> PoseSnapshot {
        PoseSnapshot {
            name: name.to_string(),
            pan,
            tilt: 45.5,
            aim: [0.1, -0.7, 0.7],
            floor,
        }
    }

    fn placed(fixture_type: &str) -> wire::FixtureMetadata {
        wire::FixtureMetadata {
            placement: wire::FixturePlacement {
                tags: Vec::new(),
                fixture_type: fixture_type.to_string(),
                position: None,
                rotation: None,
                beam_angle: None,
                rig: None,
            },
            capabilities: Vec::new(),
            cells: Vec::new(),
        }
    }

    /// A venue with a mover, a pixel bar and a par nothing drives.
    fn metadata() -> wire::StageMetadata {
        let mut head = placed("Head");
        head.placement.tags = vec!["back".to_string(), "mover".to_string()];
        head.placement.position = Some([-2.0, 3.5, 4.25]);
        head.placement.rotation = Some([0.0, 0.0, 180.0]);
        head.placement.beam_angle = Some(12.5);
        head.placement.rig = Some("rigs/head.glb".to_string());
        head.capabilities = vec![
            Capability::Color,
            Capability::PanTilt,
            Capability::ColorWheel,
        ];
        let mut bar = placed("Bar");
        bar.capabilities = vec![Capability::Cells];
        bar.cells = vec![
            wire::CellPlacement {
                name: "1".to_string(),
                offset: [-0.25, 0.0, 0.0],
            },
            wire::CellPlacement {
                name: "2".to_string(),
                offset: [0.25, 0.0, 0.0],
            },
        ];
        wire::StageMetadata {
            fixtures: BTreeMap::from([
                ("head".to_string(), head),
                ("bar".to_string(), bar),
                ("spare".to_string(), placed("Par")),
            ]),
            venue: Some(wire::VenueMetadata {
                venue: wire::StageVenue {
                    name: "club".to_string(),
                    focus_points: BTreeMap::from([("drummer".to_string(), [0.0, 2.75, 1.5])]),
                    scenery: Some("scenery/club.glb".to_string()),
                    scenery_error: None,
                },
                dir: Some("/somewhere/venues".to_string()),
            }),
        }
    }

    /// The stage `metadata()` describes, mid-show: the mover moving and
    /// strobing, a per-cell effect on the bar, a fixture the venue does not
    /// list, and a fixture test.
    fn snapshot() -> StateSnapshot {
        let mut head = fixture("head", &[("tilt", 3), ("pan", 2), ("dimmer", 1)]);
        head.strobe_hz = Some(12.5);
        let mut bar = fixture("bar", &[("red", 10), ("dimmer", 255)]);
        bar.cells = BTreeMap::from([
            ("1".to_string(), HashMap::from([("red".to_string(), 7)])),
            ("2".to_string(), HashMap::from([("red".to_string(), 9)])),
        ]);
        StateSnapshot {
            // Not in name order: the layout sorts.
            fixtures: vec![head, fixture("stray", &[("dimmer", 77)]), bar],
            active_effects: vec!["chase".to_string(), "wash".to_string()],
            poses: vec![pointing("head", 90.0, Some([1.5, -2.0]))],
            under_test: vec!["head".to_string()],
        }
    }

    fn empty() -> wire::StageMetadata {
        wire::StageMetadata {
            fixtures: BTreeMap::new(),
            venue: None,
        }
    }

    #[test]
    fn the_layout_lists_every_fixture_by_name_with_its_channels_sorted() {
        let layout = stage_layout(&metadata(), &snapshot());
        let names: Vec<&str> = layout.fixtures.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["bar", "head", "spare", "stray"]);
        assert_eq!(layout.fixtures[0].channels, ["dimmer", "red"]);
        assert_eq!(layout.fixtures[1].channels, ["dimmer", "pan", "tilt"]);
        assert!(
            layout.fixtures[2].channels.is_empty(),
            "only in the venue: placed, with no channels"
        );
        assert_eq!(layout.fixtures[2].r#type, "Par");
        assert_eq!(layout.fixtures[3].channels, ["dimmer"]);
        assert_eq!(layout.fixtures[3].r#type, "", "only in the snapshot");
        assert_eq!(layout.fixtures[3].position, None);
    }

    #[test]
    fn the_layout_carries_what_the_metadata_does_and_no_venues_directory() {
        let layout = stage_layout(&metadata(), &snapshot());
        let head = &layout.fixtures[1];
        assert_eq!(head.r#type, "Head");
        assert_eq!(head.tags, ["back", "mover"]);
        assert_eq!(head.position, Some(vec3([-2.0, 3.5, 4.25])));
        assert_eq!(head.rotation, Some(vec3([0.0, 0.0, 180.0])));
        assert_eq!(head.beam_angle, Some(12.5));
        assert_eq!(head.rig.as_deref(), Some("rigs/head.glb"));
        assert_eq!(head.capabilities, ["color", "pan_tilt", "color_wheel"]);
        assert_eq!(
            head.pose,
            Some(pose(&pointing("head", 90.0, Some([1.5, -2.0])))),
            "where it points as the layout is built"
        );

        let bar = &layout.fixtures[0];
        assert_eq!((bar.position, bar.rotation), (None, None));
        assert_eq!((bar.beam_angle, bar.rig.as_deref()), (None, None));
        assert_eq!(bar.pose, None);
        let cells: Vec<(&str, StageVec3)> = bar
            .cells
            .iter()
            .map(|c| (c.name.as_str(), c.offset.unwrap()))
            .collect();
        assert_eq!(
            cells,
            [
                ("1", vec3([-0.25, 0.0, 0.0])),
                ("2", vec3([0.25, 0.0, 0.0]))
            ]
        );

        let venue = layout.venue.as_ref().expect("the venue");
        assert_eq!(venue.name, "club");
        assert_eq!(venue.focus_points["drummer"], vec3([0.0, 2.75, 1.5]));
        assert_eq!(venue.scenery.as_deref(), Some("scenery/club.glb"));
        assert_eq!(venue.scenery_error, None);
        assert!(
            !layout.encode_to_vec().windows(9).any(|w| w == b"somewhere"),
            "the venues directory is for editing, and stays home"
        );

        assert_eq!(
            stage_layout(&empty(), &StateSnapshot::default()).venue,
            None
        );
    }

    #[test]
    fn a_frames_bytes_line_up_with_the_layouts_channel_names() {
        let snapshot = snapshot();
        let layout = stage_layout(&metadata(), &snapshot);
        let frame = stage_frame(&snapshot, &layout);
        // bar: dimmer, red; head: dimmer, pan, tilt; spare: none; stray: dimmer.
        assert_eq!(frame.channels, [255, 10, 1, 2, 3, 77]);
        let named: usize = layout.fixtures.iter().map(|f| f.channels.len()).sum();
        assert_eq!(frame.channels.len(), named);
    }

    #[test]
    fn a_frame_names_the_layout_it_is_read_against() {
        let snapshot = snapshot();
        let mut layout = stage_layout(&metadata(), &snapshot);
        assert_eq!(layout.id, 0, "the stream numbers its layouts");
        layout.id = 7;
        assert_eq!(stage_frame(&snapshot, &layout).layout, 7);
    }

    #[test]
    fn a_frame_says_only_the_poses_that_differ_from_the_layouts() {
        let mut snapshot = StateSnapshot {
            fixtures: vec![
                fixture("a", &[("pan", 0)]),
                fixture("b", &[("pan", 0)]),
                fixture("c", &[("pan", 0)]),
                fixture("d", &[("pan", 0)]),
            ],
            poses: vec![
                pointing("a", 10.0, None),
                pointing("b", 20.0, Some([1.0, 2.0])),
                pointing("c", 30.0, None),
            ],
            ..Default::default()
        };
        let layout = stage_layout(&empty(), &snapshot);
        assert!(
            stage_frame(&snapshot, &layout).poses.is_empty(),
            "nothing has moved since the layout"
        );

        // b moves, c loses its pose, d gains one, a stays.
        snapshot.poses = vec![
            pointing("a", 10.0, None),
            pointing("b", 20.0, None),
            pointing("d", 40.0, None),
        ];
        assert!(fits(&snapshot, &layout));
        assert_eq!(
            stage_frame(&snapshot, &layout).poses,
            [
                StagePoseChange {
                    fixture: 1,
                    pose: Some(pose(&pointing("b", 20.0, None))),
                },
                StagePoseChange {
                    fixture: 2,
                    pose: None,
                },
                StagePoseChange {
                    fixture: 3,
                    pose: Some(pose(&pointing("d", 40.0, None))),
                },
            ]
        );
    }

    #[test]
    fn a_frame_carries_strobes_cells_effects_and_fixtures_under_test() {
        let mut snapshot = snapshot();
        let layout = stage_layout(&metadata(), &snapshot);
        let frame = stage_frame(&snapshot, &layout);
        assert_eq!(
            frame.strobes,
            [StageStrobe {
                fixture: 1,
                hertz: 12.5
            }]
        );
        assert_eq!(
            frame.cells,
            [StageFixtureCells {
                fixture: 0,
                channels: vec!["red".to_string()],
                cells: vec![
                    StageCellValues {
                        name: "1".to_string(),
                        values: vec![7],
                    },
                    StageCellValues {
                        name: "2".to_string(),
                        values: vec![9],
                    },
                ],
            }]
        );
        assert_eq!(frame.active_effects, ["chase", "wash"]);
        assert_eq!(frame.under_test, [1]);

        // Cells that do not have the same channels are said apart.
        snapshot.fixtures[2]
            .cells
            .get_mut("2")
            .unwrap()
            .insert("green".to_string(), 4);
        let cells = stage_frame(&snapshot, &layout).cells;
        assert_eq!(cells.len(), 2);
        assert_eq!((cells[0].fixture, cells[1].fixture), (0, 0));
        assert_eq!(cells[0].channels, ["green", "red"]);
        assert_eq!(cells[0].cells[0].name, "2");
        assert_eq!(cells[0].cells[0].values, [4, 9]);
        assert_eq!(cells[1].channels, ["red"]);
        assert_eq!(cells[1].cells[0].name, "1");
    }

    #[test]
    fn a_snapshot_fits_a_layout_until_its_fixtures_or_channels_change() {
        let snapshot = snapshot();
        let layout = stage_layout(&metadata(), &snapshot);
        assert!(fits(&snapshot, &layout));

        // Values, strobes, effects and poses moving change nothing.
        let mut moved = snapshot.clone();
        moved.fixtures[0].channels.insert("pan".to_string(), 200);
        moved.fixtures[0].strobe_hz = None;
        moved.poses.clear();
        moved.active_effects.clear();
        assert!(fits(&moved, &layout));

        let mut another = snapshot.clone();
        another.fixtures.push(fixture("new", &[("dimmer", 0)]));
        assert!(!fits(&another, &layout), "a fixture the layout lacks");

        let mut wider = snapshot.clone();
        wider.fixtures[1].channels.insert("green".to_string(), 0);
        assert!(!fits(&wider, &layout), "a channel the layout lacks");

        let mut renamed = snapshot.clone();
        let value = renamed.fixtures[1].channels.remove("dimmer").unwrap();
        renamed.fixtures[1]
            .channels
            .insert("white".to_string(), value);
        assert!(!fits(&renamed, &layout), "as many channels, other names");

        let mut fewer = snapshot.clone();
        fewer.fixtures.remove(1);
        assert!(
            !fits(&fewer, &layout),
            "the layout gives bytes to a fixture that is gone"
        );

        // A venue fixture nothing drives was never given bytes.
        let mut posed = snapshot.clone();
        posed.poses.push(pointing("spare", 0.0, None));
        assert!(fits(&posed, &layout));
        posed.poses.push(pointing("unknown", 0.0, None));
        assert!(!fits(&posed, &layout), "a pose for a fixture not listed");

        let mut tested = snapshot.clone();
        tested.under_test.push("unknown".to_string());
        assert!(!fits(&tested, &layout), "a test on a fixture not listed");
        assert!(fits(&tested, &stage_layout(&metadata(), &tested)));
    }

    /// What a client makes of a layout and a frame, in the shape the
    /// websocket's `state` message has it.
    #[derive(Debug, PartialEq, Default)]
    struct Read {
        fixtures: BTreeMap<String, HashMap<String, u8>>,
        poses: BTreeMap<String, StagePose>,
        cells: BTreeMap<String, BTreeMap<String, HashMap<String, u8>>>,
        strobe_rates: BTreeMap<String, f32>,
        active_effects: Vec<String>,
        under_test: Vec<String>,
    }

    /// Reads a frame against its layout, as a client would: with nothing
    /// but the two messages.
    fn read(layout: &StageLayout, frame: &StageFrame) -> Read {
        let name = |index: u32| layout.fixtures[index as usize].name.clone();
        let mut read = Read::default();
        let mut bytes = frame.channels.iter().copied();
        for fixture in &layout.fixtures {
            if fixture.channels.is_empty() {
                continue;
            }
            let values = fixture
                .channels
                .iter()
                .map(|channel| (channel.clone(), bytes.next().expect("a byte a channel")))
                .collect();
            read.fixtures.insert(fixture.name.clone(), values);
        }
        assert_eq!(bytes.next(), None, "no bytes beyond the layout's channels");

        for fixture in &layout.fixtures {
            if let Some(pose) = &fixture.pose {
                read.poses.insert(fixture.name.clone(), *pose);
            }
        }
        for change in &frame.poses {
            match change.pose {
                Some(pose) => read.poses.insert(name(change.fixture), pose),
                None => read.poses.remove(&name(change.fixture)),
            };
        }
        for group in &frame.cells {
            let cells = read.cells.entry(name(group.fixture)).or_default();
            for cell in &group.cells {
                let values = group
                    .channels
                    .iter()
                    .cloned()
                    .zip(cell.values.iter().copied());
                cells.insert(cell.name.clone(), values.collect());
            }
        }
        for strobe in &frame.strobes {
            read.strobe_rates.insert(name(strobe.fixture), strobe.hertz);
        }
        read.active_effects = frame.active_effects.clone();
        read.under_test = frame.under_test.iter().map(|&i| name(i)).collect();
        read
    }

    /// The same, from the websocket's own message, with its geometry
    /// narrowed to the stream's `f32`.
    fn from_json(snapshot: &StateSnapshot) -> Read {
        let values = crate::webui::state::stage_frame(&snapshot.fixtures, &snapshot.poses);
        Read {
            fixtures: values.fixtures,
            poses: values
                .poses
                .iter()
                .map(|(n, p)| (n.clone(), pose(p)))
                .collect(),
            cells: values.cells,
            strobe_rates: values
                .strobe_rates
                .iter()
                .map(|(n, hz)| (n.clone(), *hz as f32))
                .collect(),
            active_effects: snapshot.active_effects.clone(),
            under_test: snapshot.under_test.clone(),
        }
    }

    #[test]
    fn the_stream_and_the_websocket_say_the_same_thing() {
        let metadata = metadata();
        let first = snapshot();
        let layout = stage_layout(&metadata, &first);
        // Over the wire and back: what a client holds.
        let layout = StageLayout::decode(layout.encode_to_vec().as_slice()).unwrap();
        let through = |snapshot: &StateSnapshot| {
            let frame = stage_frame(snapshot, &layout).encode_to_vec();
            read(&layout, &StageFrame::decode(frame.as_slice()).unwrap())
        };
        assert_eq!(through(&first), from_json(&first));

        // Later frames against the same layout: values, the pose, the
        // strobe, the cells and the test all move.
        let mut later = first.clone();
        later.fixtures[0].channels.insert("pan".to_string(), 200);
        later.fixtures[0].strobe_hz = None;
        later.fixtures[1].strobe_hz = Some(3.0);
        later.fixtures[2].cells.clear();
        later.poses = vec![pointing("head", 120.0, None)];
        later.under_test = vec!["bar".to_string(), "stray".to_string()];
        later.active_effects.clear();
        assert!(fits(&later, &layout));
        assert_eq!(through(&later), from_json(&later));

        let mut still = later.clone();
        still.poses.clear();
        assert_eq!(through(&still), from_json(&still));

        // And the layout says what the metadata does, name for name.
        for (name, meta) in &metadata.fixtures {
            let fixture = layout.fixtures.iter().find(|f| &f.name == name).unwrap();
            let json = serde_json::to_value(meta).unwrap();
            assert_eq!(json["type"], fixture.r#type);
            assert_eq!(json["tags"], serde_json::json!(fixture.tags));
            assert_eq!(
                json["capabilities"],
                serde_json::json!(fixture.capabilities)
            );
            assert_eq!(fixture.position, meta.placement.position.map(vec3));
            assert_eq!(fixture.rotation, meta.placement.rotation.map(vec3));
            assert_eq!(
                fixture.beam_angle,
                meta.placement.beam_angle.map(|degrees| degrees as f32)
            );
            assert_eq!(fixture.rig, meta.placement.rig);
            let cells: Vec<(String, Option<StageVec3>)> = meta
                .cells
                .iter()
                .map(|c| (c.name.clone(), Some(vec3(c.offset))))
                .collect();
            let said: Vec<(String, Option<StageVec3>)> = fixture
                .cells
                .iter()
                .map(|c| (c.name.clone(), c.offset))
                .collect();
            assert_eq!(said, cells);
        }
    }

    /// `fixtures` fixtures of `channels` channels each, lit, with nothing
    /// else going on: the frame an ordinary moment of a show sends.
    fn frame_size(fixtures: usize, channels: usize) -> (usize, usize) {
        let snapshot = StateSnapshot {
            fixtures: (0..fixtures)
                .map(|f| FixtureSnapshot {
                    name: format!("fixture_{f:02}"),
                    channels: (0..channels)
                        .map(|c| (format!("channel_{c}"), (f * 16 + c + 1) as u8))
                        .collect(),
                    cells: Default::default(),
                    strobe_hz: None,
                })
                .collect(),
            ..Default::default()
        };
        let mut layout = stage_layout(&empty(), &snapshot);
        // As the stream sends it: a layout always has an id.
        layout.id = 1;
        let frame = stage_frame(&snapshot, &layout);
        let json = crate::webui::state::build_state_json(&snapshot);
        (frame.encoded_len(), json.len())
    }

    /// The sizes the design quotes ("about 40 bytes" for the rig's eight
    /// fixtures, "under one packet" of 512 for fifty of eight channels),
    /// pinned so a change to the frame's shape shows what it costs.
    #[test]
    fn a_frame_is_as_small_as_the_design_says() {
        let (rig, rig_json) = frame_size(8, 5);
        let (big, big_json) = frame_size(50, 8);
        println!("8 fixtures x 5 channels: {rig} bytes (JSON state message: {rig_json})");
        println!("50 fixtures x 8 channels: {big} bytes (JSON state message: {big_json})");
        assert_eq!(rig, 44);
        assert_eq!(big, 405);

        // What moving costs: a pose that differs from the layout's.
        let mut snapshot = StateSnapshot {
            fixtures: vec![fixture("head", &[("pan", 0), ("tilt", 0)])],
            poses: vec![pointing("head", 10.0, Some([1.0, 2.0]))],
            ..Default::default()
        };
        let layout = stage_layout(&empty(), &snapshot);
        let still = stage_frame(&snapshot, &layout).encoded_len();
        snapshot.poses = vec![pointing("head", 20.0, Some([1.5, 2.0]))];
        let moving = stage_frame(&snapshot, &layout).encoded_len();
        println!(
            "a mover away from its layout pose: {} bytes",
            moving - still
        );
    }
}
