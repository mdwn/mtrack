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

//! Distills one GDTF mode into a [`FixtureType`].
//!
//! The distiller is the normalizer: GDTF's standardized attributes become
//! mtrack's canonical channel names, so imported configs never diverge on
//! spelling. Everything the distiller can't represent is skipped with a
//! named warning — never silently — per the design's skip-loudly stance.
//!
//! Changing what this module produces for the same input must bump
//! [`crate::lighting::distill::DISTILLER_VERSION`], which keys the
//! expansion cache.

use std::collections::HashMap;

use super::description::{Channel, Description};
use super::GdtfError;
use crate::lighting::types::{
    Cell, ChannelDef, ChannelFunction, FixtureType, PhysicalRange, PhysicalUnit,
};

/// A distilled fixture type plus everything the distiller had to skip or
/// guess — the seed of the import report.
#[derive(Debug)]
pub struct Distilled {
    /// The distilled fixture type. Referential metadata (source, movement
    /// overrides) is the caller's to apply.
    pub fixture_type: FixtureType,
    /// What was skipped or approximated, one line each.
    pub warnings: Vec<String>,
}

/// A mode's name and DMX footprint, for mode-picking UIs and errors.
#[derive(Debug, PartialEq, Eq)]
pub struct ModeSummary {
    /// The mode name a referential fixture pins.
    pub name: String,
    /// The number of DMX addresses the mode occupies.
    pub footprint: u16,
    /// How many channels the mode declares (virtual ones included).
    pub channel_count: usize,
}

/// Summarizes every mode in a description.
pub fn mode_summaries(description: &Description) -> Vec<ModeSummary> {
    description
        .modes
        .iter()
        .map(|mode| ModeSummary {
            name: mode.name.clone(),
            footprint: mode
                .channels
                .iter()
                .flat_map(|c| c.offsets.iter().copied())
                .max()
                .unwrap_or(0),
            channel_count: mode.channels.len(),
        })
        .collect()
}

/// A channel after pass one: its canonical name, attribute, geometry and
/// definition, before repeats are resolved.
#[derive(Clone)]
struct Named {
    name: String,
    attribute: String,
    geometry: String,
    dmx_break: String,
    def: ChannelDef,
}

