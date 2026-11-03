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

use std::collections::HashSet;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::super::effects::EffectLayer;
use super::super::tempo::{snap_beats, CueClock, CuePosition, TempoMap};
use super::effect_parse::parse_effect_definition;
use super::error::{analyze_parsing_failure, describe_pest_error};
use super::grammar::{LightingParser, Rule};
use super::include::{add_included, Include, IncludingFile};
use super::sequences::{line_at, param_refs, SequenceLibrary};
use super::tempo_parse::parse_tempo_definition;
use super::types::{
    Cue, LayerCommand, LayerCommandType, LightShow, LightShows, ParseContext, Sequence,
};
use super::types::{MusicalLength, SequenceCall, SequenceLoop, UnexpandedSequenceCue};
use super::utils::{parse_duration_string, parse_measure_time, parse_time_string};
use pest::iterators::Pair;
use pest::Parser;

/// Parses light shows from DSL content.
pub fn parse_light_shows(content: &str) -> Result<LightShows, Box<dyn Error>> {
    parse_light_shows_with_tempo(content, None)
}

/// Parses light shows, supplying a tempo map for files that carry none.
///
/// Musical timing — `@bar/beat` cues, `Nbeats` durations, `Nbeat` frequencies —
/// is resolved at parse time, so a file without a `tempo` block cannot use any
/// of it. `external` is the song's tempo map: its `tempo:` block if it has one,
/// otherwise one derived from the click-derived beat grid. A `tempo` block in
/// the file still wins, keeping precedence author-first.
pub fn parse_light_shows_with_tempo(
    content: &str,
    external: Option<&TempoMap>,
) -> Result<LightShows, Box<dyn Error>> {
    parse_light_file(content, None, external)
}

/// Parses a `.light` file's shows, as `parse_light_shows_with_tempo` does.
/// `path` is where the content lives, which its `include`s resolve against;
/// content with an `include` and no path is an error.
pub fn parse_light_file(
    content: &str,
    path: Option<&Path>,
    external: Option<&TempoMap>,
) -> Result<LightShows, Box<dyn Error>> {
    parse_light_file_within(content, path, None, external)
}

