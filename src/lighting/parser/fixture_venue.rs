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

use std::collections::{BTreeMap, HashMap};
use std::error::Error;

use super::super::types::{
    ganged_from_cells, Cell, ChannelDef, ChannelFunction, Fixture, FixtureType, FixtureTypeV1,
    GdtfSource, MovementLimits, PhysicalRange, PhysicalUnit, Vec3, Venue, VenueSource,
};
use super::error::get_error_context;
use super::grammar::{LightingParser, Rule};
use pest::iterators::Pair;
use pest::Parser;

pub fn parse_fixture_types(content: &str) -> Result<HashMap<String, FixtureType>, Box<dyn Error>> {
    let mut fixture_types = HashMap::new();

    let pairs = match LightingParser::parse(Rule::file, content) {
        Ok(pairs) => pairs,
        Err(e) => {
            let (line, col) = match e.line_col {
                pest::error::LineColLocation::Pos((line, col)) => (line, col),
                pest::error::LineColLocation::Span((line, col), _) => (line, col),
            };
            return Err(format!(
                "Fixture types DSL parsing error at line {}, column {}: {}\n\nContent around error:\n{}",
                line,
                col,
                e.variant.message(),
                get_error_context(content, line, col)
            ).into());
        }
    };

    for pair in pairs {
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::fixture_type => {
                    let fixture_type = parse_fixture_type_definition(inner_pair)
                        .map_err(|e| format!("Failed to parse fixture type definition: {}", e))?;
                    fixture_types.insert(fixture_type.name().to_string(), fixture_type);
                }
                _ => {
                    // Skip non-fixture_type rules (like comments)
                }
            }
        }
    }

    Ok(fixture_types)
}

pub fn parse_venues(content: &str) -> Result<HashMap<String, Venue>, Box<dyn Error>> {
    let mut venues = HashMap::new();

    let pairs = match LightingParser::parse(Rule::file, content) {
        Ok(pairs) => pairs,
        Err(e) => {
            let (line, col) = match e.line_col {
                pest::error::LineColLocation::Pos((line, col)) => (line, col),
                pest::error::LineColLocation::Span((line, col), _) => (line, col),
            };
            return Err(format!(
                "Venues DSL parsing error at line {}, column {}: {}\n\nContent around error:\n{}",
                line,
                col,
                e.variant.message(),
                get_error_context(content, line, col)
            )
            .into());
        }
    };

    for pair in pairs {
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::venue => {
                    let venue = parse_venue_definition(inner_pair)
                        .map_err(|e| format!("Failed to parse venue definition: {}", e))?;
                    venues.insert(venue.name().to_string(), venue);
                }
                _ => {
                    // Ignore other rules
                }
            }
        }
    }

    Ok(venues)
}