/// Distills the named mode into a fixture type called `type_name`.
pub fn distill(
    description: &Description,
    mode_name: &str,
    type_name: &str,
) -> Result<Distilled, GdtfError> {
    // Exact, then with case/whitespace/punctuation folded: a GDTF can name
    // a mode with a trailing space, and a `.fixture` written from it can
    // come back through a parser that trims. The importer matches the same
    // way, so what it pins is what this finds.
    let matched = super::match_mode(description, mode_name)?;
    let mode = description
        .modes
        .iter()
        .find(|m| m.name == matched.name)
        .expect("match_mode names an existing mode");

    let mut warnings = Vec::new();

    // Pass one: each channel to a canonical name, keeping its geometry.
    // Real fixtures repeat an attribute across geometries — the cells of a
    // pixel bar, the identical sections of an LED batten, or the master
    // and per-section controls of a panel — and mtrack's model has one
    // channel per name. Pass two decides what to do with the repeats.
    let mut named: Vec<Named> = Vec::new();
    for channel in &mode.channels {
        let Some(logical) = channel.logical_channels.first() else {
            warnings.push(format!(
                "skipped a channel with no logical channel (geometry \"{}\")",
                channel.geometry
            ));
            continue;
        };
        if channel.logical_channels.len() > 1 {
            warnings.push(format!(
                "channel \"{}\": {} logical channels; using the first",
                logical.attribute,
                channel.logical_channels.len()
            ));
        }
        if channel.offsets.is_empty() {
            // Virtual channel: no DMX footprint, controlled by the console's
            // own math. Nothing for a patch-level model to carry.
            warnings.push(format!(
                "skipped virtual channel (no DMX offset): {}",
                logical.attribute
            ));
            continue;
        }
        if logical.attribute == "NoFeature" {
            // GDTF's placeholder for a byte the fixture reserves: it has no
            // function and no name worth keeping.
            continue;
        }
        let name = match canonical_channel_name(&logical.attribute) {
            Some(name) => name.to_string(),
            None => {
                let fallback = sanitize(&logical.attribute);
                if !named.iter().any(|n| n.attribute == logical.attribute) {
                    warnings.push(format!(
                        "unmapped GDTF attribute \"{}\"; using channel name \"{fallback}\"",
                        logical.attribute
                    ));
                }
                fallback
            }
        };
        let mut def = ChannelDef::at(channel.offsets[0]);
        if channel.offsets.len() > 1 {
            def.fine = Some(channel.offsets[1]);
        }
        if channel.offsets.len() > 2 {
            warnings.push(format!(
                "channel \"{name}\": {}-byte resolution; keeping coarse+fine only",
                channel.offsets.len()
            ));
        }
        def.range = channel_range(&logical.attribute, channel);
        if def.range.is_none() && (logical.attribute == "Pan" || logical.attribute == "Tilt") {
            warnings.push(format!(
                "channel \"{name}\": no {} function carries a physical range; \
                 the channel has no degree mapping",
                logical.attribute
            ));
        }
        def.functions = convert_functions(&name, channel, &mut warnings);
        named.push(Named {
            name,
            attribute: logical.attribute.clone(),
            geometry: channel.geometry.clone(),
            dmx_break: channel.dmx_break.clone(),
            def,
        });
    }

    // Pass one and a half: template channels. A channel on a geometry that
    // `GeometryReference`s instantiate is a template — the pixel mode of a
    // Spiider puts RGBW on `Lens2` once and references it six times, each
    // reference's `Break` saying where its bytes start. Each such channel
    // becomes one channel per reference at the template offset plus the
    // reference's break offset minus one, on the reference's geometry, so
    // the sections below are the real ones, named as the rig model names
    // them.
    named = expand_templates(named, description, &mut warnings);

    // Pass two. Geometries carrying exactly the same attribute set are
    // identical sections — pixels, batten segments — and are ganged: the
    // first section's channels are the fixture's, and every other
    // section's bytes mirror them, so the whole fixture shows one color.
    // Sections that differ (a master beside per-section controls) keep the
    // first occurrence as the fixture's channel and the rest under
    // geometry-suffixed names, reachable by a `static` but driven by
    // nothing. Both are reported: neither is what the console would do.
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    for n in &named {
        match sections.iter_mut().find(|(g, _)| *g == n.geometry) {
            Some((_, attributes)) => attributes.push(n.attribute.clone()),
            None => sections.push((n.geometry.clone(), vec![n.attribute.clone()])),
        }
    }
    for (_, attributes) in &mut sections {
        attributes.sort();
    }
    let mut channel_defs: HashMap<String, ChannelDef> = HashMap::new();
    let mut owner_geometry: HashMap<String, String> = HashMap::new();
    let mut ganged: Vec<String> = Vec::new();
    let mut suffixed: Vec<String> = Vec::new();
    // Every channel by the geometry it sits on, in document order, so the
    // ganged sections can be handed back as cells (design §17.2), and
    // which owner geometry each ganged section mirrors.
    let mut by_geometry: Vec<(String, HashMap<String, ChannelDef>)> = Vec::new();
    for n in &named {
        let entry = match by_geometry.iter_mut().find(|(g, _)| *g == n.geometry) {
            Some(entry) => entry,
            None => {
                by_geometry.push((n.geometry.clone(), HashMap::new()));
                by_geometry.last_mut().expect("just pushed")
            }
        };
        entry.1.entry(n.name.clone()).or_insert_with(|| {
            let mut def = n.def.clone();
            def.mirrors.clear();
            def
        });
    }
    let mut ganged_onto: Vec<(String, String)> = Vec::new();
    // Every channel name derived from a base name (the base itself and its
    // section-suffixed forms), so a repeat can gang onto any same-shaped
    // section that already owns the name — not only the first section that
    // ever claimed it, which may be a differently shaped one (a Spiider's
    // wash `Background` owns `red` before its nineteen lenses arrive).
    let mut variants: HashMap<String, Vec<String>> = HashMap::new();
    for n in named {
        let same_shape = |g: &str| sections.iter().find(|(s, _)| s == g).map(|(_, a)| a);
        // Pan and tilt never gang: two heads exposing the same attributes
        // are still aimed independently, and mirroring one head's angles
        // onto the other would be wrong, not merely coarse.
        let aims = n.name == "pan" || n.name == "tilt";
        let gang_target = if aims {
            None
        } else {
            variants
                .get(&n.name)
                .into_iter()
                .flatten()
                .find(|entry| {
                    let owner = owner_geometry[entry.as_str()].as_str();
                    owner == n.geometry || same_shape(owner) == same_shape(&n.geometry)
                })
                .cloned()
        };
        if let Some(entry) = gang_target {
            let owner = owner_geometry[entry.as_str()].clone();
            let existing = channel_defs.get_mut(&entry).expect("variant is a channel");
            existing.mirrors.push((n.def.offset, n.def.fine));
            if owner != n.geometry && !ganged.contains(&n.geometry) {
                ganged.push(n.geometry.clone());
                ganged_onto.push((n.geometry.clone(), owner));
            }
            continue;
        }
        if let std::collections::hash_map::Entry::Vacant(vacant) =
            channel_defs.entry(n.name.clone())
        {
            owner_geometry.insert(n.name.clone(), n.geometry.clone());
            variants
                .entry(n.name.clone())
                .or_default()
                .push(n.name.clone());
            vacant.insert(n.def);
            continue;
        }
        {
            let suffix = sanitize(&n.geometry);
            let mut renamed = format!("{}:{suffix}", n.name);
            let mut ordinal = 2;
            while channel_defs.contains_key(&renamed) {
                renamed = format!("{}:{suffix}_{ordinal}", n.name);
                ordinal += 1;
            }
            suffixed.push(format!("{} → {renamed}", n.name));
            let geometry = n.geometry.clone();
            owner_geometry.insert(renamed.clone(), geometry);
            variants
                .entry(n.name.clone())
                .or_default()
                .push(renamed.clone());
            channel_defs.insert(renamed, n.def);
        }
    }
    // The cells: each ganged group's owner section and the sections ganged
    // onto it, in document order, each with its own channels and its place
    // in the fixture's frame. A fixture whose sections gang into more than
    // one group (two heads each with pixels) keeps the largest group.
    let mut cells: Vec<Cell> = Vec::new();
    if !ganged.is_empty() {
        let mut groups: Vec<Vec<String>> = Vec::new();
        for (geometry, owner) in &ganged_onto {
            if geometry == owner {
                continue;
            }
            match groups.iter_mut().find(|g| &g[0] == owner) {
                Some(group) => group.push(geometry.clone()),
                None => groups.push(vec![owner.clone(), geometry.clone()]),
            }
        }
        if let Some(group) = groups.iter().max_by_key(|g| g.len()) {
            for geometry in group {
                if let Some((_, channels)) = by_geometry.iter().find(|(g, _)| g == geometry) {
                    cells.push(Cell {
                        name: geometry.clone(),
                        channels: channels.clone(),
                        offset: geometry_offset(description, geometry),
                    });
                }
            }
        }
        warnings.push(format!(
            "{} identical section(s) ganged to the first ({}): every section receives the \
             same value — one color, one level — unless a show says `per: cell`; {} cell(s) \
             recorded",
            ganged.len(),
            ganged.join(", "),
            cells.len()
        ));
    }
    if !suffixed.is_empty() {
        warnings.push(format!(
            "repeated attributes on differing sections kept under section names, driven by \
             nothing unless a static names them: {}",
            suffixed.join(", ")
        ));
    }

    let mut fixture_type = FixtureType::from_channel_defs(type_name.to_string(), channel_defs);
    fixture_type.set_cells(cells);
    Ok(Distilled {
        fixture_type,
        warnings,
    })
}

