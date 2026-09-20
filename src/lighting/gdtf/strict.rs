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

//! A strict reading of a GDTF description, by the spec's rules rather than
//! this crate's lenient parser (design §19.5). The parser proper takes
//! what it can from any file a manufacturer ships; this asks whether a
//! file mtrack *writes* is one a console would accept: the mandatory
//! sections in the mandatory order, every node link resolving, every
//! `Name` within the spec's charset, every DMX value well formed. It is a
//! test oracle, not a gate on import.

use std::collections::{HashMap, HashSet};

use quick_xml::events::Event;
use quick_xml::Reader;

/// One element of the document.
#[derive(Debug, Default)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
}

impl Node {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Every descendant, depth first, self excluded.
    fn descendants(&self) -> Vec<&Node> {
        let mut out = Vec::new();
        for child in &self.children {
            out.push(child);
            out.extend(child.descendants());
        }
        out
    }
}

/// Every problem with the description, as one line each; empty means the
/// file passes every check here.
pub fn check(xml: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let root = match parse(xml) {
        Ok(root) => root,
        Err(e) => return vec![format!("not well-formed XML: {e}")],
    };
    if root.name != "GDTF" {
        problems.push(format!("root element is <{}>, not <GDTF>", root.name));
        return problems;
    }
    if root.attr("DataVersion").is_none() {
        problems.push("<GDTF> has no DataVersion".to_string());
    }
    let Some(fixture) = root.child("FixtureType") else {
        problems.push("no <FixtureType>".to_string());
        return problems;
    };
    check_fixture_type(fixture, &mut problems);
    problems
}