fn parse_fixture_type_definition(pair: Pair<Rule>) -> Result<FixtureType, Box<dyn Error>> {
    let mut name = String::new();
    let mut channels = HashMap::new();
    let mut special_cases = Vec::new();
    let mut max_strobe_frequency = None;
    let mut min_strobe_frequency = None;
    let mut strobe_dmx_offset = None;
    let mut source = None;
    let mut movement = MovementLimits::default();
    let mut rich_defs: HashMap<String, ChannelDef> = HashMap::new();
    let mut cells: Vec<Cell> = Vec::new();

    for pair in pair.into_inner() {
        match pair.as_rule() {
            Rule::fixture_type_name => {
                name = extract_string(pair);
            }
            Rule::gdtf_source => {
                source = Some(parse_gdtf_source(pair)?);
            }
            Rule::fixture_type_content => {
                parse_fixture_content(
                    pair,
                    &mut channels,
                    &mut rich_defs,
                    &mut cells,
                    &mut movement,
                    &mut special_cases,
                    &mut max_strobe_frequency,
                    &mut min_strobe_frequency,
                    &mut strobe_dmx_offset,
                )?;
            }
            _ => {}
        }
    }

    // The rich form (design §15.6): every channel is a `channel` line, and
    // the type is built from the definitions directly. It does not mix with
    // the v1 map or strobe fields — a strobe belongs on its channel as a
    // function — nor with a GDTF reference, which brings its own channels.
    // Cells bring their fixture-level channels with them (design §17.2):
    // the first cell's, mirroring the rest. Their offsets must not collide
    // with each other or with the fixture-level lines.
    if !cells.is_empty() {
        let ganged =
            ganged_from_cells(&cells).map_err(|e| format!("fixture type \"{name}\": {e}"))?;
        let mut taken: Vec<(u16, String)> = rich_defs
            .iter()
            .flat_map(|(n, d)| {
                std::iter::once(d.offset)
                    .chain(d.fine)
                    .map(move |o| (o, n.clone()))
            })
            .collect();
        for cell in &cells {
            for (channel, def) in &cell.channels {
                for offset in std::iter::once(def.offset).chain(def.fine) {
                    if let Some((_, other)) = taken.iter().find(|(o, _)| *o == offset) {
                        return Err(format!(
                            "fixture type \"{name}\": cell \"{}\" channel \"{channel}\" offset \
                             {offset} is already used by \"{other}\"",
                            cell.name
                        )
                        .into());
                    }
                    taken.push((offset, format!("{}/{channel}", cell.name)));
                }
            }
        }
        for (channel, def) in ganged {
            if rich_defs.contains_key(&channel) {
                return Err(format!(
                    "fixture type \"{name}\": channel \"{channel}\" is both a fixture-level line \
                     and a cell channel; a cell channel's fixture-level form is derived"
                )
                .into());
            }
            rich_defs.insert(channel, def);
        }
    }
    if !rich_defs.is_empty() {
        if !channels.is_empty() {
            return Err(format!(
                "fixture type \"{name}\" mixes `channel` lines with a channel_map; use one \
                 form for the whole type"
            )
            .into());
        }
        if max_strobe_frequency.is_some()
            || min_strobe_frequency.is_some()
            || strobe_dmx_offset.is_some()
        {
            return Err(format!(
                "fixture type \"{name}\" mixes `channel` lines with strobe fields; describe \
                 the strobe as a function on its channel instead: \
                 `channel \"strobe\" @ N {{ function \"strobe\" 16..255 0.5hz..20hz }}`"
            )
            .into());
        }
        if source.is_some() {
            return Err(format!(
                "fixture type \"{name}\" declares `from gdtf(...)` and `channel` lines; a \
                 referential fixture's channels come from the GDTF — remove the channel \
                 lines (or drop the gdtf reference to define it natively)"
            )
            .into());
        }
        // The engine's strobe path keys on a channel named "strobe" with a
        // function named "strobe": a hertz function anywhere else parses
        // and then silently does nothing, which is worse than a refusal.
        for (channel, def) in &rich_defs {
            for function in &def.functions {
                let hertz = function
                    .physical
                    .is_some_and(|p| p.unit == PhysicalUnit::Hertz);
                if hertz && (channel != "strobe" || function.name != "strobe") {
                    return Err(format!(
                        "fixture type \"{name}\": the hertz function \"{}\" on channel \
                         \"{channel}\" would never drive a strobe — the engine looks for \
                         `channel \"strobe\" ... {{ function \"strobe\" ... }}`; rename them",
                        function.name
                    )
                    .into());
                }
            }
        }
        let mut fixture_type = FixtureType::from_channel_defs(name, rich_defs);
        fixture_type.set_movement(movement);
        fixture_type.set_cells(cells);
        return Ok(fixture_type);
    }

    // A referential fixture type carries only human additions (movement);
    // its channels and strobe parameters come from the GDTF. Letting either
    // ride along would silently lose whichever side the loader didn't pick.
    if source.is_some() {
        if !channels.is_empty() {
            return Err(format!(
                "fixture type \"{name}\" declares `from gdtf(...)` and a channel map; \
                 a referential fixture's channels come from the GDTF — remove the \
                 channel_map (or drop the gdtf reference to define it natively)"
            )
            .into());
        }
        if max_strobe_frequency.is_some()
            || min_strobe_frequency.is_some()
            || strobe_dmx_offset.is_some()
        {
            return Err(format!(
                "fixture type \"{name}\" declares `from gdtf(...)` and strobe fields; \
                 a referential fixture's strobe parameters come from the GDTF's \
                 strobe function — remove the strobe fields (or drop the gdtf \
                 reference to define the fixture natively)"
            )
            .into());
        }
    }

    // The parser produces the v1 surface; From<FixtureTypeV1> is the single
    // normalization point into the internal model — no field pokes, no
    // manual step to forget.
    let mut fixture_type: FixtureType = FixtureTypeV1 {
        name,
        channels,
        max_strobe_frequency,
        min_strobe_frequency,
        strobe_dmx_offset,
    }
    .into();
    if let Some(source) = source {
        fixture_type.set_source(source);
    }
    fixture_type.set_movement(movement);
    Ok(fixture_type)
}

fn parse_gdtf_source(pair: Pair<Rule>) -> Result<GdtfSource, Box<dyn Error>> {
    // Exactly as written, inner whitespace included: a GDTF mode can be
    // named with a trailing space, and the name must pin that mode.
    let mut strings = pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::string)
        .map(|p| p.as_str().trim().trim_matches('"').to_string());
    let path = strings
        .next()
        .ok_or("gdtf reference requires an archive path")?;
    let mode = strings.next().ok_or("gdtf reference requires a mode")?;
    Ok(GdtfSource { path, mode })
}

fn parse_movement_block(pair: Pair<Rule>) -> Result<MovementLimits, Box<dyn Error>> {
    let mut movement = MovementLimits::default();
    let mut seen: Vec<String> = Vec::new();
    for param in pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::movement_param)
    {
        let mut name = String::new();
        let mut value = None;
        for inner in param.into_inner() {
            match inner.as_rule() {
                Rule::movement_param_name => name = inner.as_str().to_string(),
                Rule::speed_value => {
                    let text = inner.as_str().trim();
                    let number = text
                        .strip_suffix("deg/s")
                        .ok_or_else(|| format!("speed \"{text}\" must end in deg/s"))?;
                    value = Some(
                        number
                            .parse::<f64>()
                            .map_err(|e| format!("Invalid speed \"{text}\": {e}"))?,
                    );
                }
                _ => {}
            }
        }
        if seen.contains(&name) {
            return Err(format!("movement declares \"{name}\" more than once").into());
        }
        seen.push(name.clone());
        match name.as_str() {
            "max_pan_speed" => movement.max_pan_speed = value,
            "max_tilt_speed" => movement.max_tilt_speed = value,
            _ => {}
        }
    }
    Ok(movement)
}