/// Expands template channels (see the call site): a channel on a geometry
/// that references instantiate becomes one channel per reference, on the
/// reference's geometry, at the template offset plus the reference's break
/// offset minus one. A template referenced once is renamed too, so a cell
/// is always called what the rig model calls its lens.
fn expand_templates(
    named: Vec<Named>,
    description: &Description,
    warnings: &mut Vec<String>,
) -> Vec<Named> {
    let references_of = |geometry: &str| -> Vec<&super::description::GeometryNode> {
        description
            .geometries
            .iter()
            .filter(|g| g.kind == super::description::GeometryKind::Reference)
            .filter(|g| g.reference.as_deref() == Some(geometry))
            .collect()
    };
    let mut out = Vec::with_capacity(named.len());
    let mut expanded: Vec<String> = Vec::new();
    for n in named {
        let references = references_of(&n.geometry);
        if references.is_empty() {
            out.push(n);
            continue;
        }
        if !expanded.contains(&n.geometry) {
            expanded.push(n.geometry.clone());
        }
        for reference in references {
            let Some(shift) = break_shift(reference, &n.dmx_break) else {
                warnings.push(format!(
                    "reference \"{}\" of template \"{}\" has no Break for DMX break {}; skipped",
                    reference.name, n.geometry, n.dmx_break
                ));
                continue;
            };
            out.push(shifted(n.clone(), shift, Some(reference.name.clone())));
        }
    }
    if !expanded.is_empty() {
        warnings.push(format!(
            "template geometries expanded per reference: {}",
            expanded.join(", ")
        ));
    }
    out
}

/// The offset shift a reference applies to a template channel on `dmx_break`:
/// the matching `Break`'s DMXOffset minus one; `Overwrite` takes the last
/// Break, as the spec says.
fn break_shift(reference: &super::description::GeometryNode, dmx_break: &str) -> Option<u16> {
    let entry = if dmx_break.eq_ignore_ascii_case("overwrite") {
        reference.breaks.last()
    } else {
        reference.breaks.iter().find(|(b, _)| b == dmx_break)
    };
    entry.map(|(_, offset)| offset - 1)
}

/// `n` moved by `shift` bytes, onto `geometry` when given.
fn shifted(mut n: Named, shift: u16, geometry: Option<String>) -> Named {
    n.def.offset = n.def.offset.saturating_add(shift).min(512);
    n.def.fine = n.def.fine.map(|f| f.saturating_add(shift).min(512));
    if let Some(geometry) = geometry {
        n.geometry = geometry;
    }
    n
}

/// Where a geometry sits in the fixture's frame: the translation of its
/// transform chain from the root, meters. A name the tree lacks is at the
/// origin.
fn geometry_offset(description: &Description, name: &str) -> [f64; 3] {
    let Some(index) = description.geometries.iter().position(|g| g.name == name) else {
        return [0.0; 3];
    };
    let mut chain = Vec::new();
    let mut at = Some(index);
    while let Some(i) = at {
        chain.push(i);
        at = description.geometries[i].parent;
    }
    chain.reverse();
    let mut m = super::description::IDENTITY;
    for i in chain {
        let n = description.geometries[i].position;
        let mut out = [[0.0; 4]; 4];
        for (r, row) in out.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell = (0..4).map(|k| m[r][k] * n[k][c]).sum();
            }
        }
        m = out;
    }
    [m[0][3], m[1][3], m[2][3]]
}

/// mtrack's canonical channel name for a GDTF attribute, when one exists.
/// These are the names the engine's capability derivation keys on.
fn canonical_channel_name(attribute: &str) -> Option<&'static str> {
    Some(match attribute {
        "Dimmer" => "dimmer",
        "ColorAdd_R" => "red",
        "ColorAdd_G" => "green",
        "ColorAdd_B" => "blue",
        "ColorAdd_W" => "white",
        // Warm/cool white are distinct channels on tunable-white fixtures;
        // collapsing them onto "white" made two channels collide and
        // hard-refused legitimate fixtures. They keep distinct names (the
        // engine's white capability keys on "white" alone — refining that
        // is color-work territory).
        "ColorAdd_WW" => "warm_white",
        "ColorAdd_CW" => "cool_white",
        "ColorAdd_UV" => "uv",
        "Pan" => "pan",
        "Tilt" => "tilt",
        "Zoom" => "zoom",
        "Focus1" => "focus",
        "Gobo1" => "gobo",
        "CTC" => "ct",
        "CTO" => "cto",
        "CTB" => "ctb",
        "Prism1" => "prism",
        "Frost1" => "frost",
        "Iris" => "iris",
        "Effects1" => "effects",
        "Shutter1" => "strobe",
        _ => return None,
    })
}

/// The physical range a whole channel maps onto, for the attributes mtrack
/// models in physical units (pan/tilt angles).
fn channel_range(attribute: &str, channel: &Channel) -> Option<PhysicalRange> {
    if attribute != "Pan" && attribute != "Tilt" {
        return None;
    }
    let logical = channel.logical_channels.first()?;
    let main = logical
        .functions
        .iter()
        .find(|f| f.attribute == *attribute)?;
    Some(PhysicalRange {
        from: main.physical_from?,
        to: main.physical_to?,
        unit: PhysicalUnit::Degrees,
    })
}

