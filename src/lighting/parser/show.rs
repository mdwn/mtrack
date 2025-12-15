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

/// Return type for `parse_sequence_cue_structure`: (cue, new_offset_secs, new_measure_offset, new_abs_time)
type ParseSequenceCueResult = Result<
    (
        UnexpandedSequenceCue,
        Option<f64>,
        Option<u32>,
        Option<Duration>,
    ),
    Box<dyn Error>,
>;

/// Return type for `parse_cue_definition`: (cues, new_offset_secs, new_measure_offset, new_abs_time)
type ParseCueResult =
    Result<(Vec<Cue>, Option<f64>, Option<u32>, Option<Duration>), Box<dyn Error>>;

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

                // Track cumulative offset in seconds (applies to all subsequent cues)
                let mut offset_secs: f64 = 0.0;
                // Track cumulative measure offset (applies to all subsequent cues)
                let mut cumulative_measure_offset: u32 = 0;
                // Track last absolute cue time to anchor standalone offsets
                let mut last_abs_time: Option<Duration> = None;

                // Then parse cues (now we have tempo_map and sequences)
                for cue_pair in cue_pairs {
                    let (parsed_cues, offset_change, measure_offset_change, last_time_change) =
                        parse_cue_definition(
                            cue_pair,
                            &effective_tempo.cloned(),
                            sequences,
                            offset_secs,
                            cumulative_measure_offset,
                            last_abs_time,
                        )?;
                    cues.extend(parsed_cues);
                    // Update offset for subsequent cues
                    if let Some(change) = offset_change {
                        offset_secs = change;
                    }
                    // Update cumulative measure offset for subsequent cues
                    if let Some(change) = measure_offset_change {
                        cumulative_measure_offset = change;
                    }
                    // Update last absolute time
                    if let Some(last_time) = last_time_change {
                        last_abs_time = Some(last_time);
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

                // Track accumulated offset seconds, measure offset, and last absolute time
                let mut offset_secs: f64 = 0.0;
                let mut cumulative_measure_offset: u32 = 0;
                let mut last_abs_time: Option<Duration> = None;

                for cue_pair in cue_pairs {
                    let (unexpanded_cue, offset_change, measure_offset_change, last_time_change) =
                        parse_sequence_cue_structure(
                            cue_pair,
                            &effective_tempo.cloned(),
                            offset_secs,
                            cumulative_measure_offset,
                            last_abs_time,
                        )?;
                    unexpanded_cues.push(unexpanded_cue);
                    // Update offset for subsequent cues
                    if let Some(change) = offset_change {
                        offset_secs = change;
                    }
                    // Update cumulative measure offset for subsequent cues
                    if let Some(change) = measure_offset_change {
                        cumulative_measure_offset = change;
                    }
                    if let Some(change) = last_time_change {
                        last_abs_time = Some(change);
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
    offset_secs: f64,
    cumulative_measure_offset: u32,
    last_abs_time: Option<Duration>,
) -> ParseSequenceCueResult {
    let mut score_time = Duration::ZERO; // score-space time
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
    let mut measure_time_pair: Option<Pair<Rule>> = None;
    let mut new_offset: Option<f64> = None;

    // First pass: collect all pairs (don't parse measure_time yet, as we need to process offsets first)
    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::time_string => {
                score_time = parse_time_string(inner_pair.as_str())?;
            }
            Rule::measure_time => {
                // Store the measure_time pair to parse after we know the effective offset
                measure_time_pair = Some(inner_pair);
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

    // Extract score measure early (before measure_time_pair is moved)
    let score_measure = if let Some(ref measure_pair) = measure_time_pair {
        let (measure, _) = parse_measure_time(measure_pair.as_str())?;
        Some(measure)
    } else {
        None
    };

    // Calculate unshifted score_time first for use as score_anchor
    let mut unshifted_score_time = Duration::ZERO;
    if let Some(measure_pair) = measure_time_pair.clone() {
        let (measure, beat) = parse_measure_time(measure_pair.as_str())?;
        if let Some(tm) = tempo_map {
            unshifted_score_time = tm
                .measure_to_time_with_offset(measure, beat, 0, 0.0)
                .ok_or_else(|| format!("Invalid measure/beat position: {}/{}", measure, beat))?;
        }
    }

    // Resolve measure_time to score-space time
    // Note: We pass offset_secs here so that tempo changes are shifted by offsets
    // This ensures that when offsets are applied, tempo changes happen at the correct shifted times
    if let Some(measure_pair) = measure_time_pair {
        let (measure, beat) = parse_measure_time(measure_pair.as_str())?;
        if let Some(tm) = tempo_map {
            score_time = tm
                // Pass offset_secs so tempo changes are shifted; measure_offset stays 0 for score-space
                .measure_to_time_with_offset(measure, beat, 0, offset_secs)
                .ok_or_else(|| format!("Invalid measure/beat position: {}/{}", measure, beat))?;
        } else {
            return Err("Measure-based timing requires a tempo section".into());
        }
    }

    // Resolve anchor for offset conversion in SCORE time (not shifted by previous offsets)
    // score_anchor should be the unshifted score time of the CURRENT cue (where the offset is issued),
    // not the last cue, so that the offset uses the tempo that applies at the point where it's issued
    let applied_offset_secs = offset_secs;
    let score_anchor = if unshifted_score_time != Duration::ZERO {
        // Use current cue's unshifted time so offset uses the tempo at the point where it's issued
        unshifted_score_time
    } else if score_time != Duration::ZERO {
        score_time
    } else {
        // Fallback to last cue's time if current cue has no measure_time
        // Convert absolute time to score time (from start_offset)
        last_abs_time
            .map(|t| {
                let abs_time = t.saturating_sub(Duration::from_secs_f64(applied_offset_secs));
                if let Some(tm) = tempo_map {
                    // Convert absolute time to score time by subtracting start_offset
                    abs_time.saturating_sub(tm.start_offset)
                } else {
                    abs_time
                }
            })
            .unwrap_or(Duration::ZERO)
    };

    // Track cumulative measure offset for playback measure calculations
    // Start with the passed-in cumulative offset from previous cues
    let mut cumulative_measure_offset = cumulative_measure_offset;

    // Compute next offset (applies to subsequent cues only)
    if !offset_commands.is_empty() || !reset_commands.is_empty() {
        let mut total_offset = if !reset_commands.is_empty() {
            cumulative_measure_offset = 0; // Reset measure offset on reset
            0.0
        } else {
            offset_secs
        };
        for offset_pair in &offset_commands {
            let offset_measures = parse_offset_command(offset_pair.clone())?;
            cumulative_measure_offset += offset_measures; // Track cumulative measure offset
            if let Some(tm) = tempo_map {
                // Calculate offset using the tempo at the anchor point
                // Offsets should be calculated at a single tempo (the tempo at the anchor),
                // not accounting for tempo changes during the offset period
                // Once a tempo has changed, offsets going forward shouldn't "undo" the tempo
                let bpm = tm.bpm_at_time(score_anchor, 0.0);
                let ts = tm.time_signature_at_time(score_anchor, 0.0);
                let seconds_per_beat = 60.0 / bpm;
                let delta = offset_measures as f64 * ts.beats_per_measure() * seconds_per_beat;
                total_offset += delta;
            } else {
                return Err("Offset command requires a tempo section".into());
            }
        }
        new_offset = Some(total_offset);
    }

    // Absolute time uses the currently applied offset (not the newly computed one)
    let abs_time = score_time + Duration::from_secs_f64(applied_offset_secs);
    let last_time = Some(abs_time);

    // Parse effects and layer commands
    let unshifted_for_effects = if unshifted_score_time != Duration::ZERO {
        Some(unshifted_score_time)
    } else {
        None
    };
    for effect_pair in effect_pairs {
        let effect = parse_effect_definition(
            effect_pair,
            tempo_map,
            abs_time,
            applied_offset_secs,
            unshifted_for_effects,
            score_measure,
            cumulative_measure_offset,
        )?;
        effects.push(effect);
    }

    for layer_command_pair in layer_command_pairs {
        let layer_command = parse_layer_command(layer_command_pair, tempo_map, abs_time)?;
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
            .trim()
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
                                        param_name = param_inner.as_str().trim().to_string();
                                    }
                                    Rule::sequence_param_value => {
                                        param_value = param_inner.as_str().trim().to_string();
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
            time: abs_time,
            effects,
            layer_commands,
            stop_sequences,
            sequence_references,
        },
        new_offset,
        Some(cumulative_measure_offset),
        last_time,
    ))
}

fn parse_sequence_loop_param(value: &str) -> Result<SequenceLoop, Box<dyn Error>> {
    // Be robust against trailing comments or extra tokens after the loop value.
    // Examples we want to support:
    //   loop: 3
    //   loop: 3   # comment
    //   loop: 3 // comment
    //
    // We first strip anything after a comment marker on the same line,
    // then take only the first whitespace-delimited token.
    let cleaned = value
        .split(|c| c == '#' || c == '/')
        .next()
        .unwrap_or("")
        .trim();
    let token = cleaned.split_whitespace().next().unwrap_or("");

    match token {
        "once" => Ok(SequenceLoop::Once),
        "loop" => Ok(SequenceLoop::Loop),
        "pingpong" => Ok(SequenceLoop::PingPong),
        "random" => Ok(SequenceLoop::Random),
        numeric if !numeric.is_empty() => {
            // Try to parse as a number
            let count: usize = numeric.parse().map_err(|_| {
                format!(
                    "Invalid loop count: '{}'. Expected 'once', 'loop', 'pingpong', 'random', or a number",
                    numeric
                )
            })?;
            if count == 0 {
                return Err("Loop count must be at least 1".into());
            }
            Ok(SequenceLoop::Count(count))
        }
        _ => Err(
            "Invalid loop parameter. Expected 'once', 'loop', 'pingpong', 'random', or a number"
                .into(),
        ),
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

            // Get the sequence's base time (first cue time, or ZERO if empty)
            // Sequence cue times are stored as absolute, but we need them relative to sequence start
            let sequence_base_time = sequence
                .cues
                .first()
                .map(|cue| cue.time)
                .unwrap_or(Duration::ZERO);

            // Calculate sequence duration based on effect completion times
            // If sequence has only perpetual effects, use the relative time from first to last cue
            let sequence_duration = match sequence.duration() {
                Some(completion_time) => {
                    // duration() returns absolute completion time, convert to relative
                    completion_time.saturating_sub(sequence_base_time)
                }
                None => {
                    // Sequence has only perpetual effects - use relative time from first to last cue
                    if sequence.cues.is_empty() {
                        Duration::ZERO
                    } else {
                        let last_time = sequence.cues.last().unwrap().time;
                        last_time.saturating_sub(sequence_base_time)
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
                    // Convert absolute sequence cue time to relative, then add to iteration offset
                    let relative_time = seq_cue.time.saturating_sub(sequence_base_time);
                    expanded_cue.time = iteration_offset + relative_time;
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
    offset_secs: f64,
    cumulative_measure_offset: u32,
    last_abs_time: Option<Duration>,
) -> ParseCueResult {
    let mut score_time = Duration::ZERO;
    let mut effects = Vec::new();
    let mut layer_commands = Vec::new();
    let mut stop_sequences = Vec::new();
    let mut effect_pairs = Vec::new();
    let mut layer_command_pairs = Vec::new();
    let mut sequence_references = Vec::new();
    let mut stop_sequence_pairs = Vec::new();
    let mut offset_commands = Vec::new();
    let mut reset_commands = Vec::new();
    let mut measure_time_pair: Option<Pair<Rule>> = None;
    let mut new_offset: Option<f64> = None;

    // First pass: collect all pairs (don't parse measure_time yet, as we need to process offsets first)
    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::time_string => {
                score_time = parse_time_string(inner_pair.as_str())?;
            }
            Rule::measure_time => {
                // Store the measure_time pair to parse after we know the effective offset
                measure_time_pair = Some(inner_pair);
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

    // Extract score measure early (before measure_time_pair is moved)
    let score_measure_seq = if let Some(ref measure_pair) = measure_time_pair {
        let (measure, _) = parse_measure_time(measure_pair.as_str())?;
        Some(measure)
    } else {
        None
    };

    // Resolve measure_time to score_time
    // Note: We pass offset_secs here so that tempo changes are shifted by offsets
    // This ensures that when offsets are applied, tempo changes happen at the correct shifted times
    // Also calculate unshifted_score_time for use as score_anchor (before consuming measure_time_pair)
    let mut unshifted_score_time_seq = Duration::ZERO;
    if let Some(measure_pair) = measure_time_pair.as_ref() {
        let (measure, beat) = parse_measure_time(measure_pair.as_str())?;
        if let Some(tm) = tempo_map {
            // Calculate unshifted time first for anchor
            unshifted_score_time_seq = tm
                .measure_to_time_with_offset(measure, beat, 0, 0.0)
                .ok_or_else(|| format!("Invalid measure/beat position: {}/{}", measure, beat))?;
        }
    }

    if let Some(measure_pair) = measure_time_pair {
        let (measure, beat) = parse_measure_time(measure_pair.as_str())?;
        if let Some(tm) = tempo_map {
            score_time = tm
                // Pass offset_secs so tempo changes are shifted; measure_offset stays 0 for score-space
                .measure_to_time_with_offset(measure, beat, 0, offset_secs)
                .ok_or_else(|| format!("Invalid measure/beat position: {}/{}", measure, beat))?;
        } else {
            return Err("Measure-based timing requires a tempo section".into());
        }
    }

    // Anchor for offset conversion in SCORE time (not shifted by previous offsets)
    // score_anchor should be the unshifted score time of the CURRENT cue (where the offset is issued),
    // so that the offset uses the tempo that applies at the point where it's issued
    let applied_offset_secs = offset_secs;
    let score_anchor = if unshifted_score_time_seq != Duration::ZERO {
        unshifted_score_time_seq
    } else if score_time != Duration::ZERO {
        score_time
    } else {
        // Fallback to last cue's time if current cue has no measure_time
        // Convert absolute time to score time (from start_offset)
        last_abs_time
            .map(|t| {
                let abs_time = t.saturating_sub(Duration::from_secs_f64(applied_offset_secs));
                if let Some(tm) = tempo_map {
                    // Convert absolute time to score time by subtracting start_offset
                    abs_time.saturating_sub(tm.start_offset)
                } else {
                    abs_time
                }
            })
            .unwrap_or(Duration::ZERO)
    };

    // Track cumulative measure offset for playback measure calculations
    // Start with the passed-in cumulative offset from previous cues
    let mut cumulative_measure_offset_seq = cumulative_measure_offset;

    // Compute next offset (applies to subsequent cues only)
    if !offset_commands.is_empty() || !reset_commands.is_empty() {
        let mut total_offset = if !reset_commands.is_empty() {
            cumulative_measure_offset_seq = 0; // Reset measure offset on reset
            0.0
        } else {
            offset_secs
        };
        for offset_pair in &offset_commands {
            let offset_measures = parse_offset_command(offset_pair.clone())?;
            cumulative_measure_offset_seq += offset_measures; // Track cumulative measure offset
            if let Some(tm) = tempo_map {
                // Calculate offset using the tempo at the anchor point
                // Offsets should be calculated at a single tempo (the tempo at the anchor),
                // not accounting for tempo changes during the offset period
                // Once a tempo has changed, offsets going forward shouldn't "undo" the tempo
                let bpm = tm.bpm_at_time(score_anchor, 0.0);
                let ts = tm.time_signature_at_time(score_anchor, 0.0);
                let seconds_per_beat = 60.0 / bpm;
                let delta = offset_measures as f64 * ts.beats_per_measure() * seconds_per_beat;
                total_offset += delta;
            } else {
                return Err("Offset command requires a tempo section".into());
            }
        }
        new_offset = Some(total_offset);
    }

    // Absolute time is from start_offset (not absolute start), matching how score_time is calculated
    let abs_time = score_time + Duration::from_secs_f64(applied_offset_secs);
    let last_time = Some(abs_time);

    // Parse stop sequence commands
    for stop_pair in stop_sequence_pairs {
        let seq_name = stop_pair
            .into_inner()
            .find(|p| p.as_rule() == Rule::sequence_name)
            .ok_or("Stop sequence command missing sequence name")?
            .as_str()
            .trim_matches('"')
            .trim()
            .to_string();
        stop_sequences.push(seq_name);
    }

    // If there are sequence references, expand them into multiple cues
    if !sequence_references.is_empty() {
        let mut expanded_cues = Vec::new();

        // Parse effects and layer commands for the base cue
        for effect_pair in effect_pairs {
            let effect = parse_effect_definition(
                effect_pair,
                tempo_map,
                abs_time,
                applied_offset_secs,
                if unshifted_score_time_seq != Duration::ZERO {
                    Some(unshifted_score_time_seq)
                } else {
                    None
                },
                score_measure_seq,
                cumulative_measure_offset_seq,
            )?;
            effects.push(effect);
        }

        for layer_command_pair in layer_command_pairs {
            let layer_command = parse_layer_command(layer_command_pair, tempo_map, abs_time)?;
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
                                            param_name = param_inner.as_str().trim().to_string();
                                        }
                                        Rule::sequence_param_value => {
                                            param_value = param_inner.as_str().trim().to_string();
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

            // Get the sequence's base time (first cue time, or ZERO if empty)
            // Sequence cue times are stored as absolute, but we need them relative to sequence start
            let sequence_base_time = sequence
                .cues
                .first()
                .map(|cue| cue.time)
                .unwrap_or(Duration::ZERO);

            // Calculate sequence duration based on effect completion times
            // If sequence has only perpetual effects, use the relative time from first to last cue
            let sequence_duration = match sequence.duration() {
                Some(completion_time) => {
                    // duration() returns absolute completion time, convert to relative
                    completion_time.saturating_sub(sequence_base_time)
                }
                None => {
                    // Sequence has only perpetual effects - use relative time from first to last cue
                    if sequence.cues.is_empty() {
                        Duration::ZERO
                    } else {
                        let last_time = sequence.cues.last().unwrap().time;
                        last_time.saturating_sub(sequence_base_time)
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
                let iteration_offset = abs_time + (sequence_duration * iteration as u32);

                for seq_cue in &sequence.cues {
                    let mut expanded_cue = seq_cue.clone();
                    // Convert absolute sequence cue time to relative, then add to iteration offset
                    let relative_time = seq_cue.time.saturating_sub(sequence_base_time);
                    expanded_cue.time = iteration_offset + relative_time;
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
                    time: abs_time,
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
                    time: abs_time,
                    effects: Vec::new(),
                    layer_commands: Vec::new(),
                    stop_sequences,
                });
            } else {
                expanded_cues[0].stop_sequences.extend(stop_sequences);
            }
        }

        return Ok((
            expanded_cues,
            new_offset,
            Some(cumulative_measure_offset_seq),
            last_time,
        ));
    }

    // No sequence references - return a single cue
    // Second pass: parse effects now that we know the cue time
    for effect_pair in effect_pairs {
        let effect = parse_effect_definition(
            effect_pair,
            tempo_map,
            abs_time,
            applied_offset_secs,
            if unshifted_score_time_seq != Duration::ZERO {
                Some(unshifted_score_time_seq)
            } else {
                None
            },
            score_measure_seq,
            cumulative_measure_offset_seq,
        )?;
        effects.push(effect);
    }

    // Parse layer commands
    for layer_command_pair in layer_command_pairs {
        let layer_command = parse_layer_command(layer_command_pair, tempo_map, abs_time)?;
        layer_commands.push(layer_command);
    }

    Ok((
        vec![Cue {
            time: abs_time,
            effects,
            layer_commands,
            stop_sequences,
        }],
        new_offset,
        Some(cumulative_measure_offset_seq),
        last_time,
    ))
}

fn parse_layer_command(
    pair: Pair<Rule>,
    tempo_map: &Option<TempoMap>,
    cue_time: Duration,
) -> Result<LayerCommand, Box<dyn Error>> {
    let mut command_type = LayerCommandType::Clear;
    let mut layer: Option<EffectLayer> = None;
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
                                    param_name = param_inner.as_str().trim().to_string();
                                }
                                Rule::layer_command_param_value => {
                                    param_value = param_inner.as_str().trim().to_string();
                                }
                                _ => {}
                            }
                        }

                        match param_name.as_str() {
                            "layer" => {
                                layer = Some(match param_value.as_str() {
                                    "background" => EffectLayer::Background,
                                    "midground" => EffectLayer::Midground,
                                    "foreground" => EffectLayer::Foreground,
                                    other => return Err(format!("Invalid layer: {}", other).into()),
                                });
                            }
                            "time" => {
                                fade_time = Some(super::utils::parse_duration_string(
                                    &param_value,
                                    tempo_map,
                                    Some(cue_time),
                                    0.0, // Layer commands don't use offsets for duration parsing
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

    // Validate: clear can work without layer (clears all), but other commands require a layer
    if layer.is_none() && command_type != LayerCommandType::Clear {
        return Err(format!(
            "Layer command '{}' requires a layer parameter",
            match command_type {
                LayerCommandType::Clear => "clear",
                LayerCommandType::Release => "release",
                LayerCommandType::Freeze => "freeze",
                LayerCommandType::Unfreeze => "unfreeze",
                LayerCommandType::Master => "master",
            }
        )
        .into());
    }

    Ok(LayerCommand {
        command_type,
        layer,
        fade_time,
        intensity,
        speed,
    })
}