/// Parses a physical value with its unit: `270deg`, `-135.5deg`, `0.5hz`.
fn parse_phys_value(text: &str) -> Result<(f64, PhysicalUnit), Box<dyn Error>> {
    let text = text.trim();
    let (number, unit) = if let Some(n) = text.strip_suffix("deg") {
        (n, PhysicalUnit::Degrees)
    } else if let Some(n) = text.strip_suffix("hz") {
        (n, PhysicalUnit::Hertz)
    } else {
        return Err(format!("physical value \"{text}\" needs a unit: deg or hz").into());
    };
    let value = number
        .parse::<f64>()
        .map_err(|e| format!("invalid physical value \"{text}\": {e}"))?;
    Ok((value, unit))
}

fn parse_phys_range(pair: Pair<Rule>) -> Result<PhysicalRange, Box<dyn Error>> {
    let mut values = pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::phys_value)
        .map(|p| parse_phys_value(p.as_str()));
    let (from, unit_from) = values.next().ok_or("range needs two values")??;
    let (to, unit_to) = values.next().ok_or("range needs two values")??;
    if unit_from != unit_to {
        return Err("both ends of a range must use the same unit".into());
    }
    Ok(PhysicalRange {
        from,
        to,
        unit: unit_from,
    })
}

fn parse_number<T: std::str::FromStr>(pair: &Pair<Rule>, what: &str) -> Result<T, Box<dyn Error>>
where
    T::Err: std::fmt::Display,
{
    pair.as_str()
        .trim()
        .parse::<T>()
        .map_err(|e| format!("invalid {what} \"{}\": {e}", pair.as_str().trim()).into())
}

/// Parses one `channel "name" @ N fine M range a..b { function ... }` line.
fn parse_channel_def(pair: Pair<Rule>) -> Result<(String, ChannelDef), Box<dyn Error>> {
    let mut name = String::new();
    let mut def = ChannelDef::at(0);
    let mut seen_offset = false;
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::string => name = extract_string(inner),
            Rule::offset_value => {
                def.offset = parse_number::<u16>(&inner, "channel offset")?;
                seen_offset = true;
            }
            Rule::channel_fine => {
                let number = inner
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::offset_value)
                    .ok_or("fine needs an offset")?;
                def.fine = Some(parse_number::<u16>(&number, "fine offset")?);
            }
            Rule::channel_range => {
                let range = inner
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::phys_range)
                    .ok_or("range needs two values")?;
                def.range = Some(parse_phys_range(range)?);
            }
            Rule::channel_block => {
                for function in inner
                    .into_inner()
                    .filter(|p| p.as_rule() == Rule::function_def)
                {
                    def.functions.push(parse_function_def(function)?);
                }
            }
            _ => {}
        }
    }
    if name.is_empty() {
        return Err("channel needs a name".into());
    }
    if name.contains('#') {
        return Err(reserved_channel_name(&name).into());
    }
    if !seen_offset || def.offset == 0 {
        return Err(format!("channel \"{name}\" needs a 1-based offset").into());
    }
    if def.fine == Some(def.offset) {
        return Err(format!("channel \"{name}\": fine byte cannot share the coarse offset").into());
    }
    if def.fine == Some(0) {
        return Err(format!("channel \"{name}\": fine offset is 1-based").into());
    }
    let mut seen_names = std::collections::HashSet::new();
    for function in &def.functions {
        if !seen_names.insert(function.name.as_str()) {
            return Err(format!(
                "channel \"{name}\" declares function \"{}\" more than once",
                function.name
            )
            .into());
        }
        if function.dmx_from > function.dmx_to {
            return Err(format!(
                "channel \"{name}\" function \"{}\": DMX range {}..{} runs backwards",
                function.name, function.dmx_from, function.dmx_to
            )
            .into());
        }
    }
    for (i, a) in def.functions.iter().enumerate() {
        for b in &def.functions[i + 1..] {
            if a.dmx_from <= b.dmx_to && b.dmx_from <= a.dmx_to {
                return Err(format!(
                    "channel \"{name}\": functions \"{}\" ({}..{}) and \"{}\" ({}..{}) overlap; \
                     each DMX value belongs to one function",
                    a.name, a.dmx_from, a.dmx_to, b.name, b.dmx_from, b.dmx_to
                )
                .into());
            }
        }
    }
    Ok((name, def))
}