/// Converts a channel's GDTF functions. Each function's DMX range ends
/// where the next begins (GDTF encodes only starts); frequencies on strobe
/// functions become Hz physicals, which is what lets the model derive the
/// strobe parameters.
fn convert_functions(
    channel_name: &str,
    channel: &Channel,
    warnings: &mut Vec<String>,
) -> Vec<ChannelFunction> {
    let Some(logical) = channel.logical_channels.first() else {
        return Vec::new();
    };

    let mut starts: Vec<(u8, &super::description::Function)> = logical
        .functions
        .iter()
        .map(|function| {
            let from = match &function.dmx_from {
                Some(value) => value.coarse(),
                None => {
                    warnings.push(format!(
                        "channel \"{channel_name}\": function \"{}\" has no DMXFrom; assuming 0",
                        function.name
                    ));
                    0
                }
            };
            (from, function)
        })
        .collect();
    starts.sort_by_key(|(from, _)| *from);
    // Two functions sharing a start would mint an inverted (unmatchable)
    // range for the earlier one. The sort is stable, so on a tie the later
    // document-order function wins — kept, loudly; the earlier is dropped.
    let mut deduped: Vec<(u8, &super::description::Function)> = Vec::with_capacity(starts.len());
    for (from, function) in starts {
        if let Some((last_from, dropped)) = deduped.last().copied() {
            if last_from == from {
                warnings.push(format!(
                    "channel \"{channel_name}\": functions \"{}\" and \"{}\" both start \
                     at DMX {from}; keeping the later one",
                    dropped.name, function.name
                ));
                deduped.pop();
            }
        }
        deduped.push((from, function));
    }
    let starts = deduped;

    let mut converted = Vec::with_capacity(starts.len());
    for (i, (dmx_from, function)) in starts.iter().enumerate() {
        let dmx_to = match starts.get(i + 1) {
            Some((next_from, _)) if *next_from > 0 => next_from - 1,
            Some(_) => 0,
            None => u8::MAX,
        };
        let physical = strobe_hz_range(function);
        converted.push(ChannelFunction {
            name: canonical_function_name(function),
            dmx_from: *dmx_from,
            dmx_to,
            physical,
        });
    }
    converted
}

/// The Hz range of a strobe-frequency function, when it has one.
fn strobe_hz_range(function: &super::description::Function) -> Option<PhysicalRange> {
    if !function.attribute.starts_with("Shutter") || !function.attribute.contains("Strobe") {
        return None;
    }
    Some(PhysicalRange {
        from: function.physical_from?,
        to: function.physical_to?,
        unit: PhysicalUnit::Hertz,
    })
}

/// Canonical function names: the variable-strobe function must be called
/// "strobe" — that's the name the model's strobe derivation keys on —
/// and everything else keeps its GDTF name, sanitized.
fn canonical_function_name(function: &super::description::Function) -> String {
    match function.attribute.as_str() {
        "Shutter1Strobe" => "strobe".to_string(),
        "Shutter1StrobeRandom" => "strobe_random".to_string(),
        _ => sanitize(&function.name),
    }
}