/// Parses a `.light` file as `parse_light_file` does, its `include`s
/// confined to `root` when one is given: for source from somewhere that may
/// not read the whole disk, such as the web UI or an MCP client.
pub fn parse_light_file_within(
    content: &str,
    path: Option<&Path>,
    root: Option<&Path>,
    external: Option<&TempoMap>,
) -> Result<LightShows, Box<dyn Error>> {
    let root = match root {
        Some(root) => Some(
            root.canonicalize()
                .map_err(|e| format!("cannot read songs folder {}: {e}", root.display()))?,
        ),
        None => None,
    };
    let pairs =
        LightingParser::parse(Rule::file, content).map_err(|e| describe_pest_error(&e, content))?;

    let mut shows = LightShows::new();
    // Seeded with the song's map so a file with no tempo block inherits it; a
    // file-level `tempo { }` overwrites this below.
    let mut global_tempo: Option<TempoMap> = external.cloned();
    let mut show_pairs = Vec::new();
    let mut sequence_pairs = Vec::new();
    let mut includes = Vec::new();

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
            Rule::include => includes.push(Include::from_pair(pair)),
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
                        Rule::include => includes.push(Include::from_pair(inner_pair)),
                        _ => {}
                    }
                }
            }
        }
    }

    let mut library = SequenceLibrary::new(&global_tempo);
    let mut seen: HashSet<PathBuf> = path
        .and_then(|p| p.canonicalize().ok())
        .into_iter()
        .collect();
    add_included(
        &mut library,
        includes,
        IncludingFile {
            path,
            display: None,
            root: root.as_deref(),
        },
        &mut seen,
    )?;
    library.add(sequence_pairs, None)?;
    library.check_all()?;

    // Second pass: parse shows with tempo and sequences available
    let mut parsed_shows = Vec::new();
    for pair in show_pairs {
        if let Some((offset, name)) = param_refs(pair.as_str()).first() {
            return Err(format!(
                "line {}: ${name} is a sequence parameter, but this is a show, not a sequence \
                 that declares it",
                pair.line_col().0 + line_at(pair.as_str(), *offset) - 1
            )
            .into());
        }
        let mut show = parse_light_show_definition(pair, &global_tempo, &library)?;
        // If show doesn't have its own tempo, use global tempo
        if show.tempo_map.is_none() {
            show.tempo_map = global_tempo.clone();
        }
        parsed_shows.push(show);
    }

    // Enforce naming rules:
    // - If there's exactly one show and it has no name, synthesize a default name.
    // - If there are multiple shows, all must have explicit names.
    if parsed_shows.len() == 1 {
        let mut show = parsed_shows.remove(0);
        if show.name.trim().is_empty() {
            show.name = "default".to_string();
        }
        shows.insert(show.name.clone(), show);
    } else {
        for show in parsed_shows {
            if show.name.trim().is_empty() {
                return Err(
                    "Show name is required when multiple shows are defined in a file"
                        .to_string()
                        .into(),
                );
            }
            shows.insert(show.name.clone(), show);
        }
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
    library: &SequenceLibrary,
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

                // Each cue is placed under the offsets of the cues before it.
                let effective_tempo = effective_tempo.cloned();
                let mut clock = CueClock::new();
                for cue_pair in cue_pairs {
                    for unexpanded in parse_cue(cue_pair, &effective_tempo, &mut clock)? {
                        let expanded =
                            expand_unexpanded_sequence_cue(unexpanded, &effective_tempo, library)?;
                        // A sequence played from the cue reaches as far as it
                        // goes, for the next cue's offset anchor.
                        if let Some(last) = expanded.iter().map(|c| c.time).max() {
                            clock.reached(last);
                        }
                        cues.extend(expanded);
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

/// A sequence definition as written, before nested references are expanded.
pub(super) struct SequenceDefinition {
    name: String,
    pub(super) tempo_map: Option<TempoMap>,
    /// The `length:` value as written, e.g. `1measure`.
    length: Option<String>,
    pub(super) cues: Vec<UnexpandedSequenceCue>,
}

impl SequenceDefinition {
    /// The `length:` as written, if it declares one.
    pub(super) fn length_written(&self) -> Option<&str> {
        self.length.as_deref()
    }

    /// The declared `length:` in time from the first cue, or `None` when the
    /// sequence declares none. Every cue must start inside it: one starting
    /// at or past the end would land on the next pass's first cue.
    pub(super) fn declared_length(
        &self,
        tempo_map: Option<&TempoMap>,
    ) -> Result<Option<Duration>, Box<dyn Error>> {
        let Some(written) = &self.length else {
            return Ok(None);
        };
        let start = self.cues.iter().map(|c| c.time).min().unwrap_or_default();
        let length = parse_duration_string(written, &tempo_map.cloned(), Some(start), 0.0)
            .map_err(|e| {
                format!(
                    "Sequence '{}': invalid length '{}': {}",
                    self.name, written, e
                )
            })?;
        if length.is_zero() {
            return Err(
                format!("Sequence '{}': length must be greater than zero", self.name).into(),
            );
        }
        if let Some(late) = self.cues.iter().find(|c| c.time - start >= length) {
            return Err(format!(
                "Sequence '{}': a cue starts {:.3}s in, at or past its length of {} ({:.3}s)",
                self.name,
                (late.time - start).as_secs_f64(),
                written,
                length.as_secs_f64()
            )
            .into());
        }
        Ok(Some(length))
    }
}

pub(super) fn parse_sequence_structure(
    pair: Pair<Rule>,
    global_tempo: &Option<TempoMap>,
) -> Result<SequenceDefinition, Box<dyn Error>> {
    let mut name = String::new();
    let mut tempo_map: Option<TempoMap> = None;
    let mut length = None;
    let mut unexpanded_cues = Vec::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::sequence_name => {
                name = inner_pair.as_str().trim_matches('"').to_string();
            }
            Rule::sequence_length => {
                length = inner_pair
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::time_parameter)
                    .map(|p| p.as_str().trim().to_string());
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

                let effective_tempo = effective_tempo.cloned();
                let mut clock = CueClock::new();
                for cue_pair in cue_pairs {
                    unexpanded_cues.extend(parse_cue(cue_pair, &effective_tempo, &mut clock)?);
                }
            }
            _ => {}
        }
    }

    Ok(SequenceDefinition {
        name,
        tempo_map,
        length,
        cues: unexpanded_cues,
    })
}