fn parse_function_def(pair: Pair<Rule>) -> Result<ChannelFunction, Box<dyn Error>> {
    let mut function = ChannelFunction {
        name: String::new(),
        dmx_from: 0,
        dmx_to: 0,
        physical: None,
    };
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::string => function.name = extract_string(inner),
            Rule::dmx_range => {
                let mut numbers = inner
                    .into_inner()
                    .filter(|p| p.as_rule() == Rule::offset_value);
                let from = numbers.next().ok_or("DMX range needs two values")?;
                let to = numbers.next().ok_or("DMX range needs two values")?;
                function.dmx_from = parse_number::<u8>(&from, "DMX value")?;
                function.dmx_to = parse_number::<u8>(&to, "DMX value")?;
            }
            Rule::phys_range => function.physical = Some(parse_phys_range(inner)?),
            _ => {}
        }
    }
    if function.name.is_empty() {
        return Err("function needs a name".into());
    }
    Ok(function)
}

/// A `cell "name" at (x, y, z) { channel ... }` block.
fn parse_cell_def(pair: Pair<Rule>) -> Result<Cell, Box<dyn Error>> {
    let mut cell = Cell {
        name: String::new(),
        channels: HashMap::new(),
        offset: [0.0; 3],
    };
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::string => cell.name = extract_string(inner),
            Rule::cell_offset => {
                cell.offset = parse_vec3(single_vec3(inner)?)?;
            }
            Rule::channel_def => {
                let (name, def) = parse_channel_def(inner)?;
                if cell.channels.contains_key(&name) {
                    return Err(format!(
                        "cell \"{}\": channel \"{name}\" is declared more than once",
                        cell.name
                    )
                    .into());
                }
                cell.channels.insert(name, def);
            }
            _ => {}
        }
    }
    if cell.name.is_empty() {
        return Err("a cell needs a name".into());
    }
    if cell.channels.is_empty() {
        return Err(format!("cell \"{}\" declares no channels", cell.name).into());
    }
    Ok(cell)
}

#[allow(clippy::too_many_arguments)]
fn parse_fixture_content(
    pair: Pair<Rule>,
    channels: &mut HashMap<String, u16>,
    rich_defs: &mut HashMap<String, ChannelDef>,
    cells: &mut Vec<Cell>,
    movement: &mut MovementLimits,
    special_cases: &mut Vec<String>,
    max_strobe_frequency: &mut Option<f64>,
    min_strobe_frequency: &mut Option<f64>,
    strobe_dmx_offset: &mut Option<u8>,
) -> Result<(), Box<dyn Error>> {
    for content_pair in pair.into_inner() {
        match content_pair.as_rule() {
            Rule::channel_map => {
                *channels = parse_channel_mappings(content_pair);
                if let Some(name) = channels.keys().find(|k| k.contains('#')) {
                    return Err(reserved_channel_name(name).into());
                }
            }
            Rule::channel_def => {
                let (name, def) = parse_channel_def(content_pair)?;
                if rich_defs.contains_key(&name) {
                    return Err(format!("channel \"{name}\" is declared more than once").into());
                }
                let taken = rich_defs
                    .values()
                    .flat_map(|d| std::iter::once(d.offset).chain(d.fine));
                for offset in std::iter::once(def.offset).chain(def.fine) {
                    if taken.clone().any(|t| t == offset) {
                        return Err(format!(
                            "channel \"{name}\": offset {offset} is already used by another channel"
                        )
                        .into());
                    }
                }
                rich_defs.insert(name, def);
            }
            Rule::cell_def => {
                let cell = parse_cell_def(content_pair)?;
                if cells.iter().any(|c| c.name == cell.name) {
                    return Err(format!("cell \"{}\" is declared more than once", cell.name).into());
                }
                cells.push(cell);
            }
            Rule::movement_block => {
                *movement = parse_movement_block(content_pair)?;
            }
            Rule::max_strobe_frequency => {
                for inner in content_pair.into_inner() {
                    if inner.as_rule() == Rule::number_value {
                        let freq: f64 =
                            inner.as_str().trim().parse().map_err(|e| {
                                format!("Invalid max_strobe_frequency value: {}", e)
                            })?;
                        *max_strobe_frequency = Some(freq);
                    }
                }
            }
            Rule::min_strobe_frequency => {
                for inner in content_pair.into_inner() {
                    if inner.as_rule() == Rule::number_value {
                        let freq: f64 =
                            inner.as_str().trim().parse().map_err(|e| {
                                format!("Invalid min_strobe_frequency value: {}", e)
                            })?;
                        *min_strobe_frequency = Some(freq);
                    }
                }
            }
            Rule::strobe_dmx_offset => {
                for inner in content_pair.into_inner() {
                    if inner.as_rule() == Rule::number_value {
                        let offset: u8 = inner
                            .as_str()
                            .trim()
                            .parse()
                            .map_err(|e| format!("Invalid strobe_dmx_offset value: {}", e))?;
                        *strobe_dmx_offset = Some(offset);
                    }
                }
            }
            Rule::special_cases => {
                *special_cases = parse_special_case_list(content_pair);
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_channel_mappings(pair: Pair<Rule>) -> HashMap<String, u16> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::channel_mapping_list)
        .flat_map(|list| list.into_inner())
        .filter(|p| p.as_rule() == Rule::channel_mapping)
        .filter_map(|mapping| {
            let mut key = String::new();
            let mut value = 0u16;

            for inner in mapping.into_inner() {
                match inner.as_rule() {
                    Rule::channel_name => key = extract_string(inner),
                    Rule::channel_number => value = inner.as_str().trim().parse().unwrap_or(0),
                    _ => {}
                }
            }
            if !key.is_empty() && value > 0 {
                Some((key, value))
            } else {
                None
            }
        })
        .collect()
}

fn parse_special_case_list(pair: Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::special_case_list)
        .flat_map(|list| list.into_inner())
        .filter(|p| p.as_rule() == Rule::special_case)
        .map(|case| extract_string(case))
        .collect()
}

/// `#` is what a state snapshot uses to name a ganged channel's repeats
/// (`red#2`), so a channel of the fixture's own cannot carry it.
fn reserved_channel_name(name: &str) -> String {
    format!(
        "channel \"{name}\": `#` is reserved in channel names (it marks a ganged repeat in state)"
    )
}

fn extract_string(pair: Pair<Rule>) -> String {
    pair.as_str().trim_matches('"').trim().to_string()
}

fn parse_venue_definition(pair: Pair<Rule>) -> Result<Venue, Box<dyn Error>> {
    let mut name = String::new();
    let mut body = VenueBody::default();

    for pair in pair.into_inner() {
        match pair.as_rule() {
            Rule::string => {
                name = extract_string(pair);
            }
            Rule::venue_content => {
                parse_venue_content(pair, &mut body)?;
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return Err("Venue name is required".into());
    }

    Ok(Venue::new(name, body.fixtures)
        .with_focus_points(body.focus_points)
        .with_source(body.source))
}

/// What a venue body accumulates while parsing.
#[derive(Default)]
struct VenueBody {
    fixtures: HashMap<String, Fixture>,
    focus_points: BTreeMap<String, Vec3>,
    source: Option<VenueSource>,
}

/// Parses a `(x, y, z)` triple.
fn parse_vec3(pair: Pair<Rule>) -> Result<Vec3, Box<dyn Error>> {
    let mut values = [0.0; 3];
    let mut count = 0;
    for inner in pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::signed_number)
    {
        if count == 3 {
            break;
        }
        values[count] = inner
            .as_str()
            .trim()
            .parse::<f64>()
            .map_err(|e| format!("Invalid coordinate \"{}\": {e}", inner.as_str()))?;
        count += 1;
    }
    if count != 3 {
        return Err("a coordinate triple needs exactly three numbers".into());
    }
    Ok(values)
}

fn parse_venue_source(pair: Pair<Rule>) -> Result<VenueSource, Box<dyn Error>> {
    let mut mvr = None;
    let mut origin = [0.0; 3];
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::string => mvr = Some(extract_string(inner)),
            Rule::venue_origin => {
                let vec = inner
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::vec3)
                    .ok_or("origin requires a coordinate triple")?;
                origin = parse_vec3(vec)?;
            }
            _ => {}
        }
    }
    Ok(VenueSource {
        mvr: mvr.ok_or("mvr import requires an archive path")?,
        origin,
    })
}

