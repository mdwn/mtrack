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
    ChannelDef, ChannelFunction, FixtureType, PhysicalRange, PhysicalUnit,
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

/// Distills the named mode into a fixture type called `type_name`.
pub fn distill(
    description: &Description,
    mode_name: &str,
    type_name: &str,
) -> Result<Distilled, GdtfError> {
    let Some(mode) = description.modes.iter().find(|m| m.name == mode_name) else {
        let candidates: Vec<&str> = description.modes.iter().map(|m| m.name.as_str()).collect();
        return Err(GdtfError::new(format!(
            "GDTF \"{}\" has no mode named \"{mode_name}\"; available modes: {}",
            description.name,
            candidates.join(", ")
        )));
    };

    let mut warnings = Vec::new();
    let mut channel_defs: HashMap<String, ChannelDef> = HashMap::new();

    for channel in &mode.channels {
        // A channel attached to a GeometryReference is one instance of a
        // repeated cell — a pixel bar or multi-head mode. mtrack's model
        // has no instancing; a flattened import would be wrong, so the
        // whole mode is refused rather than half-imported.
        if description
            .geometry_reference_names
            .iter()
            .any(|name| *name == channel.geometry)
        {
            return Err(GdtfError::new(format!(
                "mode \"{}\" is multi-instance (channel on geometry reference \"{}\"); \
                 pixel/matrix modes are not supported — pick a non-pixel mode",
                mode.name, channel.geometry
            )));
        }

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

        let name = match canonical_channel_name(&logical.attribute) {
            Some(name) => name.to_string(),
            None => {
                let fallback = sanitize(&logical.attribute);
                warnings.push(format!(
                    "unmapped GDTF attribute \"{}\"; using channel name \"{fallback}\"",
                    logical.attribute
                ));
                fallback
            }
        };
        if channel_defs.contains_key(&name) {
            return Err(GdtfError::new(format!(
                "mode \"{}\" maps two channels to \"{name}\" — it appears to be \
                 multi-instance, which is not supported; pick a non-pixel mode",
                mode.name
            )));
        }

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
        def.functions = convert_functions(&name, channel, &mut warnings);

        channel_defs.insert(name, def);
    }

    Ok(Distilled {
        fixture_type: FixtureType::from_channel_defs(type_name.to_string(), channel_defs),
        warnings,
    })
}

/// mtrack's canonical channel name for a GDTF attribute, when one exists.
/// These are the names the engine's capability derivation keys on.
fn canonical_channel_name(attribute: &str) -> Option<&'static str> {
    Some(match attribute {
        "Dimmer" => "dimmer",
        "ColorAdd_R" => "red",
        "ColorAdd_G" => "green",
        "ColorAdd_B" => "blue",
        "ColorAdd_W" | "ColorAdd_WW" | "ColorAdd_CW" => "white",
        "ColorAdd_UV" => "uv",
        "Pan" => "pan",
        "Tilt" => "tilt",
        "Zoom" => "zoom",
        "Focus1" => "focus",
        "Gobo1" => "gobo",
        "CTC" | "CTO" | "CTB" => "ct",
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
        assert!(err.contains("no mode named \"Nope\""), "{err}");
        assert!(err.contains("8: RGBS"), "{err}");
        assert!(err.contains("Mover 16bit"), "{err}");
    }

    #[test]
    fn multi_instance_modes_are_refused() {
        // A mode whose channels sit on a GeometryReference is pixel/matrix
        // instancing; flattening it would be wrong, so it must refuse.
        let xml = r#"<GDTF><FixtureType Name="Bar" Manufacturer="m">
  <Geometries>
    <Geometry Name="Base"><GeometryReference Name="Pixel 1" Geometry="Cell"/></Geometry>
  </Geometries>
  <DMXModes>
    <DMXMode Name="Pixel Mode" Geometry="Base">
      <DMXChannels>
        <DMXChannel Offset="1" Geometry="Pixel 1">
          <LogicalChannel Attribute="ColorAdd_R">
            <ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/>
          </LogicalChannel>
        </DMXChannel>
      </DMXChannels>
    </DMXMode>
  </DMXModes>
</FixtureType></GDTF>"#;
        let description = parse_description(xml).unwrap();
        let err = distill(&description, "Pixel Mode", "Bar")
            .unwrap_err()
            .to_string();
        assert!(err.contains("multi-instance"), "{err}");
    }

    #[test]
    fn duplicate_canonical_names_are_refused() {
        // Two channels mapping to the same canonical name is instancing by
        // another route (or a broken file); either way, refuse.
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
        let err = distill(&description, "Twin Mode", "Twin")
            .unwrap_err()
            .to_string();
        assert!(err.contains("maps two channels to \"red\""), "{err}");
    }

    #[test]
    fn sanitize_makes_identifiers() {
        assert_eq!(sanitize("Strobe Off"), "strobe_off");
        assert_eq!(sanitize("Gobo (Rot.)"), "gobo_rot");
        assert_eq!(sanitize("__weird__"), "weird");
    }
}