/// Lowercases and underscores a GDTF name into mtrack's identifier style.
fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    out.trim_end_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::super::description::{parse_description, tests::SYNTHETIC_DESCRIPTION};
    use super::*;

    #[test]
    fn mode_summaries_report_name_footprint_and_count() {
        let description = parse_description(SYNTHETIC_DESCRIPTION).unwrap();
        let summaries = mode_summaries(&description);
        assert_eq!(
            summaries,
            vec![
                ModeSummary {
                    name: "8: RGBS".to_string(),
                    footprint: 4,
                    channel_count: 5,
                },
                ModeSummary {
                    name: "Mover 16bit".to_string(),
                    footprint: 5,
                    channel_count: 3,
                },
            ]
        );
    }

    #[test]
    fn distills_the_rgbs_mode_to_the_hand_written_equivalent() {
        // The distilled model must agree with the hand-written v1 DSL for
        // the same fixture — the PixelBrick proof, through the real code
        // path, at the model level the engine consumes.
        let description = parse_description(SYNTHETIC_DESCRIPTION).unwrap();
        let distilled = distill(&description, "8: RGBS", "Astera-PixelBrick").unwrap();

        let hand_written = crate::lighting::parser::parse_fixture_types(
            r#"fixture_type "Astera-PixelBrick" {
  channels: 4
  channel_map: {
    "red": 1,
    "green": 2,
    "blue": 3,
    "strobe": 4
  }
  max_strobe_frequency: 25.0
  min_strobe_frequency: 0.4
  strobe_dmx_offset: 7
}"#,
        )
        .unwrap();
        let hand_written = hand_written.get("Astera-PixelBrick").unwrap();

        let ft = &distilled.fixture_type;
        assert_eq!(ft.channels(), hand_written.channels());
        assert_eq!(
            ft.max_strobe_frequency(),
            hand_written.max_strobe_frequency()
        );
        assert_eq!(
            ft.min_strobe_frequency(),
            hand_written.min_strobe_frequency()
        );
        assert_eq!(ft.strobe_dmx_offset(), hand_written.strobe_dmx_offset());

        // The virtual dimmer was skipped, loudly.
        assert!(
            distilled
                .warnings
                .iter()
                .any(|w| w.contains("virtual channel") && w.contains("Dimmer")),
            "{:?}",
            distilled.warnings
        );

        // The function table carries the whole strobe channel, ranges closed
        // off where the next function begins.
        let strobe = ft.channel_defs().get("strobe").unwrap();
        let names: Vec<&str> = strobe.functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["strobe_off", "strobe_random", "strobe"]);
        assert_eq!(strobe.functions[0].dmx_from, 0);
        assert_eq!(strobe.functions[0].dmx_to, 3);
        assert_eq!(strobe.functions[1].dmx_to, 6);
        assert_eq!(strobe.functions[2].dmx_to, 255);
    }

    #[test]
    fn distills_a_16bit_mover_with_ranges_and_warns_on_unmapped() {
        let description = parse_description(SYNTHETIC_DESCRIPTION).unwrap();
        let distilled = distill(&description, "Mover 16bit", "Mover").unwrap();
        let ft = &distilled.fixture_type;

        let pan = ft.channel_defs().get("pan").unwrap();
        assert_eq!(pan.offset, 1);
        assert_eq!(pan.fine, Some(2));
        let range = pan.range.unwrap();
        assert_eq!((range.from, range.to), (-270.0, 270.0));
        assert_eq!(range.unit, PhysicalUnit::Degrees);
        assert_eq!(ft.footprint(), 5);

        // The unknown attribute landed under a sanitized name, loudly.
        assert!(ft.channel_defs().contains_key("frobnicator"));
        assert!(
            distilled
                .warnings
                .iter()
                .any(|w| w.contains("unmapped GDTF attribute \"Frobnicator\"")),
            "{:?}",
            distilled.warnings
        );

        // The v1 view sees the coarse bytes only.
        assert_eq!(ft.channels().get("pan"), Some(&1));
        assert_eq!(ft.channels().get("tilt"), Some(&3));
    }

    #[test]
    fn unknown_mode_lists_the_candidates() {
        let description = parse_description(SYNTHETIC_DESCRIPTION).unwrap();
        let err = distill(&description, "Nope", "X").unwrap_err().to_string();
        assert!(err.contains("no mode matching \"Nope\""), "{err}");
        assert!(err.contains("8: RGBS"), "{err}");
        assert!(err.contains("Mover 16bit"), "{err}");
    }

    #[test]
    fn identical_sections_are_ganged_to_one_channel() {
        // Three pixels, each with its own RGB: the fixture gets one red,
        // one green, one blue, and every other pixel mirrors them.
        let xml = r#"<GDTF><FixtureType Name="Bar" Manufacturer="m">
  <Geometries>
    <Geometry Name="Base"><GeometryReference Name="Pixel 1" Geometry="Cell"/><GeometryReference Name="Pixel 2" Geometry="Cell"/><GeometryReference Name="Pixel 3" Geometry="Cell"/></Geometry>
  </Geometries>
  <DMXModes>
    <DMXMode Name="Pixel Mode" Geometry="Base">
      <DMXChannels>
        <DMXChannel Offset="1" Geometry="Pixel 1"><LogicalChannel Attribute="ColorAdd_R"><ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="2" Geometry="Pixel 1"><LogicalChannel Attribute="ColorAdd_G"><ChannelFunction Name="G" Attribute="ColorAdd_G" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="3" Geometry="Pixel 2"><LogicalChannel Attribute="ColorAdd_R"><ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="4" Geometry="Pixel 2"><LogicalChannel Attribute="ColorAdd_G"><ChannelFunction Name="G" Attribute="ColorAdd_G" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="5,6" Geometry="Pixel 3"><LogicalChannel Attribute="ColorAdd_R"><ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="7" Geometry="Pixel 3"><LogicalChannel Attribute="ColorAdd_G"><ChannelFunction Name="G" Attribute="ColorAdd_G" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
      </DMXChannels>
    </DMXMode>
  </DMXModes>
</FixtureType></GDTF>"#;
        let description = parse_description(xml).unwrap();
        let distilled = distill(&description, "Pixel Mode", "Bar").unwrap();
        let defs = distilled.fixture_type.channel_defs();
        assert_eq!(defs.len(), 2, "{:?}", defs.keys().collect::<Vec<_>>());
        assert_eq!(defs["red"].offset, 1);
        assert_eq!(defs["red"].mirrors, vec![(3, None), (5, Some(6))]);
        assert_eq!(defs["green"].mirrors, vec![(4, None), (7, None)]);
        assert_eq!(distilled.fixture_type.footprint(), 7);
        assert!(
            distilled.warnings.iter().any(|w| w.contains("ganged")),
            "{:?}",
            distilled.warnings
        );
    }

    /// The same three-pixel bar as `identical_sections_are_ganged_to_one_channel`,
    /// reused so the cell recording can be checked against the exact ganging
    /// this fixture already exercises.
    const PIXEL_BAR_XML: &str = r#"<GDTF><FixtureType Name="Bar" Manufacturer="m">
  <Geometries>
    <Geometry Name="Base"><GeometryReference Name="Pixel 1" Geometry="Cell"/><GeometryReference Name="Pixel 2" Geometry="Cell"/><GeometryReference Name="Pixel 3" Geometry="Cell"/></Geometry>
  </Geometries>
  <DMXModes>
    <DMXMode Name="Pixel Mode" Geometry="Base">
      <DMXChannels>
        <DMXChannel Offset="1" Geometry="Pixel 1"><LogicalChannel Attribute="ColorAdd_R"><ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="2" Geometry="Pixel 1"><LogicalChannel Attribute="ColorAdd_G"><ChannelFunction Name="G" Attribute="ColorAdd_G" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="3" Geometry="Pixel 2"><LogicalChannel Attribute="ColorAdd_R"><ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="4" Geometry="Pixel 2"><LogicalChannel Attribute="ColorAdd_G"><ChannelFunction Name="G" Attribute="ColorAdd_G" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="5" Geometry="Pixel 3"><LogicalChannel Attribute="ColorAdd_R"><ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="6" Geometry="Pixel 3"><LogicalChannel Attribute="ColorAdd_G"><ChannelFunction Name="G" Attribute="ColorAdd_G" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
      </DMXChannels>
    </DMXMode>
  </DMXModes>