fn check_fixture_type(fixture: &Node, problems: &mut Vec<String>) {
    match fixture.attr("Name") {
        Some(name) if !name.is_empty() => check_name("FixtureType Name", name, problems),
        _ => problems.push("FixtureType has no Name".to_string()),
    }
    if let Some(short) = fixture.attr("ShortName") {
        if short.is_empty() {
            problems.push("FixtureType ShortName is empty".to_string());
        }
    }
    match fixture.attr("FixtureTypeID") {
        Some(id) if is_guid(id) => {}
        Some(id) => problems.push(format!("FixtureTypeID {id:?} is not a GUID")),
        None => problems.push("FixtureType has no FixtureTypeID".to_string()),
    }

    // The sections, in the order the spec mandates; the mandatory three
    // present.
    const ORDER: [&str; 9] = [
        "AttributeDefinitions",
        "Wheels",
        "PhysicalDescriptions",
        "Models",
        "Geometries",
        "DMXModes",
        "Revisions",
        "FTPresets",
        "Protocols",
    ];
    let mut last = 0;
    for child in &fixture.children {
        match ORDER.iter().position(|o| *o == child.name) {
            Some(i) if i >= last => last = i,
            Some(_) => problems.push(format!("<{}> is out of order", child.name)),
            None => problems.push(format!("unknown FixtureType section <{}>", child.name)),
        }
    }
    for mandatory in ["AttributeDefinitions", "Geometries", "DMXModes"] {
        if fixture.child(mandatory).is_none() {
            problems.push(format!("no <{mandatory}>"));
        }
    }

    // Attribute definitions: features and attributes are mandatory, and
    // every link between them resolves.
    let mut attributes: HashSet<String> = HashSet::new();
    let mut features: HashSet<String> = HashSet::new();
    let mut activation_groups: HashSet<String> = HashSet::new();
    if let Some(defs) = fixture.child("AttributeDefinitions") {
        if let Some(groups) = defs.child("ActivationGroups") {
            for g in groups.children_named("ActivationGroup") {
                if let Some(name) = g.attr("Name") {
                    check_name("ActivationGroup", name, problems);
                    activation_groups.insert(name.to_string());
                }
            }
        }
        match defs.child("FeatureGroups") {
            None => problems.push("no <FeatureGroups>".to_string()),
            Some(groups) => {
                for g in groups.children_named("FeatureGroup") {
                    let Some(group) = g.attr("Name") else {
                        problems.push("a FeatureGroup has no Name".to_string());
                        continue;
                    };
                    check_name("FeatureGroup", group, problems);
                    for f in g.children_named("Feature") {
                        match f.attr("Name") {
                            Some(feature) => {
                                features.insert(format!("{group}.{feature}"));
                            }
                            None => problems.push(format!("a Feature of {group} has no Name")),
                        }
                    }
                }
            }
        }
        match defs.child("Attributes") {
            None => problems.push("no <Attributes>".to_string()),
            Some(list) => {
                let nodes: Vec<&Node> = list.children_named("Attribute").collect();
                for a in &nodes {
                    let Some(name) = a.attr("Name") else {
                        problems.push("an Attribute has no Name".to_string());
                        continue;
                    };
                    check_name("Attribute", name, problems);
                    if !attributes.insert(name.to_string()) {
                        problems.push(format!("Attribute {name:?} is defined twice"));
                    }
                }
                for a in &nodes {
                    let name = a.attr("Name").unwrap_or("?");
                    match a.attr("Feature") {
                        Some(feature) if features.contains(feature) => {}
                        Some(feature) => problems.push(format!(
                            "Attribute {name:?} links to Feature {feature:?}, which is not defined"
                        )),
                        None => problems.push(format!("Attribute {name:?} has no Feature")),
                    }
                    if let Some(group) = a.attr("ActivationGroup") {
                        if !activation_groups.contains(group) {
                            problems.push(format!(
                                "Attribute {name:?} links to ActivationGroup {group:?}, which is not defined"
                            ));
                        }
                    }
                    if let Some(main) = a.attr("MainAttribute") {
                        if !attributes.contains(main) {
                            problems.push(format!(
                                "Attribute {name:?} links to MainAttribute {main:?}, which is not defined"
                            ));
                        }
                    }
                }
            }
        }
    }

    // Geometry: unique names, references to top-level geometries only.
    let mut geometry_names: HashSet<String> = HashSet::new();
    let mut top_level: HashSet<String> = HashSet::new();
    let mut referenced_templates: HashSet<String> = HashSet::new();
    if let Some(geometries) = fixture.child("Geometries") {
        for top in &geometries.children {
            if let Some(name) = top.attr("Name") {
                top_level.insert(name.to_string());
            }
            for node in std::iter::once(top).chain(top.descendants()) {
                // A Break is a reference's child, not a geometry.
                if node.name == "Break" {
                    continue;
                }
                match node.attr("Name") {
                    Some(name) => {
                        check_name("geometry", name, problems);
                        if !geometry_names.insert(name.to_string()) {
                            problems.push(format!("geometry {name:?} is defined twice"));
                        }
                    }
                    None => problems.push(format!("a <{}> geometry has no Name", node.name)),
                }
                if node.name == "GeometryReference" {
                    match node.attr("Geometry") {
                        Some(target) => referenced_templates.insert(target.to_string()),
                        None => {
                            problems.push("a GeometryReference names no Geometry".to_string());
                            false
                        }
                    };
                    for b in node.children_named("Break") {
                        match b.attr("DMXOffset").map(str::parse::<u32>) {
                            Some(Ok(offset)) if offset >= 1 => {}
                            _ => problems.push(format!(
                                "GeometryReference {:?} has a Break without a DMXOffset of 1 or more",
                                node.attr("Name").unwrap_or("?")
                            )),
                        }
                    }
                }
            }
        }
        for target in &referenced_templates {
            if !top_level.contains(target) {
                problems.push(format!(
                    "GeometryReference to {target:?}, which is not a top-level geometry"
                ));
            }
        }
    }

    // Modes: at least one, each on a top-level geometry, each channel on
    // a geometry of the tree, every link resolving, every value well
    // formed, offsets unique per break among the channels that are not
    // template channels (those repeat by design).
    let Some(modes) = fixture.child("DMXModes") else {
        return;
    };
    let mode_nodes: Vec<&Node> = modes.children_named("DMXMode").collect();
    if mode_nodes.is_empty() {
        problems.push("no <DMXMode>".to_string());
    }
    for mode in mode_nodes {
        let mode_name = mode.attr("Name").unwrap_or("?").to_string();
        check_name("DMXMode", &mode_name, problems);
        match mode.attr("Geometry") {
            Some(g) if top_level.contains(g) => {}
            Some(g) => problems.push(format!(
                "DMXMode {mode_name:?} is on geometry {g:?}, which is not top-level"
            )),
            None => problems.push(format!("DMXMode {mode_name:?} names no Geometry")),
        }
        let Some(channels) = mode.child("DMXChannels") else {
            problems.push(format!("DMXMode {mode_name:?} has no <DMXChannels>"));
            continue;
        };
        // Bytes used, per break — and per geometry for a template's
        // channels, which repeat per instance by design but must not
        // collide among themselves.
        let mut used: HashMap<(String, String, u32), String> = HashMap::new();
        let mut channel_names: HashSet<String> = HashSet::new();
        for channel in channels.children_named("DMXChannel") {
            let geometry = channel.attr("Geometry").unwrap_or("");
            if !geometry_names.contains(geometry) {
                problems.push(format!(
                    "DMXChannel on geometry {geometry:?}, which the tree does not have"
                ));
            }
            let logical: Vec<&Node> = channel.children_named("LogicalChannel").collect();
            let Some(first) = logical.first() else {
                problems.push(format!(
                    "a DMXChannel on {geometry:?} has no LogicalChannel"
                ));
                continue;
            };
            let first_attribute = first.attr("Attribute").unwrap_or("?");
            let channel_name = format!("{geometry}_{first_attribute}");
            // Several NoFeature channels on one geometry share a name by
            // the spec's own naming rule; manufacturers write them.
            if !channel_names.insert(channel_name.clone()) && first_attribute != "NoFeature" {
                problems.push(format!(
                    "DMXChannel {channel_name:?} is defined twice in mode {mode_name:?}"
                ));
            }
            let dmx_break = channel.attr("DMXBreak").unwrap_or("1").to_string();
            match channel.attr("Offset") {
                // A virtual channel; the wild writes an empty Offset as
                // often as the spec's "None".
                Some("None") | Some("") | None => {}
                Some(offsets) => {
                    for text in offsets.split(',') {
                        match text.trim().parse::<u32>() {
                            Ok(offset) if (1..=512).contains(&offset) => {
                                let scope = if referenced_templates.contains(geometry) {
                                    geometry.to_string()
                                } else {
                                    String::new()
                                };
                                if let Some(other) = used.insert(
                                    (dmx_break.clone(), scope, offset),
                                    channel_name.clone(),
                                ) {
                                    problems.push(format!(
                                        "byte {offset} of break {dmx_break} is used by both {other:?} and {channel_name:?}"
                                    ));
                                }
                            }
                            _ => problems.push(format!(
                                "DMXChannel {channel_name:?} has an offset {text:?} outside 1..512"
                            )),
                        }
                    }
                }
            }
            if let Some(highlight) = channel.attr("Highlight") {
                if highlight != "None" && parse_dmx_value(highlight).is_none() {
                    problems.push(format!(
                        "DMXChannel {channel_name:?} Highlight {highlight:?} is not a DMX value"
                    ));
                }
            }
            let mut function_paths: HashSet<String> = HashSet::new();
            for lc in &logical {
                let Some(attribute) = lc.attr("Attribute") else {
                    problems.push(format!(
                        "a LogicalChannel of {channel_name:?} has no Attribute"
                    ));
                    continue;
                };
                if !attributes.contains(attribute) {
                    problems.push(format!(
                        "LogicalChannel of {channel_name:?} uses Attribute {attribute:?}, which is not defined"
                    ));
                }
                let mut names: HashSet<String> = HashSet::new();
                let functions: Vec<&Node> = lc.children_named("ChannelFunction").collect();
                if functions.is_empty() {
                    problems.push(format!(
                        "LogicalChannel {attribute:?} of {channel_name:?} has no ChannelFunction"
                    ));
                }
                for f in functions {
                    let fname = f.attr("Name").unwrap_or("").to_string();
                    check_name("ChannelFunction", &fname, problems);
                    if !names.insert(fname.clone()) {
                        problems.push(format!(
                            "ChannelFunction {fname:?} appears twice in {channel_name:?}.{attribute}"
                        ));
                    }
                    function_paths.insert(format!("{channel_name}.{attribute}.{fname}"));
                    match f.attr("Attribute") {
                        Some("NoFeature") | None => {}
                        Some(a) if attributes.contains(a) => {}
                        Some(a) => problems.push(format!(
                            "ChannelFunction {fname:?} uses Attribute {a:?}, which is not defined"
                        )),
                    }
                    for key in ["DMXFrom", "Default"] {
                        if let Some(value) = f.attr(key) {
                            if parse_dmx_value(value).is_none() {
                                problems.push(format!(
                                    "ChannelFunction {fname:?} {key} {value:?} is not a DMX value"
                                ));
                            }
                        }
                    }
                }
            }
            if let Some(initial) = channel.attr("InitialFunction") {
                if !function_paths.contains(initial) {
                    problems.push(format!(
                        "DMXChannel {channel_name:?} InitialFunction {initial:?} names no function of its own"
                    ));
                }
            }
        }
    }
}