fn parse_venue_content(pair: Pair<Rule>, body: &mut VenueBody) -> Result<(), Box<dyn Error>> {
    for content_pair in pair.into_inner() {
        match content_pair.as_rule() {
            Rule::fixture => {
                let fixture = parse_fixture_definition(content_pair)?;
                if body.fixtures.contains_key(fixture.name()) {
                    return Err(format!(
                        "venue declares fixture \"{}\" more than once",
                        fixture.name()
                    )
                    .into());
                }
                body.fixtures.insert(fixture.name().to_string(), fixture);
            }
            Rule::focus_point => {
                let mut name = String::new();
                let mut point = None;
                for inner in content_pair.into_inner() {
                    match inner.as_rule() {
                        Rule::string => name = extract_string(inner),
                        Rule::vec3 => point = Some(parse_vec3(inner)?),
                        _ => {}
                    }
                }
                if name.is_empty() {
                    return Err("focus point name is required".into());
                }
                if body.focus_points.contains_key(&name) {
                    return Err(
                        format!("venue declares focus point \"{name}\" more than once").into(),
                    );
                }
                body.focus_points.insert(
                    name,
                    point.ok_or("focus point requires a coordinate triple")?,
                );
            }
            Rule::venue_source => {
                if body.source.is_some() {
                    return Err("venue declares `imported from mvr(...)` more than once".into());
                }
                body.source = Some(parse_venue_source(content_pair)?);
            }
            // `group` still parses so this can say what to do about it. Venue
            // groups were superseded by fixture tags one release after they
            // landed; a bare pest failure here would only say "expected
            // fixture", which is no help to someone holding a rig file.
            Rule::group => {
                let name = content_pair
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::string)
                    .map(extract_string)
                    .unwrap_or_default();
                return Err(format!(
                    "venue group `{name}` is no longer supported. Tag the fixtures \
                     instead — add a tag to each member, e.g. `tags [\"{name}\"]`, \
                     then declare a logical group under `dmx.lighting.groups` in \
                     the player config:\n\n{}\n\
                     Tags survive a venue change; venue groups did not.",
                    migration_yaml(&name)
                )
                .into());
            }
            _ => {}
        }
    }
    Ok(())
}

