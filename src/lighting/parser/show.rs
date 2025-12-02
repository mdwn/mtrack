// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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

use std::collections::HashMap;
use std::error::Error;
use std::time::Duration;

use super::super::effects::EffectLayer;
use super::super::tempo::TempoMap;
use super::effect_parse::parse_effect_definition;
use super::error::{analyze_parsing_failure, get_error_context};
use super::grammar::{LightingParser, Rule};
use super::tempo_parse::parse_tempo_definition;
use super::types::{Cue, LayerCommand, LayerCommandType, LightShow, Sequence};
use super::types::{SequenceLoop, UnexpandedSequenceCue};
use super::utils::{parse_measure_time, parse_time_string};
use pest::iterators::Pair;
use pest::Parser;

/// Parses light shows from DSL content.
pub fn parse_light_shows(content: &str) -> Result<HashMap<String, LightShow>, Box<dyn Error>> {
    let pairs = match LightingParser::parse(Rule::file, content) {
        Ok(pairs) => pairs,
        Err(e) => {
            let (line, col) = match e.line_col {
                pest::error::LineColLocation::Pos((line, col)) => (line, col),
                pest::error::LineColLocation::Span((line, col), _) => (line, col),
            };
            return Err(format!(
                "DSL parsing error at line {}, column {}: {}\n\nContent around error:\n{}",
                line,
                col,
                e.variant.message(),
                get_error_context(content, line, col)
            )
            .into());
        }
    };

    let mut shows = HashMap::new();
    let mut sequences = HashMap::new();
    let mut global_tempo: Option<TempoMap> = None;
    let mut show_pairs = Vec::new();
    let mut sequence_pairs = Vec::new();

    // First pass: collect tempo sections, sequences, and show pairs
    for pair in pairs {
        match pair.as_rule() {
            Rule::tempo => {
                // Parse tempo at file level (applies to all shows if no show-specific tempo)
                global_tempo = Some(parse_tempo_definition(pair)?);
            }
            Rule::sequence => {
                sequence_pairs.push(pair);
            }
            Rule::light_show => {
                show_pairs.push(pair);
            }
            _ => {
                for inner_pair in pair.into_inner() {
                    match inner_pair.as_rule() {
                        Rule::tempo => {
                            global_tempo = Some(parse_tempo_definition(inner_pair)?);
                        }
                        Rule::sequence => {
                            sequence_pairs.push(inner_pair);
                        }
                        Rule::light_show => {
                            show_pairs.push(inner_pair);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Parse sequences in two passes to support forward references
    // First pass: Parse all sequence definitions and extract unexpanded cue data
    let mut unexpanded_sequences: Vec<(String, Option<TempoMap>, Vec<UnexpandedSequenceCue>)> =
        Vec::new();

    for pair in sequence_pairs {
        let (name, tempo_map, unexpanded_cues) = parse_sequence_structure(pair, &global_tempo)?;
        unexpanded_sequences.push((name, tempo_map, unexpanded_cues));
    }

    // Insert all sequences into the map (with empty cues for now)
    for (name, _tempo_map, _) in &unexpanded_sequences {
        let sequence = Sequence { cues: Vec::new() };
        sequences.insert(name.clone(), sequence);
    }

    // Second pass: Expand nested references in all sequences recursively
    // Build a map of unexpanded data for recursive expansion
    let mut unexpanded_data_map = HashMap::new();
    for (name, tempo_map, unexpanded_cues) in unexpanded_sequences {
        unexpanded_data_map.insert(name, (tempo_map, unexpanded_cues));
    }

    // Recursive function to expand a sequence and its dependencies
    fn expand_sequence_recursive(
        name: &str,
        sequences: &mut HashMap<String, Sequence>,
        unexpanded_data: &HashMap<String, (Option<TempoMap>, Vec<UnexpandedSequenceCue>)>,
        global_tempo: &Option<TempoMap>,
        expanded: &mut std::collections::HashSet<String>,
        recursion_stack: &mut Vec<String>,
    ) -> Result<(), Box<dyn Error>> {
        // Check for circular reference
        if recursion_stack.contains(&name.to_string()) {
            return Err(format!(
                "Circular sequence reference detected: {} -> {}",
                recursion_stack.join(" -> "),
                name
            )
            .into());
        }

        // If already expanded, skip
        if expanded.contains(name) {
            return Ok(());
        }

        // Get unexpanded data
        let (tempo_map, unexpanded_cues) = unexpanded_data
            .get(name)
            .ok_or_else(|| format!("Sequence '{}' not found in unexpanded data", name))?;

        // Add to recursion stack
        recursion_stack.push(name.to_string());

        let effective_tempo = tempo_map.as_ref().or(global_tempo.as_ref());
        let mut expanded_cues = Vec::new();

        // Expand all cues in this sequence
        for unexpanded_cue in unexpanded_cues {
            // Check if this cue references other sequences that need expansion first
            for (ref_seq_name, _) in &unexpanded_cue.sequence_references {
                // Recursively expand referenced sequences first
                expand_sequence_recursive(
                    ref_seq_name,
                    sequences,
                    unexpanded_data,
                    global_tempo,
                    expanded,
                    recursion_stack,
                )?;
            }

            // Now expand this cue
            let cues = expand_unexpanded_sequence_cue(
                unexpanded_cue.clone(),
                &effective_tempo.cloned(),
                sequences,
                recursion_stack,
            )?;
            expanded_cues.extend(cues);
        }

        // Remove from recursion stack
        recursion_stack.pop();

        // Mark as expanded and update the sequence
        if let Some(sequence) = sequences.get_mut(name) {
            sequence.cues = expanded_cues;
        }
        expanded.insert(name.to_string());

        Ok(())
    }

    // Expand all sequences (recursive expansion will handle dependencies)
    let mut expanded_sequence_names = std::collections::HashSet::new();
    for name in unexpanded_data_map.keys() {
        let mut recursion_stack = Vec::new();
        expand_sequence_recursive(
            name,
            &mut sequences,
            &unexpanded_data_map,
            &global_tempo,
            &mut expanded_sequence_names,
            &mut recursion_stack,
        )?;
    }

    // Second pass: parse shows with tempo and sequences available
    for pair in show_pairs {
        let mut show = parse_light_show_definition(pair, &global_tempo, &sequences)?;
        // If show doesn't have its own tempo, use global tempo
        if show.tempo_map.is_none() {
            show.tempo_map = global_tempo.clone();
        }
        shows.insert(show.name.clone(), show);
    }

    // If we have content that looks like a show but no shows were parsed, provide detailed analysis
    if shows.is_empty() && content.contains("show") {
        return Err(analyze_parsing_failure(content).into());
    }

    Ok(shows)
}

fn parse_light_show_definition(
    pair: Pair<Rule>,
    global_tempo: &Option<TempoMap>,
    sequences: &HashMap<String, Sequence>,
) -> Result<LightShow, Box<dyn Error>> {
    let mut name = String::new();
    let mut cues = Vec::new();
    let mut tempo_map: Option<TempoMap> = None;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::show_name => {
                name = inner_pair.as_str().trim_matches('"').to_string();
            }
            Rule::show_content => {
                // Parse the show content which contains cues and potentially tempo
                // First pass: collect tempo and cue pairs
                let mut tempo_pairs = Vec::new();
                let mut cue_pairs = Vec::new();

                for content_pair in inner_pair.into_inner() {
                    match content_pair.as_rule() {
                        Rule::tempo => {
                            tempo_pairs.push(content_pair);
                        }
                        Rule::cue => {
                            cue_pairs.push(content_pair);
                        }
                        _ => {}
                    }
                }

                // Parse tempo first (if any)
                for tempo_pair in tempo_pairs {
                    tempo_map = Some(parse_tempo_definition(tempo_pair)?);
                }

                // If no show-specific tempo, use global tempo for cue parsing
                let effective_tempo = tempo_map.as_ref().or(global_tempo.as_ref());

                // Track measure offset (starts at 0, accumulates as offset commands are encountered)
                let mut measure_offset: u32 = 0;

                // Then parse cues (now we have tempo_map and sequences)
                for cue_pair in cue_pairs {
                    let (parsed_cues, offset_change) = parse_cue_definition(
                        cue_pair,
                        &effective_tempo.cloned(),
                        sequences,
                        measure_offset,
                    )?;
                    cues.extend(parsed_cues);
                    // Update offset for subsequent cues
                    if let Some(change) = offset_change {
                        measure_offset = change;
                    }
                }
            }
            _ => {}
        }
    }

    Ok(LightShow {
        name,
        cues,
        tempo_map,
    })
}

/// Parse sequence structure without expanding nested references
/// Returns (name, tempo_map, unexpanded_cues) for later expansion
type SequenceStructureResult = (String, Option<TempoMap>, Vec<UnexpandedSequenceCue>);

fn parse_sequence_structure(
    pair: Pair<Rule>,
    global_tempo: &Option<TempoMap>,
) -> Result<SequenceStructureResult, Box<dyn Error>> {
    let mut name = String::new();
    let mut tempo_map: Option<TempoMap> = None;
    let mut unexpanded_cues = Vec::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::sequence_name => {
                name = inner_pair.as_str().trim_matches('"').to_string();
            }
            Rule::sequence_content => {
                // Parse the sequence content which contains cues and potentially tempo
                let mut tempo_pairs = Vec::new();
                let mut cue_pairs = Vec::new();

                for content_pair in inner_pair.into_inner() {
                    match content_pair.as_rule() {
                        Rule::tempo => {
                            tempo_pairs.push(content_pair);
                        }
                        Rule::sequence_cue => {
                            cue_pairs.push(content_pair);
                        }
                        _ => {}
                    }
                }

                // Parse tempo first (if any)
                for tempo_pair in tempo_pairs {
                    tempo_map = Some(parse_tempo_definition(tempo_pair)?);
                }

                // Parse cues but don't expand sequence references yet
                let effective_tempo = tempo_map.as_ref().or(global_tempo.as_ref());

                // Track measure offset (starts at 0, accumulates as offset commands are encountered)
                let mut measure_offset: u32 = 0;

                for cue_pair in cue_pairs {
                    let (unexpanded_cue, offset_change) = parse_sequence_cue_structure(
                        cue_pair,
                        &effective_tempo.cloned(),
                        measure_offset,
                    )?;
                    unexpanded_cues.push(unexpanded_cue);
                    // Update offset for subsequent cues
                    if let Some(change) = offset_change {
                        measure_offset = change;
                    }
                }
            }
            _ => {}
        }
    }

    Ok((name, tempo_map, unexpanded_cues))
}

/// Parse a sequence cue structure without expanding nested sequence references
/// Returns (cue, new_offset) where new_offset is Some(new_value) if offset changed, None otherwise
fn parse_sequence_cue_structure(
    pair: Pair<Rule>,
    tempo_map: &Option<TempoMap>,
    measure_offset: u32,
) -> Result<(UnexpandedSequenceCue, Option<u32>), Box<dyn Error>> {
    let mut time = Duration::ZERO;
    let mut effects = Vec::new();
    let mut layer_commands = Vec::new();
    let mut stop_sequences = Vec::new();
    let mut sequence_references = Vec::new();
    let mut effect_pairs = Vec::new();
    let mut layer_command_pairs = Vec::new();
    let mut sequence_ref_pairs = Vec::new();
    let mut stop_sequence_pairs = Vec::new();
    let mut offset_commands = Vec::new();
    let mut reset_commands = Vec::new();
    let mut new_offset: Option<u32> = None;

    // First pass: parse time and collect all pairs
    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::time_string => {
                time = parse_time_string(inner_pair.as_str())?;
            }
            Rule::measure_time => {
                let (measure, beat) = parse_measure_time(inner_pair.as_str())?;
                // Apply offset to measure number
                let playback_measure = measure + measure_offset;
                if let Some(tm) = tempo_map {
                    // For sequences, measure 1 should be 0.0s relative to the sequence start
                    // So we subtract the start_offset to make it relative to 0.0s
                    // Pass measure_offset so tempo changes respect offsets
                    let absolute_time = tm
                        .measure_to_time_with_offset(measure, beat, measure_offset)
                        .ok_or_else(|| {
                            format!(
                                "Invalid measure/beat position: {}/{}",
                                playback_measure, beat
                            )
                        })?;
                    time = absolute_time - tm.start_offset;
                } else {
                    return Err("Measure-based timing requires a tempo section".into());
                }
            }
            Rule::offset_command => {
                offset_commands.push(inner_pair);
            }
            Rule::reset_measures_command => {
                reset_commands.push(inner_pair);
            }
            Rule::effect => {
                effect_pairs.push(inner_pair);
            }
            Rule::layer_command => {
                layer_command_pairs.push(inner_pair);
            }
            Rule::sequence_reference => {
                sequence_ref_pairs.push(inner_pair);
            }
            Rule::stop_sequence_command => {
                stop_sequence_pairs.push(inner_pair);
            }
            _ => {}
        }
    }

    // Process offset commands
    // If reset is present, start from 0, then add any offset commands
    let base_offset = if !reset_commands.is_empty() {
        0
    } else {
        measure_offset
    };

    if !offset_commands.is_empty() {
        // Parse the offset value from the last offset command
        let mut total_offset = base_offset;
        for offset_pair in offset_commands {
            let offset_value = parse_offset_command(offset_pair)?;
            total_offset += offset_value;
        }
        new_offset = Some(total_offset);
    } else if !reset_commands.is_empty() {
        // Reset without offset - just set to 0
        new_offset = Some(0);
    }

    // Parse effects and layer commands (these can be parsed immediately)
    for effect_pair in effect_pairs {
        let effect = parse_effect_definition(effect_pair, tempo_map, time)?;
        effects.push(effect);
    }

    for layer_command_pair in layer_command_pairs {
        let layer_command = parse_layer_command(layer_command_pair, tempo_map, time)?;
        layer_commands.push(layer_command);
    }

    // Parse stop sequence commands
    for stop_pair in stop_sequence_pairs {
        let seq_name = stop_pair
            .into_inner()
            .find(|p| p.as_rule() == Rule::sequence_name)
            .ok_or("Stop sequence command missing sequence name")?
            .as_str()
            .trim_matches('"')
            .to_string();
        stop_sequences.push(seq_name);
    }

    // Parse sequence references (store as metadata for later expansion)
    for seq_ref_pair in sequence_ref_pairs {
        let mut seq_name = String::new();
        let mut loop_param: Option<SequenceLoop> = None;

        for inner in seq_ref_pair.into_inner() {
            match inner.as_rule() {
                Rule::sequence_name => {
                    seq_name = inner.as_str().trim_matches('"').to_string();
                }
                Rule::sequence_params => {
                    for param_pair in inner.into_inner() {
                        if param_pair.as_rule() == Rule::sequence_param {
                            let mut param_name = String::new();
                            let mut param_value = String::new();

                            for param_inner in param_pair.into_inner() {
                                match param_inner.as_rule() {
                                    Rule::sequence_param_name => {
                                        param_name = param_inner.as_str().to_string();
                                    }
                                    Rule::sequence_param_value => {
                                        param_value = param_inner.as_str().to_string();
                                    }
                                    _ => {}
                                }
                            }

                            if param_name == "loop" {
                                loop_param = Some(parse_sequence_loop_param(&param_value)?);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        sequence_references.push((seq_name, loop_param));
    }

    Ok((
        UnexpandedSequenceCue {
            time,
            effects,
            layer_commands,
            stop_sequences,
            sequence_references,
        },
        new_offset,
    ))
}

fn parse_sequence_loop_param(value: &str) -> Result<SequenceLoop, Box<dyn Error>> {
    match value.trim() {
        "once" => Ok(SequenceLoop::Once),
        "loop" => Ok(SequenceLoop::Loop),
        "pingpong" => Ok(SequenceLoop::PingPong),
        "random" => Ok(SequenceLoop::Random),
        numeric => {
            // Try to parse as a number
            let count: usize = numeric.parse()
                .map_err(|_| format!("Invalid loop count: '{}'. Expected 'once', 'loop', 'pingpong', 'random', or a number", numeric))?;
            if count == 0 {
                return Err("Loop count must be at least 1".into());
            }
            Ok(SequenceLoop::Count(count))
        }
    }
}

/// Parse an offset command to extract the number of measures
fn parse_offset_command(pair: Pair<Rule>) -> Result<u32, Box<dyn Error>> {
    // offset_command = { "offset" ~ number_value ~ "measures" }
    let mut number_str = String::new();
    let mut found_number = false;

    for inner_pair in pair.into_inner() {
        if inner_pair.as_rule() == Rule::number_value {
            number_str = inner_pair.as_str().trim().to_string();
            found_number = true;
        }
    }

    if !found_number {
        return Err("Offset command missing number value".into());
    }

    let offset: u32 = number_str
        .parse()
        .map_err(|e| format!("Failed to parse offset value '{}': {}", number_str, e))?;

    Ok(offset)
}

/// Expand an unexpanded sequence cue, resolving all nested sequence references
fn expand_unexpanded_sequence_cue(
    unexpanded: UnexpandedSequenceCue,
    _tempo_map: &Option<TempoMap>,
    sequences: &HashMap<String, Sequence>,
    recursion_stack: &mut Vec<String>,
) -> Result<Vec<Cue>, Box<dyn Error>> {
    // If there are sequence references, expand them
    if !unexpanded.sequence_references.is_empty() {
        let mut expanded_cues = Vec::new();

        // Expand each sequence reference
        for (seq_name, loop_param) in unexpanded.sequence_references {
            // Check for circular reference
            if recursion_stack.contains(&seq_name) {
                return Err(format!(
                    "Circular sequence reference detected: {} -> {}",
                    recursion_stack.join(" -> "),
                    seq_name
                )
                .into());
            }

            let sequence = sequences
                .get(&seq_name)
                .ok_or_else(|| format!("Sequence '{}' not found", seq_name))?;

            // If the referenced sequence hasn't been expanded yet (empty cues), we need to expand it first
            // This handles forward references - when seq_a references seq_b, we expand seq_b first
            if sequence.cues.is_empty() {
                // This shouldn't happen in the two-pass approach, but handle it gracefully
                return Err(format!(
                    "Sequence '{}' has not been expanded yet (internal error)",
                    seq_name
                )
                .into());
            }

            // Calculate sequence duration based on effect completion times
            // If sequence has only perpetual effects, use the last cue time as the end time
            let sequence_duration = match sequence.duration() {
                Some(duration) => duration,
                None => {
                    // Sequence has only perpetual effects - use last cue time as end time
                    if sequence.cues.is_empty() {
                        Duration::ZERO
                    } else {
                        sequence.cues.last().unwrap().time
                    }
                }
            };

            // Determine how many times to loop
            let loop_count = match loop_param {
                Some(SequenceLoop::Once) => 1,
                Some(SequenceLoop::Loop) => 10000, // Practical infinity
                Some(SequenceLoop::PingPong) => {
                    return Err("PingPong loop mode not yet implemented for sequences".into());
                }
                Some(SequenceLoop::Random) => {
                    return Err("Random loop mode not yet implemented for sequences".into());
                }
                Some(SequenceLoop::Count(n)) => n,
                None => 1, // Default to once if not specified
            };

            // Add to recursion stack
            recursion_stack.push(seq_name.clone());

            // Expand the sequence the specified number of times
            for iteration in 0..loop_count {
                let iteration_offset = unexpanded.time + (sequence_duration * iteration as u32);

                for seq_cue in &sequence.cues {
                    let mut expanded_cue = seq_cue.clone();
                    expanded_cue.time = iteration_offset + seq_cue.time;
                    // Mark all effects in this cue as belonging to the referenced sequence
                    for effect in &mut expanded_cue.effects {
                        if effect.sequence_name.is_none() {
                            effect.sequence_name = Some(seq_name.clone());
                        }
                    }
                    expanded_cues.push(expanded_cue);
                }
            }

            // Remove from recursion stack
            recursion_stack.pop();
        }

        // Add effects and layer commands to the first expanded cue
        if !unexpanded.effects.is_empty() || !unexpanded.layer_commands.is_empty() {
            if expanded_cues.is_empty() {
                expanded_cues.push(Cue {
                    time: unexpanded.time,
                    effects: unexpanded.effects,
                    layer_commands: unexpanded.layer_commands,
                    stop_sequences: unexpanded.stop_sequences,
                });
            } else {
                expanded_cues[0].effects.extend(unexpanded.effects);
                expanded_cues[0]
                    .layer_commands
                    .extend(unexpanded.layer_commands);
                expanded_cues[0]
                    .stop_sequences
                    .extend(unexpanded.stop_sequences);
            }
        } else if !unexpanded.stop_sequences.is_empty() && !expanded_cues.is_empty() {
            expanded_cues[0]
                .stop_sequences
                .extend(unexpanded.stop_sequences);
        }

        return Ok(expanded_cues);
    }

    // No sequence references - return a single cue
    Ok(vec![Cue {
        time: unexpanded.time,
        effects: unexpanded.effects,
        layer_commands: unexpanded.layer_commands,
        stop_sequences: unexpanded.stop_sequences,
    }])
}

fn parse_cue_definition(
    pair: Pair<Rule>,
    tempo_map: &Option<TempoMap>,
    sequences: &HashMap<String, Sequence>,
    measure_offset: u32,
) -> Result<(Vec<Cue>, Option<u32>), Box<dyn Error>> {
    let mut time = Duration::ZERO;
    let mut effects = Vec::new();
    let mut layer_commands = Vec::new();
    let mut stop_sequences = Vec::new();
    let mut effect_pairs = Vec::new();
    let mut layer_command_pairs = Vec::new();
    let mut sequence_references = Vec::new();
    let mut stop_sequence_pairs = Vec::new();
    let mut offset_commands = Vec::new();
    let mut reset_commands = Vec::new();
    let mut new_offset: Option<u32> = None;

    // First pass: parse time and collect effect/layer_command/sequence_reference/stop_sequence pairs
    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::time_string => {
                time = parse_time_string(inner_pair.as_str())?;
            }
            Rule::measure_time => {
                let (measure, beat) = parse_measure_time(inner_pair.as_str())?;
                // Apply offset to measure number
                let playback_measure = measure + measure_offset;
                if let Some(tm) = tempo_map {
                    // Pass measure_offset so tempo changes respect offsets
                    time = tm
                        .measure_to_time_with_offset(measure, beat, measure_offset)
                        .ok_or_else(|| {
                            format!(
                                "Invalid measure/beat position: {}/{}",
                                playback_measure, beat
                            )
                        })?;
                } else {
                    return Err("Measure-based timing requires a tempo section".into());
                }
            }
            Rule::offset_command => {
                offset_commands.push(inner_pair);
            }
            Rule::reset_measures_command => {
                reset_commands.push(inner_pair);
            }
            Rule::effect => {
                effect_pairs.push(inner_pair);
            }
            Rule::layer_command => {
                layer_command_pairs.push(inner_pair);
            }
            Rule::sequence_reference => {
                sequence_references.push(inner_pair);
            }
            Rule::stop_sequence_command => {
                stop_sequence_pairs.push(inner_pair);
            }
            _ => {
                // Skip unexpected rules
            }
        }
    }

    // Process offset commands
    // If reset is present, start from 0, then add any offset commands
    let base_offset = if !reset_commands.is_empty() {
        0
    } else {
        measure_offset
    };

    if !offset_commands.is_empty() {
        // Parse the offset value from the last offset command
        let mut total_offset = base_offset;
        for offset_pair in offset_commands {
            let offset_value = parse_offset_command(offset_pair)?;
            total_offset += offset_value;
        }
        new_offset = Some(total_offset);
    } else if !reset_commands.is_empty() {
        // Reset without offset - just set to 0
        new_offset = Some(0);
    }

    // Parse stop sequence commands
    for stop_pair in stop_sequence_pairs {
        let seq_name = stop_pair
            .into_inner()
            .find(|p| p.as_rule() == Rule::sequence_name)
            .ok_or("Stop sequence command missing sequence name")?
            .as_str()
            .trim_matches('"')
            .to_string();
        stop_sequences.push(seq_name);
    }

    // If there are sequence references, expand them into multiple cues
    if !sequence_references.is_empty() {
        let mut expanded_cues = Vec::new();

        // Parse effects and layer commands for the base cue
        for effect_pair in effect_pairs {
            let effect = parse_effect_definition(effect_pair, tempo_map, time)?;
            effects.push(effect);
        }

        for layer_command_pair in layer_command_pairs {
            let layer_command = parse_layer_command(layer_command_pair, tempo_map, time)?;
            layer_commands.push(layer_command);
        }

        // Expand each sequence reference
        for seq_ref_pair in sequence_references {
            let mut seq_name = String::new();
            let mut loop_param: Option<SequenceLoop> = None;

            // Parse sequence name and parameters
            for inner in seq_ref_pair.into_inner() {
                match inner.as_rule() {
                    Rule::sequence_name => {
                        seq_name = inner.as_str().trim_matches('"').to_string();
                    }
                    Rule::sequence_params => {
                        for param_pair in inner.into_inner() {
                            if param_pair.as_rule() == Rule::sequence_param {
                                let mut param_name = String::new();
                                let mut param_value = String::new();

                                for param_inner in param_pair.into_inner() {
                                    match param_inner.as_rule() {
                                        Rule::sequence_param_name => {
                                            param_name = param_inner.as_str().to_string();
                                        }
                                        Rule::sequence_param_value => {
                                            param_value = param_inner.as_str().to_string();
                                        }
                                        _ => {}
                                    }
                                }

                                if param_name == "loop" {
                                    loop_param = Some(parse_sequence_loop_param(&param_value)?);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            let sequence = sequences
                .get(&seq_name)
                .ok_or_else(|| format!("Sequence '{}' not found", seq_name))?;

            // Calculate sequence duration based on effect completion times
            // If sequence has only perpetual effects, use the last cue time as the end time
            let sequence_duration = match sequence.duration() {
                Some(duration) => duration,
                None => {
                    // Sequence has only perpetual effects - use last cue time as end time
                    if sequence.cues.is_empty() {
                        Duration::ZERO
                    } else {
                        sequence.cues.last().unwrap().time
                    }
                }
            };

            // Determine how many times to loop
            let loop_count = match loop_param {
                Some(SequenceLoop::Once) => 1,
                Some(SequenceLoop::Loop) => 10000, // Practical infinity
                Some(SequenceLoop::PingPong) => {
                    return Err("PingPong loop mode not yet implemented for sequences".into());
                }
                Some(SequenceLoop::Random) => {
                    return Err("Random loop mode not yet implemented for sequences".into());
                }
                Some(SequenceLoop::Count(n)) => n,
                None => 1, // Default to once if not specified
            };

            // Expand the sequence the specified number of times
            for iteration in 0..loop_count {
                let iteration_offset = time + (sequence_duration * iteration as u32);

                for seq_cue in &sequence.cues {
                    let mut expanded_cue = seq_cue.clone();
                    expanded_cue.time = iteration_offset + seq_cue.time;
                    // Mark all effects in this cue as belonging to this sequence
                    for effect in &mut expanded_cue.effects {
                        effect.sequence_name = Some(seq_name.clone());
                    }
                    expanded_cues.push(expanded_cue);
                }
            }
        }

        // If there are effects or layer commands, add them to the first expanded cue
        // or create a base cue if no sequences were expanded
        if !effects.is_empty() || !layer_commands.is_empty() {
            if expanded_cues.is_empty() {
                expanded_cues.push(Cue {
                    time,
                    effects,
                    layer_commands,
                    stop_sequences: stop_sequences.clone(),
                });
            } else {
                // Add effects and layer commands to the first expanded cue
                expanded_cues[0].effects.extend(effects);
                expanded_cues[0].layer_commands.extend(layer_commands);
                // Add stop sequences to the first expanded cue
                expanded_cues[0]
                    .stop_sequences
                    .extend(stop_sequences.clone());
            }
        } else if !stop_sequences.is_empty() {
            // No sequences expanded but we have stop commands - create a cue for them
            if expanded_cues.is_empty() {
                expanded_cues.push(Cue {
                    time,
                    effects: Vec::new(),
                    layer_commands: Vec::new(),
                    stop_sequences,
                });
            } else {
                expanded_cues[0].stop_sequences.extend(stop_sequences);
            }
        }

        return Ok((expanded_cues, new_offset));
    }

    // No sequence references - return a single cue
    // Second pass: parse effects now that we know the cue time
    for effect_pair in effect_pairs {
        let effect = parse_effect_definition(effect_pair, tempo_map, time)?;
        effects.push(effect);
    }

    // Parse layer commands
    for layer_command_pair in layer_command_pairs {
        let layer_command = parse_layer_command(layer_command_pair, tempo_map, time)?;
        layer_commands.push(layer_command);
    }

    Ok((
        vec![Cue {
            time,
            effects,
            layer_commands,
            stop_sequences,
        }],
        new_offset,
    ))
}

fn parse_layer_command(
    pair: Pair<Rule>,
    tempo_map: &Option<TempoMap>,
    cue_time: Duration,
) -> Result<LayerCommand, Box<dyn Error>> {
    let mut command_type = LayerCommandType::Clear;
    let mut layer = EffectLayer::Background;
    let mut fade_time = None;
    let mut intensity = None;
    let mut speed = None;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::layer_command_type => {
                command_type = match inner_pair.as_str() {
                    "clear" => LayerCommandType::Clear,
                    "release" => LayerCommandType::Release,
                    "freeze" => LayerCommandType::Freeze,
                    "unfreeze" => LayerCommandType::Unfreeze,
                    "master" => LayerCommandType::Master,
                    other => return Err(format!("Unknown layer command type: {}", other).into()),
                };
            }
            Rule::layer_command_params => {
                for param_pair in inner_pair.into_inner() {
                    if param_pair.as_rule() == Rule::layer_command_param {
                        let mut param_name = String::new();
                        let mut param_value = String::new();

                        for param_inner in param_pair.into_inner() {
                            match param_inner.as_rule() {
                                Rule::layer_command_param_name => {
                                    param_name = param_inner.as_str().to_string();
                                }
                                Rule::layer_command_param_value => {
                                    param_value = param_inner.as_str().to_string();
                                }
                                _ => {}
                            }
                        }

                        match param_name.as_str() {
                            "layer" => {
                                layer = match param_value.as_str() {
                                    "background" => EffectLayer::Background,
                                    "midground" => EffectLayer::Midground,
                                    "foreground" => EffectLayer::Foreground,
                                    other => return Err(format!("Invalid layer: {}", other).into()),
                                };
                            }
                            "time" => {
                                fade_time = Some(super::utils::parse_duration_string(
                                    &param_value,
                                    tempo_map,
                                    Some(cue_time),
                                )?);
                            }
                            "intensity" => {
                                // Parse percentage (e.g., "50%") or number (e.g., "0.5")
                                let value = if param_value.ends_with('%') {
                                    let percent_str = param_value.trim_end_matches('%');
                                    percent_str.parse::<f64>()? / 100.0
                                } else {
                                    param_value.parse::<f64>()?
                                };
                                intensity = Some(value.clamp(0.0, 1.0));
                            }
                            "speed" => {
                                // Parse percentage (e.g., "50%") or number (e.g., "0.5")
                                let value = if param_value.ends_with('%') {
                                    let percent_str = param_value.trim_end_matches('%');
                                    percent_str.parse::<f64>()? / 100.0
                                } else {
                                    param_value.parse::<f64>()?
                                };
                                speed = Some(value.max(0.0));
                            }
                            other => {
                                return Err(
                                    format!("Unknown layer command parameter: {}", other).into()
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(LayerCommand {
        command_type,
        layer,
        fade_time,
        intensity,
        speed,
    })
}