/// The spec's `Name` charset (Annex C): letters, digits and a short list
/// of punctuation in the first 128 code points; anything above.
fn check_name(what: &str, name: &str, problems: &mut Vec<String>) {
    let bad: Vec<char> = name
        .chars()
        .filter(|c| {
            c.is_ascii() && !c.is_ascii_alphanumeric() && !" \"#%'()*+-/:;<=>@_`".contains(*c)
        })
        .collect();
    if !bad.is_empty() {
        problems.push(format!(
            "{what} {name:?} holds {bad:?}, outside the Name charset"
        ));
    }
}

fn is_guid(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    parts.len() == 5
        && [8, 4, 4, 4, 12]
            .iter()
            .zip(&parts)
            .all(|(len, part)| part.len() == *len && part.chars().all(|c| c.is_ascii_hexdigit()))
}

fn parse_dmx_value(text: &str) -> Option<(u64, u8)> {
    let text = text.trim().trim_end_matches(['s', 'S']);
    let (value, bytes) = text.split_once('/')?;
    let value: u64 = value.trim().parse().ok()?;
    let bytes: u8 = bytes.trim().parse().ok()?;
    ((1..=8).contains(&bytes) && value < 1u64 << (8 * u32::from(bytes))).then_some((value, bytes))
}

/// A plain element tree; text is ignored (GDTF carries everything in
/// attributes).
fn parse(xml: &str) -> Result<Node, String> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<Node> = vec![Node::default()];
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(start) => {
                stack.push(node_of(&start)?);
            }
            Event::Empty(start) => {
                let node = node_of(&start)?;
                stack.last_mut().expect("root").children.push(node);
            }
            Event::End(_) => {
                let node = stack.pop().ok_or("unbalanced element")?;
                stack
                    .last_mut()
                    .ok_or("unbalanced element")?
                    .children
                    .push(node);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut root = stack.pop().ok_or("empty document")?;
    if !stack.is_empty() {
        return Err("unclosed element".to_string());
    }
    match root.children.len() {
        1 => Ok(root.children.remove(0)),
        n => Err(format!("{n} root elements")),
    }
}