/// The config the venue-group migration message tells the reader to write.
///
/// Extracted so a test can feed the *actual* advice to the loader rather than a
/// copy of it. `groups` is a map keyed by name, and an earlier version of this
/// message printed a sequence — following it produced a config that would not
/// load, which is worse than the error it replaced. A test that parses its own
/// hand-written string cannot catch that.
pub(crate) fn migration_yaml(name: &str) -> String {
    format!(
        "    groups:\n      {name}:\n        name: {name}\n        \
         constraints:\n          - AllOf: [\"{name}\"]\n"
    )
}

pub(crate) fn parse_fixture_definition(pair: Pair<Rule>) -> Result<Fixture, Box<dyn Error>> {
    let mut name: Option<String> = None;
    let mut fixture_type = String::new();
    let mut universe = 0u16;
    let mut start_channel = 0u16;
    let mut tags: Option<Vec<String>> = None;
    let mut position = None;
    let mut rotation = None;

    for pair in pair.into_inner() {
        match pair.as_rule() {
            // Positional: the first quoted value is the fixture's name, a
            // second is its type. Only the type may be given either way.
            Rule::string => match name {
                None => name = Some(extract_string(pair)),
                Some(_) => fixture_type = extract_string(pair),
            },
            Rule::identifier => {
                fixture_type = pair.as_str().to_string();
            }
            Rule::universe_num => {
                universe = pair.as_str().trim().parse()?;
            }
            Rule::address_num => {
                start_channel = pair.as_str().trim().parse()?;
            }
            Rule::tags => {
                if tags.is_some() {
                    return Err(duplicate_attribute(name.as_deref(), "tags"));
                }
                tags = Some(parse_tags(pair));
            }
            Rule::position => {
                if position.is_some() {
                    return Err(duplicate_attribute(name.as_deref(), "position"));
                }
                position = Some(parse_vec3(single_vec3(pair)?)?);
            }
            Rule::rotation => {
                if rotation.is_some() {
                    return Err(duplicate_attribute(name.as_deref(), "rotation"));
                }
                rotation = Some(parse_vec3(single_vec3(pair)?)?);
            }
            _ => {}
        }
    }

    Ok(Fixture::new(
        name.unwrap_or_default(),
        fixture_type,
        universe,
        start_channel,
        tags.unwrap_or_default(),
    )
    .with_position(position)
    .with_rotation(rotation))
}

fn duplicate_attribute(fixture: Option<&str>, attribute: &str) -> Box<dyn Error> {
    format!(
        "fixture \"{}\" declares `{attribute}` more than once",
        fixture.unwrap_or_default()
    )
    .into()
}

/// The `vec3` inside a `position`/`rotation` attribute.
fn single_vec3(pair: Pair<Rule>) -> Result<Pair<Rule>, Box<dyn Error>> {
    pair.into_inner()
        .find(|p| p.as_rule() == Rule::vec3)
        .ok_or_else(|| "coordinate triple required".into())
}

fn parse_tags(pair: Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::tag_list)
        .flat_map(|tag_list| {
            tag_list
                .into_inner()
                .filter(|p| p.as_rule() == Rule::string)
                .map(|tag| extract_string(tag))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_fixture_types ──────────────────────────────────────

    #[test]
    fn fixture_type_basic_channels() {
        let content = r#"fixture_type "LED_Par" {
    channels: 4
    channel_map: {
        "red": 1,
        "green": 2,
        "blue": 3,
        "dimmer": 4
    }
}"#;
        let result = parse_fixture_types(content).unwrap();
        assert_eq!(result.len(), 1);
        let ft = result.get("LED_Par").unwrap();
        assert_eq!(ft.name(), "LED_Par");
        assert_eq!(ft.channels().len(), 4);
        assert_eq!(ft.channels().get("red"), Some(&1));
        assert_eq!(ft.channels().get("dimmer"), Some(&4));
    }

    #[test]
    fn fixture_type_strobe_properties() {
        let content = r#"fixture_type "Strobe_Fix" {
    channels: 2
    channel_map: {
        "dimmer": 1,
        "strobe": 2
    }
    max_strobe_frequency: 25.0
    min_strobe_frequency: 0.5
    strobe_dmx_offset: 10
}"#;
        let result = parse_fixture_types(content).unwrap();
        let ft = result.get("Strobe_Fix").unwrap();
        assert_eq!(ft.max_strobe_frequency(), Some(25.0));
        assert_eq!(ft.min_strobe_frequency(), Some(0.5));
        assert_eq!(ft.strobe_dmx_offset(), Some(10));
    }

    #[test]
    fn fixture_type_no_strobe() {
        let content = r#"fixture_type "Simple" {
    channels: 1
    channel_map: {
        "dimmer": 1
    }
}"#;
        let result = parse_fixture_types(content).unwrap();
        let ft = result.get("Simple").unwrap();
        assert_eq!(ft.max_strobe_frequency(), None);
        assert_eq!(ft.min_strobe_frequency(), None);
        assert_eq!(ft.strobe_dmx_offset(), None);
    }

    #[test]
    fn fixture_type_multiple() {
        let content = r#"fixture_type "TypeA" {
    channels: 1
    channel_map: { "dimmer": 1 }
}