</FixtureType></GDTF>"#;

    #[test]
    fn identical_sections_are_recorded_as_cells_in_order() {
        let description = parse_description(PIXEL_BAR_XML).unwrap();
        let distilled = distill(&description, "Pixel Mode", "Bar").unwrap();
        let cells = distilled.fixture_type.cells();
        assert_eq!(cells.len(), 3, "{cells:?}");
        assert_eq!(cells[0].name, "Pixel 1");
        assert_eq!(cells[1].name, "Pixel 2");
        assert_eq!(cells[2].name, "Pixel 3");

        // Each cell keeps its own offsets, unganged.
        assert_eq!(cells[0].channels["red"].offset, 1);
        assert!(cells[0].channels["red"].mirrors.is_empty());
        assert_eq!(cells[0].channels["green"].offset, 2);
        assert_eq!(cells[1].channels["red"].offset, 3);
        assert_eq!(cells[1].channels["green"].offset, 4);
        assert_eq!(cells[2].channels["red"].offset, 5);
        assert_eq!(cells[2].channels["green"].offset, 6);

        // No Position anywhere in the chain: every cell sits at the origin.
        for cell in cells {
            assert_eq!(cell.offset, [0.0, 0.0, 0.0], "{}", cell.name);
        }

        assert!(
            distilled
                .warnings
                .iter()
                .any(|w| w.contains("3 cell(s) recorded")),
            "{:?}",
            distilled.warnings
        );
    }

    #[test]
    fn cell_offsets_compose_the_geometry_transform_chain() {
        // The parent geometry carries a translation of its own; each cell's
        // offset is the parent's transform composed with the cell's.
        let xml = r#"<GDTF><FixtureType Name="Bar" Manufacturer="m">
  <Geometries>
    <Geometry Name="Base" Position="{1,0,0,0}{0,1,0,0}{0,0,1,-0.2}{0,0,0,1}">
      <GeometryReference Name="Pixel 1" Geometry="Cell" Position="{1,0,0,0.1}{0,1,0,0}{0,0,1,0}{0,0,0,1}"/>
      <GeometryReference Name="Pixel 2" Geometry="Cell" Position="{1,0,0,0.1}{0,1,0,0}{0,0,1,0}{0,0,0,1}"/>
      <GeometryReference Name="Pixel 3" Geometry="Cell" Position="{1,0,0,0.1}{0,1,0,0}{0,0,1,0}{0,0,0,1}"/>
    </Geometry>
  </Geometries>
  <DMXModes>
    <DMXMode Name="Pixel Mode" Geometry="Base">
      <DMXChannels>
        <DMXChannel Offset="1" Geometry="Pixel 1"><LogicalChannel Attribute="ColorAdd_R"><ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="2" Geometry="Pixel 1"><LogicalChannel Attribute="ColorAdd_G"><ChannelFunction Name="G" Attribute="ColorAdd_G" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="3" Geometry="Pixel 2"><LogicalChannel Attribute="ColorAdd_R"><ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="4" Geometry="Pixel 2"><LogicalChannel Attribute="ColorAdd_G"><ChannelFunction Name="G" Attribute="ColorAdd_G" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="5" Geometry="Pixel 3"><LogicalChannel Attribute="ColorAdd_R"><ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="6" Geometry="Pixel 3"><LogicalChannel Attribute="ColorAdd_G"><ChannelFunction Name="G" Attribute="ColorAdd_G" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
      </DMXChannels>
    </DMXMode>
  </DMXModes>
</FixtureType></GDTF>"#;
        let description = parse_description(xml).unwrap();
        let distilled = distill(&description, "Pixel Mode", "Bar").unwrap();
        let cells = distilled.fixture_type.cells();
        assert_eq!(cells.len(), 3, "{cells:?}");
        // Parent z −0.2 composed with the cell's own x 0.1.
        assert_eq!(cells[0].offset, [0.1, 0.0, -0.2]);
    }

    #[test]
    fn a_repeat_on_the_same_geometry_gangs_and_pan_tilt_never_do() {
        let xml = r#"<GDTF><FixtureType Name="Twin" Manufacturer="m">
  <Geometries><Geometry Name="Base"><Geometry Name="Head A"/><Geometry Name="Head B"/></Geometry></Geometries>
  <DMXModes>
    <DMXMode Name="M" Geometry="Base">
      <DMXChannels>
        <DMXChannel Offset="1" Geometry="Head A"><LogicalChannel Attribute="Dimmer"><ChannelFunction Name="D" Attribute="Dimmer" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="2" Geometry="Head A"><LogicalChannel Attribute="Dimmer"><ChannelFunction Name="D" Attribute="Dimmer" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="3" Geometry="Head A"><LogicalChannel Attribute="Pan"><ChannelFunction Name="P" Attribute="Pan" DMXFrom="0/1" PhysicalFrom="-270" PhysicalTo="270"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="4" Geometry="Head B"><LogicalChannel Attribute="Dimmer"><ChannelFunction Name="D" Attribute="Dimmer" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="5" Geometry="Head B"><LogicalChannel Attribute="Dimmer"><ChannelFunction Name="D" Attribute="Dimmer" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="6" Geometry="Head B"><LogicalChannel Attribute="Pan"><ChannelFunction Name="P" Attribute="Pan" DMXFrom="0/1" PhysicalFrom="-270" PhysicalTo="270"/></LogicalChannel></DMXChannel>
      </DMXChannels>
    </DMXMode>
  </DMXModes>
</FixtureType></GDTF>"#;
        let description = parse_description(xml).unwrap();
        let distilled = distill(&description, "M", "Twin").unwrap();
        let defs = distilled.fixture_type.channel_defs();
        // Both heads have the same shape: the dimmers gang, all four bytes.
        assert_eq!(
            defs["dimmer"].mirrors,
            vec![(2, None), (4, None), (5, None)]
        );
        // Pan does not: the second head keeps its own.
        assert_eq!(defs["pan"].offset, 3);
        assert!(defs["pan"].mirrors.is_empty());
        assert_eq!(defs["pan:head_b"].offset, 6);
    }

    #[test]
    fn differing_sections_keep_the_first_and_suffix_the_rest() {
        // A master dimmer beside a section with a dimmer and color: the
        // master's dimmer is the fixture's; the section's is kept by name.
        let xml = r#"<GDTF><FixtureType Name="Panel" Manufacturer="m">
  <Geometries><Geometry Name="Base"><Geometry Name="Master"/><Geometry Name="Section A"/></Geometry></Geometries>
  <DMXModes>
    <DMXMode Name="M" Geometry="Base">
      <DMXChannels>
        <DMXChannel Offset="1" Geometry="Master"><LogicalChannel Attribute="Dimmer"><ChannelFunction Name="D" Attribute="Dimmer" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="2" Geometry="Section A"><LogicalChannel Attribute="Dimmer"><ChannelFunction Name="D" Attribute="Dimmer" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="3" Geometry="Section A"><LogicalChannel Attribute="ColorAdd_R"><ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
        <DMXChannel Offset="4" Geometry="Master"><LogicalChannel Attribute="NoFeature"><ChannelFunction Name="N" Attribute="NoFeature" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
      </DMXChannels>
    </DMXMode>
  </DMXModes>
</FixtureType></GDTF>"#;
        let description = parse_description(xml).unwrap();
        let distilled = distill(&description, "M", "Panel").unwrap();
        let defs = distilled.fixture_type.channel_defs();
        assert_eq!(defs["dimmer"].offset, 1);
        assert!(defs["dimmer"].mirrors.is_empty());
        assert_eq!(defs["dimmer:section_a"].offset, 2);
        assert_eq!(defs["red"].offset, 3);
        assert!(!defs.contains_key("nofeature"), "NoFeature is skipped");
        assert!(distilled
            .warnings
            .iter()
            .any(|w| w.contains("dimmer → dimmer:section_a")));
        assert!(
            distilled.fixture_type.cells().is_empty(),
            "differing sections are not cells: {:?}",
            distilled.fixture_type.cells()
        );
    }

    #[test]
    fn twin_sections_with_one_attribute_each_are_ganged() {
        // Two geometries carrying the same single attribute are identical
        // sections: the second mirrors the first, and the report says so.
        let xml = r#"<GDTF><FixtureType Name="Twin" Manufacturer="m">
  <DMXModes>
    <DMXMode Name="Twin Mode" Geometry="Base">
      <DMXChannels>
        <DMXChannel Offset="1" Geometry="A">
          <LogicalChannel Attribute="ColorAdd_R">
            <ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/>
          </LogicalChannel>
        </DMXChannel>
        <DMXChannel Offset="2" Geometry="B">
          <LogicalChannel Attribute="ColorAdd_R">
            <ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/>
          </LogicalChannel>
        </DMXChannel>
      </DMXChannels>
    </DMXMode>
  </DMXModes>
</FixtureType></GDTF>"#;
        let description = parse_description(xml).unwrap();
        let distilled = distill(&description, "Twin Mode", "Twin").unwrap();
        let red = &distilled.fixture_type.channel_defs()["red"];
        assert_eq!((red.offset, red.mirrors.as_slice()), (1, &[(2, None)][..]));
        assert!(
            distilled.warnings.iter().any(|w| w.contains("ganged")),
            "{:?}",
            distilled.warnings
        );
    }

    #[test]
    fn tunable_white_fixtures_distill_cleanly() {
        // Warm white + cool white are distinct channels on any tunable-white
        // fixture; collapsing both onto "white" used to refuse the mode.
        let xml = r#"<GDTF><FixtureType Name="Tunable" Manufacturer="m">
  <DMXModes>
    <DMXMode Name="CCT Mode" Geometry="Base">
      <DMXChannels>
        <DMXChannel Offset="1" Geometry="Base">
          <LogicalChannel Attribute="ColorAdd_WW">
            <ChannelFunction Name="WW" Attribute="ColorAdd_WW" DMXFrom="0/1"/>
          </LogicalChannel>
        </DMXChannel>
        <DMXChannel Offset="2" Geometry="Base">
          <LogicalChannel Attribute="ColorAdd_CW">
            <ChannelFunction Name="CW" Attribute="ColorAdd_CW" DMXFrom="0/1"/>
          </LogicalChannel>
        </DMXChannel>
      </DMXChannels>
    </DMXMode>
  </DMXModes>
</FixtureType></GDTF>"#;
        let description = parse_description(xml).unwrap();
        let distilled = distill(&description, "CCT Mode", "Tunable").unwrap();
        assert_eq!(
            distilled.fixture_type.channels().get("warm_white"),
            Some(&1)
        );
        assert_eq!(
            distilled.fixture_type.channels().get("cool_white"),
            Some(&2)
        );
    }

    #[test]
    fn tied_function_starts_keep_the_later_one_loudly() {
        // Two functions sharing a DMXFrom used to mint an inverted
        // (unmatchable) range for the earlier one.
        let xml = r#"<GDTF><FixtureType Name="Tie" Manufacturer="m">
  <DMXModes>
    <DMXMode Name="Tie Mode" Geometry="Base">
      <DMXChannels>
        <DMXChannel Offset="1" Geometry="Base">
          <LogicalChannel Attribute="Shutter1">
            <ChannelFunction Name="Open" Attribute="Shutter1" DMXFrom="0/1"/>
            <ChannelFunction Name="Old Strobe" Attribute="Shutter1" DMXFrom="7/1"/>
            <ChannelFunction Name="Variable Strobe" Attribute="Shutter1Strobe" DMXFrom="7/1" PhysicalFrom="1" PhysicalTo="10"/>
          </LogicalChannel>
        </DMXChannel>
      </DMXChannels>
    </DMXMode>
  </DMXModes>
</FixtureType></GDTF>"#;
        let description = parse_description(xml).unwrap();
        let distilled = distill(&description, "Tie Mode", "Tie").unwrap();
        let strobe = &distilled.fixture_type.channel_defs()["strobe"];
        assert_eq!(strobe.functions.len(), 2, "{:?}", strobe.functions);
        for func in &strobe.functions {
            assert!(func.dmx_from <= func.dmx_to, "inverted range: {func:?}");
        }
        // The later document-order function won the tie...
        assert_eq!(strobe.functions[1].name, "strobe");
        assert_eq!(strobe.functions[1].dmx_from, 7);
        // ...and the drop was loud.
        assert!(
            distilled
                .warnings
                .iter()
                .any(|w| w.contains("both start at DMX 7")),
            "{:?}",
            distilled.warnings
        );
    }

    #[test]
    fn pan_without_a_degree_range_warns() {
        let xml = r#"<GDTF><FixtureType Name="P" Manufacturer="m">
  <DMXModes>
    <DMXMode Name="M" Geometry="Base">
      <DMXChannels>
        <DMXChannel Offset="1" Geometry="Base">
          <LogicalChannel Attribute="Pan">
            <ChannelFunction Name="Pan 1" Attribute="Pan" DMXFrom="0/1"/>
          </LogicalChannel>
        </DMXChannel>
      </DMXChannels>
    </DMXMode>
  </DMXModes>
</FixtureType></GDTF>"#;
        let description = parse_description(xml).unwrap();
        let distilled = distill(&description, "M", "P").unwrap();
        assert!(distilled.fixture_type.channel_defs()["pan"].range.is_none());
        assert!(
            distilled
                .warnings
                .iter()
                .any(|w| w.contains("no degree mapping")),
            "{:?}",
            distilled.warnings
        );
    }

    #[test]
    fn sanitize_makes_identifiers() {
        assert_eq!(sanitize("Strobe Off"), "strobe_off");
        assert_eq!(sanitize("Gobo (Rot.)"), "gobo_rot");
        assert_eq!(sanitize("__weird__"), "weird");
    }

    /// A wash section that owns `red` first, then a lens template
    /// referenced three times with Break offsets: the pixel-mode shape.
    const TEMPLATE_PIXELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<GDTF DataVersion="1.2"><FixtureType Name="Templ" Manufacturer="m">
  <Geometries>
    <Geometry Name="Base" Position="{1,0,0,0}{0,1,0,0}{0,0,1,0}{0,0,0,1}">
      <Geometry Name="Background"/>
      <Geometry Name="Zone">
        <GeometryReference Name="P1" Geometry="Lens" Position="{1,0,0,-0.1}{0,1,0,0}{0,0,1,0}{0,0,0,1}"><Break DMXBreak="1" DMXOffset="1"/><Break DMXBreak="1" DMXOffset="1"/></GeometryReference>
        <GeometryReference Name="P2" Geometry="Lens" Position="{1,0,0,0}{0,1,0,0}{0,0,1,0}{0,0,0,1}"><Break DMXBreak="1" DMXOffset="5"/><Break DMXBreak="1" DMXOffset="4"/></GeometryReference>
        <GeometryReference Name="P3" Geometry="Lens" Position="{1,0,0,0.1}{0,1,0,0}{0,0,1,0}{0,0,0,1}"><Break DMXBreak="1" DMXOffset="9"/><Break DMXBreak="1" DMXOffset="7"/></GeometryReference>
      </Geometry>
    </Geometry>
    <Geometry Name="Lens"/>
  </Geometries>
  <DMXModes><DMXMode Name="Pixel" Geometry="Base"><DMXChannels>
    <DMXChannel Offset="1,2" Geometry="Background"><LogicalChannel Attribute="ColorAdd_R"><ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/2"/></LogicalChannel></DMXChannel>
    <DMXChannel Offset="3,4" Geometry="Background"><LogicalChannel Attribute="ColorAdd_G"><ChannelFunction Name="G" Attribute="ColorAdd_G" DMXFrom="0/2"/></LogicalChannel></DMXChannel>
    <DMXChannel Offset="5" Geometry="Background"><LogicalChannel Attribute="Dimmer"><ChannelFunction Name="D" Attribute="Dimmer" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
    <DMXChannel Offset="10" Geometry="Lens"><LogicalChannel Attribute="ColorAdd_R"><ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
    <DMXChannel Offset="11" Geometry="Lens"><LogicalChannel Attribute="ColorAdd_G"><ChannelFunction Name="G" Attribute="ColorAdd_G" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
    <DMXChannel Offset="12" Geometry="Lens"><LogicalChannel Attribute="ColorAdd_B"><ChannelFunction Name="B" Attribute="ColorAdd_B" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
    <DMXChannel Offset="13" Geometry="Lens" DMXBreak="Overwrite"><LogicalChannel Attribute="ColorAdd_W"><ChannelFunction Name="W" Attribute="ColorAdd_W" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
  </DMXChannels></DMXMode></DMXModes>