fn node_of(start: &quick_xml::events::BytesStart<'_>) -> Result<Node, String> {
    let name = String::from_utf8_lossy(start.name().as_ref()).to_string();
    let mut attrs = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| e.to_string())?;
        let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|e| e.to_string())?
            .to_string();
        attrs.push((key, value));
    }
    Ok(Node {
        name,
        attrs,
        children: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The generator's own output for a rich mover: the base every rule
    /// is tried against. (The parser's synthetic fixture is deliberately
    /// not a valid file; it exercises leniency.)
    fn generated() -> String {
        let types = crate::lighting::parser::parse_fixture_types(
            "fixture_type \"Mover\" {\n  channel \"pan\" @ 1 fine 2 range -270deg..270deg\n  channel \"tilt\" @ 3 fine 4 range -135deg..135deg\n  channel \"dimmer\" @ 5\n  channel \"strobe\" @ 6 {\n    function \"open\" 0..15\n    function \"strobe\" 16..255 1hz..25hz\n  }\n}\n",
        )
        .unwrap();
        crate::lighting::export::gdtf::description(&types["Mover"]).unwrap()
    }

    #[test]
    fn a_generated_description_passes() {
        let problems = check(&generated());
        assert!(problems.is_empty(), "{problems:#?}");
    }

    /// Each rule catches what it is for.
    #[test]
    fn broken_files_are_named_by_what_is_broken() {
        let base = generated();
        let cases: [(&str, &str, &str); 7] = [
            (
                r#"<LogicalChannel Attribute="Pan""#,
                r#"<LogicalChannel Attribute="Pann""#,
                "not defined",
            ),
            (
                r#"<Geometry Name="Body"/>"#,
                r#"<Geometry Name="Body.1"/>"#,
                "Name charset",
            ),
            (
                r#"<DMXMode Name="mtrack" Geometry="Body">"#,
                r#"<DMXMode Name="mtrack" Geometry="Nope">"#,
                "not top-level",
            ),
            (r#"DMXFrom="16/1""#, r#"DMXFrom="700/1""#, "not a DMX value"),
            (r#"Offset="5" "#, r#"Offset="6" "#, "used by both"),
            (
                r#"InitialFunction="Body_Pan.Pan.pan""#,
                r#"InitialFunction="Body_Pan.Pan.nope""#,
                "names no function",
            ),
            (r#"FixtureTypeID=""#, r#"FixtureTypeID="nope"#, "not a GUID"),
        ];
        for (from, to, expected) in cases {
            assert!(base.contains(from), "test text changed: {from}");
            let xml = base.replacen(from, to, 1);
            let problems = check(&xml);
            assert!(
                problems.iter().any(|p| p.contains(expected)),
                "{from} → {to}: expected {expected:?} in {problems:#?}"
            );
        }
        // Sections out of order, and a mandatory one missing.
        let reordered = base.replace(
            "    <Wheels/>\n    <PhysicalDescriptions/>\n",
            "    <PhysicalDescriptions/>\n    <Wheels/>\n",
        );
        assert!(
            check(&reordered).iter().any(|p| p.contains("out of order")),
            "{:#?}",
            check(&reordered)
        );
        let i = base.find("    <Geometries>").unwrap();
        let j = base.find("    </Geometries>\n").unwrap() + "    </Geometries>\n".len();
        let without = format!("{}{}", &base[..i], &base[j..]);
        assert!(check(&without)
            .iter()
            .any(|p| p.contains("no <Geometries>")));
    }
}