fixture_type "TypeB" {
    channels: 3
    channel_map: {
        "red": 1,
        "green": 2,
        "blue": 3
    }
}"#;
        let result = parse_fixture_types(content).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains_key("TypeA"));
        assert!(result.contains_key("TypeB"));
    }

    #[test]
    fn fixture_type_with_special_cases() {
        let content = r#"fixture_type "RGBW" {
    channels: 5
    channel_map: {
        "dimmer": 1,
        "red": 2,
        "green": 3,
        "blue": 4,
        "white": 5
    }
    special_cases: ["RGB", "Dimmer"]
}"#;
        let result = parse_fixture_types(content).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains_key("RGBW"));
    }

    #[test]
    fn fixture_type_strobe_fields_without_strobe_channel() {
        // Fields with no "strobe" channel to attach to stay plain fields —
        // through the real grammar, not just the Rust constructors.
        let content = r#"fixture_type "Blinder" {
    channels: 1
    channel_map: {
        "dimmer": 1
    }
    max_strobe_frequency: 25.0
    min_strobe_frequency: 0.5
    strobe_dmx_offset: 10
}"#;
        let result = parse_fixture_types(content).unwrap();
        let ft = result.get("Blinder").unwrap();
        assert_eq!(ft.max_strobe_frequency(), Some(25.0));
        assert_eq!(ft.min_strobe_frequency(), Some(0.5));
        assert_eq!(ft.strobe_dmx_offset(), Some(10));
        assert!(ft.channel_defs().get("strobe").is_none());
    }

    #[test]
    fn fixture_type_without_channel_map() {
        // The grammar allows a fixture type with no channel_map at all.
        let content = r#"fixture_type "Fieldsy" {
    channels: 2
    max_strobe_frequency: 20.0
}"#;
        let result = parse_fixture_types(content).unwrap();
        let ft = result.get("Fieldsy").unwrap();
        assert!(ft.channels().is_empty());
        assert_eq!(ft.max_strobe_frequency(), Some(20.0));
    }

    #[test]
    fn fixture_type_empty_input() {
        let result = parse_fixture_types("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn fixture_type_comments_only() {
        let content = "# This is a comment\n# Another comment\n";
        let result = parse_fixture_types(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn fixture_type_invalid_syntax() {
        let content = "fixture_type {";
        assert!(parse_fixture_types(content).is_err());
    }

    // ── referential fixture types ────────────────────────────────

    #[test]
    fn referential_fixture_type_parses_with_movement() {
        let content = r#"fixture_type "Brick"
  from gdtf("lighting/library/pb15.gdtf", mode "8: RGBS")
{
  movement { max_pan_speed: 240.0deg/s max_tilt_speed: 200deg/s }
}"#;
        let result = parse_fixture_types(content).unwrap();
        let ft = result.get("Brick").unwrap();
        let source = ft.source().unwrap();
        assert_eq!(source.path, "lighting/library/pb15.gdtf");
        assert_eq!(source.mode, "8: RGBS");
        assert_eq!(ft.movement().max_pan_speed, Some(240.0));
        assert_eq!(ft.movement().max_tilt_speed, Some(200.0));
        assert!(ft.channels().is_empty());
    }

    #[test]
    fn referential_with_channel_map_is_rejected() {
        let content = r#"fixture_type "Brick"
  from gdtf("x.gdtf", mode "M")
{
  channel_map: { "red": 1 }
}"#;
        let err = parse_fixture_types(content).unwrap_err().to_string();
        assert!(err.contains("channels come from the GDTF"), "{err}");
    }

    #[test]
    fn referential_with_strobe_fields_is_rejected() {
        // Silently dropping these was worse than refusing: the file looked
        // configured while the override never applied.
        let content = r#"fixture_type "Brick"
  from gdtf("x.gdtf", mode "M")
{
  max_strobe_frequency: 999.0
}"#;
        let err = parse_fixture_types(content).unwrap_err().to_string();
        assert!(
            err.contains("strobe parameters come from the GDTF"),
            "{err}"
        );
    }

    #[test]
    fn duplicate_movement_params_are_rejected() {
        let content = r#"fixture_type "M" {
  movement { max_pan_speed: 100deg/s max_pan_speed: 200deg/s }
}"#;
        let err = parse_fixture_types(content).unwrap_err().to_string();
        assert!(err.contains("more than once"), "{err}");
    }

    #[test]
    fn empty_movement_block_is_fine() {
        let content = r#"fixture_type "M" {
  movement { }
}"#;
        let ft = parse_fixture_types(content).unwrap();
        assert!(ft.get("M").unwrap().movement().is_empty());
    }

    #[test]
    fn speed_without_unit_is_a_parse_error() {
        let content = r#"fixture_type "M" {
  movement { max_pan_speed: 240 }
}"#;
        assert!(parse_fixture_types(content).is_err());
    }

    // ── parse_venues ─────────────────────────────────────────────

    #[test]
    fn venue_empty() {
        let content = r#"venue "Empty Hall" { }"#;
        let result = parse_venues(content).unwrap();
        assert_eq!(result.len(), 1);
        let v = result.get("Empty Hall").unwrap();
        assert_eq!(v.name(), "Empty Hall");
        assert!(v.fixtures().is_empty());
    }

    #[test]
    fn venue_with_fixtures() {
        let content = r#"venue "Club" {
    fixture "Spot1" GenericPar @ 1:1
    fixture "Spot2" GenericPar @ 1:5
    fixture "Spot3" GenericPar @ 2:1
}"#;
        let result = parse_venues(content).unwrap();
        let v = result.get("Club").unwrap();
        assert_eq!(v.fixtures().len(), 3);

        let s1 = v.fixtures().get("Spot1").unwrap();
        assert_eq!(s1.fixture_type(), "GenericPar");
        assert_eq!(s1.universe(), 1);
        assert_eq!(s1.start_channel(), 1);

        let s3 = v.fixtures().get("Spot3").unwrap();
        assert_eq!(s3.universe(), 2);
        assert_eq!(s3.start_channel(), 1);
    }

    #[test]
    fn venue_with_tags() {
        let content = r#"venue "Tagged" {
    fixture "Wash1" Par @ 1:1 tags ["front", "wash"]
    fixture "Wash2" Par @ 1:5 tags ["back"]
}"#;
        let result = parse_venues(content).unwrap();
        let v = result.get("Tagged").unwrap();
        assert_eq!(v.fixtures().len(), 2);

        let w1 = v.fixtures().get("Wash1").unwrap();
        assert_eq!(w1.tags(), &["front", "wash"]);

        let w2 = v.fixtures().get("Wash2").unwrap();
        assert_eq!(w2.tags(), &["back"]);
    }

    #[test]
    fn venue_group_is_rejected_with_migration_advice() {
        // Venue groups were superseded by fixture tags (#104, one release after
        // they landed) and are gone. The error has to be actionable: this is a
        // rig file someone may be reading at soundcheck.
        let content = r#"venue "Stage" {
    fixture "L1" Par @ 1:1
    fixture "L2" Par @ 1:5
    group "front" = L1, L2
}"#;
        let msg = match parse_venues(content) {
            Ok(_) => panic!("venue groups should be rejected"),
            Err(e) => e.to_string(),
        };
        assert!(msg.contains("front"), "should name the group: {msg}");
        assert!(msg.contains("tags"), "should point at tags: {msg}");
        assert!(
            msg.contains("dmx.lighting.groups"),
            "should point at logical groups: {msg}"
        );
    }

    #[test]
    fn venue_multiple() {
        let content = r#"venue "A" {
    fixture "F1" Par @ 1:1
}