</FixtureType></GDTF>"#;

    #[test]
    fn template_channels_expand_per_reference_and_gang_by_shape() {
        let description = parse_description(TEMPLATE_PIXELS).unwrap();
        let distilled = distill(&description, "Pixel", "Templ").unwrap();
        let ft = &distilled.fixture_type;
        // The wash keeps the plain names; the lens group, arriving second
        // with a different shape, is suffixed once and mirrors within.
        assert_eq!(ft.channels()["red"], 1);
        assert_eq!(
            ft.channel_defs()["red"].mirrors,
            Vec::<(u16, Option<u16>)>::new()
        );
        let lens_red = ft
            .channel_defs()
            .iter()
            .find(|(n, _)| n.starts_with("red:"))
            .map(|(n, d)| (n.clone(), d.clone()))
            .expect("suffixed lens red");
        assert_eq!(lens_red.1.offset, 10, "P1 at the template offset");
        assert_eq!(
            lens_red.1.mirrors,
            vec![(14, None), (18, None)],
            "P2, P3 shifted by their Breaks"
        );
        // Overwrite takes the last Break: 13, 13+3, 13+6.
        let lens_white = ft
            .channel_defs()
            .iter()
            .find(|(n, _)| n.starts_with("white"))
            .map(|(_, d)| d.clone())
            .unwrap();
        assert_eq!(lens_white.offset, 13);
        assert_eq!(lens_white.mirrors, vec![(16, None), (19, None)]);
        // Cells: the three references, with canonical channel names and
        // their offsets in the fixture's frame.
        let cells = ft.cells();
        assert_eq!(
            cells.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["P1", "P2", "P3"]
        );
        assert_eq!(cells[1].channels["red"].offset, 14);
        assert_eq!(cells[2].channels["white"].offset, 19);
        assert!((cells[0].offset[0] + 0.1).abs() < 1e-9 && (cells[2].offset[0] - 0.1).abs() < 1e-9);
        assert!(
            distilled
                .warnings
                .iter()
                .any(|w| w.contains("template geometries expanded")),
            "{:?}",
            distilled.warnings
        );
        assert!(
            distilled
                .warnings
                .iter()
                .any(|w| w.contains("3 cell(s) recorded")),
            "{:?}",
            distilled.warnings
        );
        assert_eq!(ft.footprint(), 20, "blue on P3: 12 + 8");
    }
}