/// Determine loop count from a SequenceLoop parameter
fn determine_loop_count(loop_param: Option<SequenceLoop>) -> Result<usize, Box<dyn Error>> {
    match loop_param {
        Some(SequenceLoop::Once) => Ok(1),
        Some(SequenceLoop::Loop) => Ok(10000), // Practical infinity
        Some(SequenceLoop::PingPong) => {
            Err("PingPong loop mode not yet implemented for sequences".into())
        }
        Some(SequenceLoop::Random) => {
            Err("Random loop mode not yet implemented for sequences".into())
        }
        Some(SequenceLoop::Count(n)) => Ok(n),
        None => Ok(1), // Default to once if not specified
    }
}

/// Parse sequence reference from a Pair
fn parse_sequence_reference_pair(seq_ref_pair: Pair<Rule>) -> Result<SequenceCall, Box<dyn Error>> {
    let line = seq_ref_pair.line_col().0;
    let mut seq_name = String::new();
    let mut args = Vec::new();
    let mut loop_param: Option<SequenceLoop> = None;

    for inner in seq_ref_pair.into_inner() {
        match inner.as_rule() {
            Rule::sequence_name => {
                seq_name = inner.as_str().trim_matches('"').to_string();
            }
            Rule::sequence_args => {
                for arg in inner.into_inner() {
                    let mut parts = arg.into_inner();
                    if let (Some(param), Some(value)) = (parts.next(), parts.next()) {
                        args.push((param.as_str().to_string(), value.as_str().to_string()));
                    }
                }
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

    Ok(SequenceCall {
        name: seq_name,
        args,
        loop_param,
        line,
    })
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
        .split(|c| ['#', '/'].contains(&c))
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

/// Parses one cue -- in a show, a sequence or an inline loop -- placing it
/// with `clock`. Its inline loops are expanded into cues of their own (merged
/// into the cue where they fall on it); its sequence references are kept for
/// [`expand_unexpanded_sequence_cue`] to expand, which a show does at once and
/// a sequence when it is used.
fn parse_cue(
    pair: Pair<Rule>,
    tempo_map: &Option<TempoMap>,
    clock: &mut CueClock,
) -> Result<Vec<UnexpandedSequenceCue>, Box<dyn Error>> {
    let mut position = CuePosition::Time(Duration::ZERO);
    let mut effect_pairs = Vec::new();
    let mut layer_command_pairs = Vec::new();
    let mut sequence_ref_pairs = Vec::new();
    let mut stop_sequence_pairs = Vec::new();
    let mut offset_commands = Vec::new();
    let mut reset = false;
    let mut inline_loop_pairs = Vec::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::time_string => {
                position = CuePosition::Time(parse_time_string(inner_pair.as_str())?);
            }
            Rule::measure_time => {
                let (measure, beat) = parse_measure_time(inner_pair.as_str())?;
                position = CuePosition::MeasureBeat(measure, beat);
            }
            Rule::offset_command => offset_commands.push(inner_pair),
            Rule::reset_measures_command => reset = true,
            Rule::effect => effect_pairs.push(inner_pair),
            Rule::layer_command => layer_command_pairs.push(inner_pair),
            Rule::sequence_reference => sequence_ref_pairs.push(inner_pair),
            Rule::stop_sequence_command => stop_sequence_pairs.push(inner_pair),
            Rule::inline_loop => inline_loop_pairs.push(inner_pair),
            _ => {}
        }
    }

    let mut offset_measures = 0;
    for offset_pair in &offset_commands {
        offset_measures += parse_offset_command(offset_pair.clone())?;
    }
    let placed = clock.place(
        tempo_map.as_ref(),
        position,
        offset_measures,
        !offset_commands.is_empty(),
        reset,
    )?;
    let time = placed.time;

    let effect_ctx = ParseContext {
        tempo_map: tempo_map.clone(),
        cue_time: time,
        offset_secs: placed.offset_secs,
        unshifted_score_time: placed.unshifted.filter(|t| !t.is_zero()),
    };
    let effects = effect_pairs
        .into_iter()
        .map(|p| parse_effect_definition(p, &effect_ctx))
        .collect::<Result<Vec<_>, _>>()?;
    let layer_commands = layer_command_pairs
        .into_iter()
        .map(parse_layer_command)
        .collect::<Result<Vec<_>, _>>()?;
    let mut stop_sequences = Vec::new();
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
    let sequence_references = sequence_ref_pairs
        .into_iter()
        .map(parse_sequence_reference_pair)
        .collect::<Result<Vec<_>, _>>()?;

    let mut loops = Vec::new();
    for inline_loop_pair in inline_loop_pairs.iter().cloned() {
        loops.push(parse_inline_loop(
            inline_loop_pair,
            tempo_map,
            time,
            placed.offset_secs,
        )?);
    }
    let musical = matches!(position, CuePosition::MeasureBeat(..))
        || !offset_commands.is_empty()
        || effects.iter().any(|e| !e.musical.is_empty())
        || loops.iter().any(|(_, musical)| *musical);

    let has_base_content = !effects.is_empty()
        || !layer_commands.is_empty()
        || !stop_sequences.is_empty()
        || !sequence_references.is_empty();
    let mut cues = Vec::new();
    if has_base_content || inline_loop_pairs.is_empty() {
        cues.push(UnexpandedSequenceCue {
            time,
            offset_secs: placed.offset_secs,
            effects,
            layer_commands,
            stop_sequences,
            sequence_references,
            musical,
        });
    }
    let has_base = !cues.is_empty();

    for (loop_cues, _) in loops {
        for loop_cue in loop_cues {
            // A loop cue on the cue itself joins it.
            if has_base && cues[0].time == loop_cue.time {
                cues[0].effects.extend(loop_cue.effects);
                cues[0].layer_commands.extend(loop_cue.layer_commands);
                cues[0].stop_sequences.extend(loop_cue.stop_sequences);
                continue;
            }
            cues.push(UnexpandedSequenceCue {
                time: loop_cue.time,
                offset_secs: placed.offset_secs,
                effects: loop_cue.effects,
                layer_commands: loop_cue.layer_commands,
                stop_sequences: loop_cue.stop_sequences,
                sequence_references: Vec::new(),
                musical,
            });
        }
    }

    // A loop's cues come in source order, which need not be time order
    // (`@0.5` written before `@0.0`).
    cues.sort_by_key(|c| c.time);
    if let Some(last) = cues.iter().map(|c| c.time).max() {
        clock.reached(last);
    }
    Ok(cues)
}

/// The cues an unexpanded cue becomes: itself, with every sequence it
/// references played from it (a cue of a sequence's that falls on it joins
/// it).
pub(super) fn expand_unexpanded_sequence_cue(
    unexpanded: UnexpandedSequenceCue,
    tempo_map: &Option<TempoMap>,
    library: &SequenceLibrary,
) -> Result<Vec<Cue>, Box<dyn Error>> {
    let has_base_content = !unexpanded.effects.is_empty()
        || !unexpanded.layer_commands.is_empty()
        || !unexpanded.stop_sequences.is_empty();
    let mut cues = Vec::new();
    if has_base_content || unexpanded.sequence_references.is_empty() {
        cues.push(Cue {
            time: unexpanded.time,
            effects: unexpanded.effects,
            layer_commands: unexpanded.layer_commands,
            stop_sequences: unexpanded.stop_sequences,
            start_sequences: vec![],
        });
    }
    let has_base = !cues.is_empty();

    for call in unexpanded.sequence_references {
        let sequence = library.instance(&call)?;
        let passes = determine_loop_count(call.loop_param)?;
        for cue in expand_sequence_call(
            &sequence,
            unexpanded.time,
            unexpanded.offset_secs,
            tempo_map,
            Some(&call.name),
            passes,
        ) {
            if has_base && cues[0].time == cue.time {
                let base = &mut cues[0];
                base.effects.extend(cue.effects);
                base.layer_commands.extend(cue.layer_commands);
                base.stop_sequences.extend(cue.stop_sequences);
                base.start_sequences.extend(cue.start_sequences);
                continue;
            }
            cues.push(cue);
        }
    }

    cues.sort_by_key(|c| c.time);
    Ok(cues)
}

/// Plays `passes` passes of `sequence` from `at`, under an offset of
/// `offset_secs`: a sequence referenced from a cue (`name` given, so its
/// effects can be stopped by it) or an inline loop's body (no name).
///
/// A sequence's cues and its pass length are counted in beats, through its
/// own tempo map, and placed through the tempo map where it is played: from
/// the beat at `at`, so a pass that crosses a tempo change (or a ramp)
/// follows it, as the cues around it do. Its effects' durations in beats or
/// measures are resolved where they play (#529). Without a tempo map where it
/// is played, or `in_seconds` (an inline loop's body written wholly in
/// seconds), it keeps the times it has: its cues and its passes stay in
/// seconds whatever the tempo does.
fn expand_sequence_call(
    sequence: &Sequence,
    at: Duration,
    offset_secs: f64,
    tempo_map: &Option<TempoMap>,
    name: Option<&str>,
    passes: usize,
) -> Vec<Cue> {
    let in_seconds = sequence.in_seconds;
    let base = sequence.start_time();
    let pass_beats = sequence.pass_beats();
    let pass_length = sequence.pass_length();
    let cue_beats: Vec<f64> = sequence
        .cues
        .iter()
        .map(|cue| sequence.beats_from_start(cue.time))
        .collect();

    let mut cues = Vec::with_capacity(sequence.cues.len() * passes);
    for pass in 0..passes {
        for (index, (seq_cue, beats)) in sequence.cues.iter().zip(&cue_beats).enumerate() {
            let mut cue = seq_cue.clone();
            cue.time = match tempo_map.as_ref().filter(|_| !in_seconds) {
                Some(tm) => {
                    // Snapped: a pass length's rounding error multiplied by
                    // the pass count crept the loop later every pass (#534).
                    let beats = snap_beats(pass_beats * pass as f64 + beats);
                    at + tm.beats_to_duration(beats, at, offset_secs)
                }
                None => at + pass_length * pass as u32 + seq_cue.time.saturating_sub(base),
            };
            if let Some(name) = name {
                // Each pass after the first stops the one before, so its
                // effects (a multiply chase, say) do not pile up; the first
                // clears any earlier stop of the sequence, so it plays again.
                if index == 0 && pass > 0 {
                    cue.stop_sequences.push(name.to_string());
                }
                if index == 0 && pass == 0 {
                    cue.start_sequences.push(name.to_string());
                }
            }
            for effect in &mut cue.effects {
                if let Some(name) = name {
                    effect.sequence_name = Some(name.to_string());
                }
                // Tempo-aware rates read the map as shifted where it plays.
                effect.tempo_offset_secs = offset_secs;
                if let Some(tm) = tempo_map {
                    effect.resolve_musical_durations(tm, cue.time, offset_secs);
                }
            }
            cues.push(cue);
        }
    }
    cues
}

/// Whether a body -- a sequence's, or an inline loop's -- is purely in
/// seconds: no cue at a bar and beat or issuing an `offset` (in measures),
/// no effect lasting beats or measures,
/// no inline loop or sequence played from it that is not purely in seconds
/// itself, and no `length:` in beats or measures. Such a body is laid out
/// in seconds wherever it plays; any other in beats.
pub(super) fn body_in_seconds(
    cues: &[UnexpandedSequenceCue],
    length: Option<&str>,
    library: &SequenceLibrary,
) -> Result<bool, Box<dyn Error>> {
    if length.and_then(MusicalLength::parse).is_some() || cues.iter().any(|c| c.musical) {
        return Ok(false);
    }
    for call in cues.iter().flat_map(|c| &c.sequence_references) {
        if !library.instance(call)?.in_seconds {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Parses an inline loop at `at` (under an offset of `offset_secs`) and plays
/// it: its body is an unnamed sequence, `repeats` passes of it.
fn parse_inline_loop(
    pair: Pair<Rule>,
    tempo_map: &Option<TempoMap>,
    at: Duration,
    offset_secs: f64,
) -> Result<(Vec<Cue>, bool), Box<dyn Error>> {
    let mut loop_cue_pairs = Vec::new();
    let mut repeats = 1;
    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::inline_loop_cue => loop_cue_pairs.push(inner_pair),
            Rule::number_value => {
                repeats = inner_pair
                    .as_str()
                    .trim()
                    .parse::<usize>()
                    .map_err(|e| format!("Invalid repeats count: {}", e))?;
                if repeats == 0 {
                    return Err("Loop repeats must be at least 1".into());
                }
            }
            _ => {}
        }
    }

    // The body is timed from its own start, with offsets of its own, at
    // the one tempo and time signature in effect where the loop is: as a
    // named sequence is timed at one tempo, so a body in seconds keeps its
    // seconds there. A body purely in seconds keeps them on every pass;
    // any other is laid out in beats from the loop, following any tempo
    // change its passes reach.
    let body_tempo = tempo_map.as_ref().map(|tm| tm.steady_at(at, offset_secs));
    let mut clock = CueClock::new();
    let mut parsed = Vec::new();
    for loop_cue_pair in loop_cue_pairs {
        parsed.extend(parse_cue(loop_cue_pair, &body_tempo, &mut clock)?);
    }
    // A loop's body references no sequence, so no library is needed.
    let in_seconds = body_in_seconds(&parsed, None, &SequenceLibrary::empty())?;
    let mut body: Vec<Cue> = parsed
        .into_iter()
        .map(|cue| Cue {
            time: cue.time,
            effects: cue.effects,
            layer_commands: cue.layer_commands,
            stop_sequences: cue.stop_sequences,
            start_sequences: vec![],
        })
        .collect();
    if body.is_empty() {
        return Ok((Vec::new(), !in_seconds));
    }
    body.sort_by_key(|c| c.time);
    let sequence = Sequence {
        cues: body,
        tempo_map: body_tempo,
        length: None,
        in_seconds,
    };
    Ok((
        expand_sequence_call(&sequence, at, offset_secs, tempo_map, None, repeats),
        !in_seconds,
    ))
}

fn parse_layer_command(pair: Pair<Rule>) -> Result<LayerCommand, Box<dyn Error>> {
    let mut command_type = LayerCommandType::Clear;
    let mut layer: Option<EffectLayer> = None;
    let mut intensity = None;
    let mut speed = None;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::layer_command_type => {
                command_type = match inner_pair.as_str() {
                    "clear" => LayerCommandType::Clear,
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
        intensity,
        speed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Sequence pass lengths ────────────────────────────────────

    fn make_cue(time: Duration) -> Cue {
        Cue {
            time,
            effects: vec![],
            layer_commands: vec![],
            stop_sequences: vec![],
            start_sequences: vec![],
        }
    }

    fn make_sequence(cues: Vec<Cue>) -> Sequence {
        Sequence {
            cues,
            tempo_map: None,
            length: None,
            in_seconds: false,
        }
    }

    #[test]
    fn sequence_start_time_empty() {
        let seq = make_sequence(vec![]);
        assert_eq!(seq.start_time(), Duration::ZERO);
    }

    #[test]
    fn sequence_start_time_is_its_earliest_cue() {
        let seq = make_sequence(vec![
            make_cue(Duration::from_secs(10)),
            make_cue(Duration::from_secs(5)),
        ]);
        assert_eq!(seq.start_time(), Duration::from_secs(5));
    }

    #[test]
    fn sequence_pass_length_empty() {
        let seq = make_sequence(vec![]);
        assert_eq!(seq.pass_length(), Duration::ZERO);
    }

    #[test]
    fn sequence_pass_without_effects_lasts_to_its_last_cue() {
        // No effect ends anywhere, so a pass lasts until its last cue starts,
        // as an inline loop's always has.
        let seq = make_sequence(vec![
            make_cue(Duration::from_secs(2)),
            make_cue(Duration::from_secs(7)),
        ]);
        assert_eq!(seq.pass_length(), Duration::from_secs(5));
    }

    // ── determine_loop_count ─────────────────────────────────────

    #[test]
    fn loop_count_none() {
        assert_eq!(determine_loop_count(None).unwrap(), 1);
    }

    #[test]
    fn loop_count_once() {
        assert_eq!(determine_loop_count(Some(SequenceLoop::Once)).unwrap(), 1);
    }

    #[test]
    fn loop_count_loop() {
        assert_eq!(
            determine_loop_count(Some(SequenceLoop::Loop)).unwrap(),
            10000
        );
    }

    #[test]
    fn loop_count_specific() {
        assert_eq!(
            determine_loop_count(Some(SequenceLoop::Count(5))).unwrap(),
            5
        );
    }

    #[test]
    fn loop_count_pingpong_unsupported() {
        assert!(determine_loop_count(Some(SequenceLoop::PingPong)).is_err());
    }

    #[test]
    fn loop_count_random_unsupported() {
        assert!(determine_loop_count(Some(SequenceLoop::Random)).is_err());
    }

    // ── parse_light_shows (integration) ──────────────────────────

    #[test]
    fn parse_empty_content() {
        let shows = parse_light_shows("").unwrap();
        assert!(shows.is_empty());
    }

    #[test]
    fn parse_keeps_the_shows_in_file_order() {
        // #597: the shows came back in a HashMap, in an order that changed
        // from one parse to the next.
        let names = ["m", "c", "x", "a", "q", "b", "z", "k"];
        let content: String = names
            .iter()
            .map(|name| {
                format!(
                    "show \"{name}\" {{\n    @0.000\n    wash: static color: \"red\", \
                     duration: 1s\n}}\n"
                )
            })
            .collect();
        let shows = parse_light_shows(&content).unwrap();
        let parsed: Vec<&str> = shows.keys().map(String::as_str).collect();
        assert_eq!(parsed, names);
    }

    #[test]
    fn parse_comments_only() {
        let shows = parse_light_shows("# Just a comment\n").unwrap();
        assert!(shows.is_empty());
    }

    #[test]
    fn parse_minimal_show() {
        let content = r#"show "Test Show" {
    @0:00.000
    front_wash: static color: "red", duration: 5s
}"#;
        let shows = parse_light_shows(content).unwrap();
        assert_eq!(shows.len(), 1);
        let show = shows.get("Test Show").unwrap();
        assert_eq!(show.name, "Test Show");
        assert!(!show.cues.is_empty());
    }

    #[test]
    fn parse_show_multiple_cues() {
        let content = r#"show "Multi" {
    @0:00.000
    front_wash: static color: "red", duration: 5s

    @0:05.000
    front_wash: static color: "blue", duration: 5s
}"#;
        let shows = parse_light_shows(content).unwrap();
        let show = shows.get("Multi").unwrap();
        assert_eq!(show.cues.len(), 2);
        assert_eq!(show.cues[0].time, Duration::ZERO);
        assert_eq!(show.cues[1].time, Duration::from_secs(5));
    }

    #[test]
    fn parse_show_with_tempo() {
        let content = r#"tempo {
    bpm: 120
    time_signature: 4/4
}

show "Tempo Show" {
    @1/1
    front_wash: static color: "red", duration: 5s
}"#;
        let shows = parse_light_shows(content).unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Tempo Show"));
    }

    #[test]
    fn parse_invalid_syntax() {
        assert!(parse_light_shows("show {").is_err());
    }

    #[test]
    fn parse_multiple_shows() {
        let content = r#"show "A" {
    @0:00.000
    front: static color: "red", duration: 5s
}

show "B" {
    @0:00.000
    back: static color: "blue", duration: 5s
}"#;
        let shows = parse_light_shows(content).unwrap();
        assert_eq!(shows.len(), 2);
        assert!(shows.contains_key("A"));
        assert!(shows.contains_key("B"));
    }
}