venue "B" {
    fixture "F2" Par @ 2:1
}"#;
        let result = parse_venues(content).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains_key("A"));
        assert!(result.contains_key("B"));
    }

    #[test]
    fn venue_with_comments() {
        let content = r#"# Main venue
venue "Main" {
    fixture "F1" Par @ 1:1
}"#;
        let result = parse_venues(content).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains_key("Main"));
    }

    #[test]
    fn venue_empty_input() {
        let result = parse_venues("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn venue_invalid_syntax() {
        let content = "venue {";
        assert!(parse_venues(content).is_err());
    }

    // ── parse_fixture_definition (standalone) ────────────────────

    #[test]
    fn fixture_definition_basic() {
        let content = r#"fixture "MyLight" SomePar @ 3:17"#;
        let mut pairs = LightingParser::parse(Rule::fixture, content).unwrap();
        let pair = pairs.next().unwrap();
        let f = parse_fixture_definition(pair).unwrap();
        assert_eq!(f.name(), "MyLight");
        assert_eq!(f.fixture_type(), "SomePar");
        assert_eq!(f.universe(), 3);
        assert_eq!(f.start_channel(), 17);
    }

    #[test]
    fn fixture_definition_with_tags() {
        let content = r#"fixture "Spot" GenericSpot @ 1:100 tags ["front", "spot"]"#;
        let mut pairs = LightingParser::parse(Rule::fixture, content).unwrap();
        let pair = pairs.next().unwrap();
        let f = parse_fixture_definition(pair).unwrap();
        assert_eq!(f.name(), "Spot");
        assert_eq!(f.tags(), &["front", "spot"]);
    }

    #[test]
    fn a_one_digit_address_survives_a_trailing_comment() {
        // A seeded venue writes `# layer "..."` after every fixture; an
        // unplaced one ends at its address, and a one-digit address used
        // to swallow the comment (pest's implicit whitespace in a
        // non-atomic digit run).
        let venues = parse_venues(
            "venue \"V\" {\n  fixture \"A\" Par @ 1:9  # layer \"Club\"\n  fixture \"B\" Par @ 2:100  # layer \"Back Wall\"\n}\n",
        )
        .unwrap();
        let v = &venues["V"];
        assert_eq!(v.fixtures()["A"].start_channel(), 9);
        assert_eq!(v.fixtures()["B"].start_channel(), 100);
    }
}
