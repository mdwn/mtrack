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

use std::cell::Cell;
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
    beats_between, DroppedCue, MusicalLength, SequenceCall, SequenceLoop, UnexpandedSequenceCue,
};
use super::types::{
    Cue, LayerCommand, LayerCommandType, LightShow, LightShows, ParseContext, Sequence,
    SequenceGrid,
};
use super::utils::{
    parse_duration_string, parse_measure_time, parse_time_string, past_show, within_show,
};
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
    parse_file(content, path, root, external, None)
}

/// The sequences a file's shows play, as the player plays them, for drawing:
/// for each show, each sequence played from each of its cues, with the cues
/// it expands to and where its last pass ends. The same parse and expansion
/// the player's shows come from; `external` as [`parse_light_shows_with_tempo`]
/// takes it. A file with an `include` cannot be laid out (there is no path to
/// read it from).
pub fn layout_sequence_calls(
    content: &str,
    external: Option<&TempoMap>,
) -> Result<FileLayout, Box<dyn Error>> {
    let mut layout = FileLayout::default();
    parse_file(content, None, None, external, Some(&mut layout))?;
    Ok(layout)
}

/// A file's sequences, as [`layout_sequence_calls`] lays them out.
#[derive(Debug, Clone, Default)]
pub struct FileLayout {
    pub shows: Vec<ShowLayout>,
    /// Why the player would not load the file although it lays out: a
    /// sequence nothing plays that does not parse, say (`check_all`).
    pub problem: Option<String>,
}

/// A show's sequences, as [`layout_sequence_calls`] lays them out.
#[derive(Debug, Clone, Default)]
pub struct ShowLayout {
    pub name: String,
    pub calls: Vec<CallLayout>,
    /// The cues that did not parse, by their place among the show's cues,
    /// with why: they are passed over.
    pub cue_errors: Vec<(usize, String)>,
}

/// One sequence played from a show's cue.
#[derive(Debug, Clone)]
pub struct CallLayout {
    /// The cue it is played from: its place among the show's cues as written.
    pub cue: usize,
    /// Its place among that cue's sequence references.
    pub call: usize,
    pub name: String,
    /// The offset in force at the cue, in seconds.
    pub offset_secs: f64,
    /// Where it is played from.
    pub start: Duration,
    /// The cues it expands to, in time order: every pass, up to
    /// [`MAX_LAID_OUT_CUES`].
    pub cues: Vec<Cue>,
    /// Where its last pass laid out ends.
    pub end: Duration,
    /// Where each pass laid out ends.
    pub pass_ends: Vec<Duration>,
    /// Whether [`MAX_LAID_OUT_CUES`] stopped it before its last pass.
    pub truncated: bool,
    /// How many of its cues land past a pass's `length:` where it plays,
    /// and so do not play.
    pub dropped: usize,
    /// Why it cannot be played (an unknown sequence, one that does not
    /// parse, an argument it does not take), when it cannot: it has no cues.
    pub error: Option<String>,
}

/// The most cues a laid-out call carries (the passes that fit): a `loop:
/// loop` runs for ten thousand passes, which nothing draws.
pub const MAX_LAID_OUT_CUES: usize = 2000;

fn parse_file(
    content: &str,
    path: Option<&Path>,
    root: Option<&Path>,
    external: Option<&TempoMap>,
    mut layouts: Option<&mut FileLayout>,
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
    if layouts.is_some() {
        library = library.capped(MAX_LAID_OUT_CUES);
    }
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
    // Every sequence is checked, played or not. Laid out, one that does not
    // parse fails only the calls that play it, and the file says why the
    // player would not load it.
    match (library.check_all(), layouts.as_deref_mut()) {
        (Ok(()), _) => {}
        (Err(e), Some(layout)) => layout.problem = Some(e.to_string()),
        (Err(e), None) => return Err(e),
    }

    // Second pass: parse shows with tempo and sequences available
    let mut parsed_shows = Vec::new();
    for pair in show_pairs {
        if let Some((offset, name)) = param_refs(pair.as_str())
            .first()
            .filter(|_| layouts.is_none())
        {
            return Err(format!(
                "line {}: ${name} is a sequence parameter, but this is a show, not a sequence \
                 that declares it",
                pair.line_col().0 + line_at(pair.as_str(), *offset) - 1
            )
            .into());
        }
        let mut laid_out = ShowLayout::default();
        let mut show = parse_light_show_definition(
            pair,
            &global_tempo,
            &library,
            layouts.is_some().then_some(&mut laid_out),
        )?;
        if let Some(layout) = layouts.as_deref_mut() {
            laid_out.name = show.name.clone();
            layout.shows.push(laid_out);
        }
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
    mut layout: Option<&mut ShowLayout>,
) -> Result<LightShow, Box<dyn Error>> {
    let mut name = String::new();
    let mut cues = Vec::new();
    let mut tempo_map: Option<TempoMap> = None;
    let mut dropped = Vec::new();

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
                for (cue_index, cue_pair) in cue_pairs.into_iter().enumerate() {
                    // Laid out, a cue that does not parse (mid-edit, say)
                    // is said so and passed over; played, it fails the show.
                    let parsed = match (
                        parse_cue(cue_pair, &effective_tempo, &mut clock),
                        layout.as_deref_mut(),
                    ) {
                        (Ok((parsed, _)), _) => parsed,
                        (Err(e), Some(layout)) => {
                            layout.cue_errors.push((cue_index, e.to_string()));
                            continue;
                        }
                        (Err(e), None) => return Err(e),
                    };
                    for unexpanded in parsed {
                        let offset_secs = unexpanded.offset_secs;
                        let start = unexpanded.time;
                        let laying_out = layout.is_some();
                        let mut calls = layout.as_deref_mut().map(|layout| &mut layout.calls);
                        let mut on_call =
                            |call: usize, name: &str, played: Result<&PlayedCall, String>| {
                                let Some(calls) = calls.as_deref_mut() else {
                                    return;
                                };
                                let mut laid_out = CallLayout {
                                    cue: cue_index,
                                    call,
                                    name: name.to_string(),
                                    offset_secs,
                                    start,
                                    cues: Vec::new(),
                                    end: start,
                                    pass_ends: Vec::new(),
                                    truncated: false,
                                    dropped: 0,
                                    error: None,
                                };
                                match played {
                                    Ok(played) => {
                                        laid_out.cues =
                                            played.cues.iter().map(|p| p.cue.clone()).collect();
                                        laid_out.end = played.end;
                                        laid_out.pass_ends = played.pass_ends.clone();
                                        laid_out.truncated = played.truncated;
                                        laid_out.dropped = played.dropped.count;
                                    }
                                    Err(e) => laid_out.error = Some(e),
                                }
                                calls.push(laid_out);
                            };
                        let expanded = expand_cue(
                            unexpanded,
                            &effective_tempo,
                            library,
                            laying_out.then_some(LayoutCalls {
                                on_call: &mut on_call,
                            }),
                        )?;
                        dropped.extend(expanded.dropped);
                        let expanded = expanded.cues;
                        // A sequence played from the cue reaches as far as it
                        // goes, for the next cue's offset anchor.
                        if let Some(last) = expanded.iter().map(|c| c.cue.time).max() {
                            clock.reached(last);
                        }
                        // In a show, every cue is where it is played.
                        cues.extend(expanded.into_iter().map(|p| p.cue));
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
        dropped,
    })
}

/// A sequence definition as written, before nested references are expanded.
pub(super) struct SequenceDefinition {
    name: String,
    pub(super) tempo_map: Option<TempoMap>,
    /// The `length:` value as written, e.g. `1measure`.
    length: Option<String>,
    pub(super) cues: Vec<UnexpandedSequenceCue>,
    /// Whether any of its cues is timed in beats (see [`body_in_seconds`]).
    pub(super) musical: bool,
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
        if let Some(late) = self
            .cues
            .iter()
            .find(|c| past_pass_end(c.time - start, length))
        {
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
    let mut musical = false;

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
                    let (cues, cue_musical) = parse_cue(cue_pair, &effective_tempo, &mut clock)?;
                    unexpanded_cues.extend(cues);
                    musical |= cue_musical;
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
        musical,
    })
}

/// The passes `loop: loop` plays: for good, in practice. A count may not be
/// more.
const LOOP_FOREVER: usize = 10_000;

/// Determine loop count from a SequenceLoop parameter
pub(super) fn determine_loop_count(
    loop_param: Option<SequenceLoop>,
) -> Result<usize, Box<dyn Error>> {
    match loop_param {
        Some(SequenceLoop::Once) => Ok(1),
        Some(SequenceLoop::Loop) => Ok(LOOP_FOREVER),
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
            if count > LOOP_FOREVER {
                return Err(format!(
                    "Loop count {count} is more than {LOOP_FOREVER} (use 'loop' to loop for good)"
                )
                .into());
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

/// A cue as a body expands it, with -- for one that comes from a body purely
/// in seconds, played from it -- how far it is from where that body was
/// played (its anchor). A parent laying its body out in beats places the
/// anchor in beats and keeps the cue that far from it, so a body in seconds
/// keeps its seconds wherever it plays.
pub(super) struct PlacedCue {
    pub(super) cue: Cue,
    pub(super) from_anchor: Option<Duration>,
}

/// Whether a cue at `time` lands at or past the end of its pass, `end`
/// (both from the same start): the one rule, for a cue written there (an
/// error) as for one that lands there where it plays (dropped).
fn past_pass_end(time: Duration, end: Duration) -> bool {
    time >= end
}

/// A cue's distance from its anchor, with none for one on it: a cue on its
/// anchor is placed wherever the anchor is.
fn from_anchor(distance: Duration) -> Option<Duration> {
    (!distance.is_zero()).then_some(distance)
}

/// Parses one cue -- in a show, a sequence or an inline loop -- placing it
/// with `clock`. Its inline loops are expanded into cues of their own (merged
/// into the cue where they fall on it); its sequence references are kept, for a
/// show to play at once ([`expand_cue`]) and a sequence wherever it is
/// played ([`expand_sequence_call`]). Also answers whether the cue is timed in beats
/// (see [`body_in_seconds`]), even when it leaves no cue of its own.
fn parse_cue(
    pair: Pair<Rule>,
    tempo_map: &Option<TempoMap>,
    clock: &mut CueClock,
) -> Result<(Vec<UnexpandedSequenceCue>, bool), Box<dyn Error>> {
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
    within_show(time, "A cue")?;

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
    // A cue with nothing else to show for itself (its loops empty) is kept
    // too, empty: it still marks where its body starts.
    let mut cues = Vec::new();
    if has_base_content || loops.iter().all(|(loop_cues, _)| loop_cues.is_empty()) {
        cues.push(UnexpandedSequenceCue {
            time,
            offset_secs: placed.offset_secs,
            effects,
            layer_commands,
            stop_sequences,
            sequence_references,
            from_anchor: None,
        });
    }
    let has_base = !cues.is_empty();

    for (loop_cues, _) in loops {
        for PlacedCue {
            cue: loop_cue,
            from_anchor: distance,
            ..
        } in loop_cues
        {
            // A loop cue on the cue itself joins it.
            if has_base && cues[0].time == loop_cue.time && distance.is_none() {
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
                from_anchor: distance,
            });
        }
    }

    // A loop's cues come in source order, which need not be time order
    // (`@0.5` written before `@0.0`).
    cues.sort_by_key(|c| c.time);
    if let Some(last) = cues.iter().map(|c| c.time).max() {
        clock.reached(last);
    }
    Ok((cues, musical))
}

/// The cues a show's cue becomes, and what of the sequences it plays falls
/// past a pass's `length:` and does not play.
pub(super) struct CueExpansion {
    pub(super) cues: Vec<PlacedCue>,
    pub(super) dropped: Vec<DroppedCue>,
}

/// Told of each sequence a laid-out cue plays: its place among the cue's
/// references, its name, and how it plays or why it cannot.
type OnCall<'a> = dyn FnMut(usize, &str, Result<&PlayedCall, String>) + 'a;

/// What a layout wants of [`expand_cue`]: each sequence played, told to
/// `on_call` (its place among the cue's references, its name, and how it
/// plays or why it cannot), a call that cannot play skipped rather than
/// failing the cue.
struct LayoutCalls<'a> {
    on_call: &'a mut OnCall<'a>,
}

/// The cues a show's cue becomes: itself, with every sequence it references
/// played from it (a cue of a sequence's that falls on it joins it), laying
/// out each call for `layout` when there is one. Each call places at most
/// the library's cue cap (a layout's; none when played).
fn expand_cue(
    unexpanded: UnexpandedSequenceCue,
    tempo_map: &Option<TempoMap>,
    library: &SequenceLibrary,
    mut layout: Option<LayoutCalls>,
) -> Result<CueExpansion, Box<dyn Error>> {
    let mut cues: Vec<PlacedCue> = expand_cue_base(&unexpanded).into_iter().collect();
    let mut dropped = Vec::new();
    let has_base = !cues.is_empty();
    let base_anchor = unexpanded.from_anchor;
    for (index, call) in unexpanded.sequence_references.iter().enumerate() {
        let budget = Cell::new(library.max_cues());
        let reaches = std::cell::RefCell::new(std::collections::HashMap::new());
        let work = Cell::new(if library.laying_out() {
            MAX_LAID_OUT_CALLS
        } else {
            usize::MAX
        });
        let inside = Inside {
            library,
            budget: &budget,
            reaches: &reaches,
            work: &work,
            within: None,
            tags: true,
            effects_around: false,
            endless: false,
            // Laid out, a cue that does not play is only counted.
            record_drops: !library.laying_out(),
        };
        let played = match library.instance(call).and_then(|sequence| {
            play_call(
                call,
                &sequence,
                unexpanded.time,
                unexpanded.offset_secs,
                tempo_map,
                &inside,
            )
        }) {
            Ok(mut played) => {
                // Said as played from the show's cue.
                for d in &mut played.dropped.cues {
                    d.call = call.name.clone();
                    d.call_at = unexpanded.time;
                }
                played
            }
            Err(e) => match layout.as_mut() {
                Some(layout) => {
                    (layout.on_call)(index, &call.name, Err(e.to_string()));
                    continue;
                }
                None => return Err(e),
            },
        };
        if let Some(layout) = layout.as_mut() {
            (layout.on_call)(index, &call.name, Ok(&played));
        }
        dropped.extend(played.dropped.cues);
        for placed in played.cues {
            if has_base && cues[0].cue.time == placed.cue.time && placed.from_anchor == base_anchor
            {
                merge_into(&mut cues[0].cue, placed.cue);
                continue;
            }
            cues.push(placed);
        }
    }
    cues.sort_by_key(|c| c.cue.time);
    Ok(CueExpansion { cues, dropped })
}

/// A cue of a sequence's joining the cue it falls on.
fn merge_into(base: &mut Cue, cue: Cue) {
    base.effects.extend(cue.effects);
    base.layer_commands.extend(cue.layer_commands);
    base.stop_sequences.extend(cue.stop_sequences);
    base.start_sequences.extend(cue.start_sequences);
}

/// The cue an unexpanded cue keeps of its own, if it has anything to keep
/// besides the sequences it plays.
fn expand_cue_base(unexpanded: &UnexpandedSequenceCue) -> Option<PlacedCue> {
    keeps_its_own(unexpanded).then(|| PlacedCue {
        cue: Cue {
            time: unexpanded.time,
            effects: unexpanded.effects.clone(),
            layer_commands: unexpanded.layer_commands.clone(),
            stop_sequences: unexpanded.stop_sequences.clone(),
            start_sequences: vec![],
        },
        from_anchor: unexpanded.from_anchor,
    })
}

/// Whether a cue keeps a cue of its own where it is played: when it has
/// anything besides the sequences it plays, or plays none (it then marks
/// where its body starts).
pub(super) fn keeps_its_own(unexpanded: &UnexpandedSequenceCue) -> bool {
    !unexpanded.effects.is_empty()
        || !unexpanded.layer_commands.is_empty()
        || !unexpanded.stop_sequences.is_empty()
        || unexpanded.sequence_references.is_empty()
}

/// The cues that land past a pass's `length:` and do not play: each one,
/// played, for the lint to say where; laid out, only how many.
#[derive(Default)]
pub(super) struct Dropped {
    pub(super) cues: Vec<DroppedCue>,
    pub(super) count: usize,
}

impl Dropped {
    fn add(&mut self, record: bool, cue: impl FnOnce() -> DroppedCue) {
        self.count += 1;
        if record {
            self.cues.push(cue());
        }
    }

    fn extend(&mut self, other: Dropped) {
        self.count += other.count;
        self.cues.extend(other.cues);
    }
}

/// How far what a pass (or a whole call) plays reaches, by which the next
/// pass of the body around it starts: in time, where its content ends; in
/// seconds, where content in seconds ends; and on the beat grid of a body in
/// beats, in beats from where it is played. A sequence a body plays reaches
/// as far as it ends: a sequence in beats on its own grid (its own content
/// at the tempo it was written at, its `length:` in beats), one in seconds
/// where it lands -- so a sequence wrapped in another keeps its timing.
#[derive(Clone, Copy, Default)]
struct Reach {
    real: Option<Duration>,
    seconds: Option<Duration>,
    beats: Option<f64>,
}

impl Reach {
    fn reach_time(&mut self, time: Duration) {
        self.real = self.real.max(Some(time));
    }

    fn reach_seconds(&mut self, time: Duration) {
        self.reach_time(time);
        self.seconds = self.seconds.max(Some(time));
    }

    fn reach_beats(&mut self, beats: f64) {
        self.beats = Some(self.beats.map_or(beats, |b| b.max(beats)));
    }
}

/// A sequence played from a cue: its cues, where its last pass ends,
/// whether the cap on cues cut it short, what of it lands past a pass's
/// `length:`, how far what it plays reaches, and whether it stopped at the
/// end of a show (played for good).
struct PlayedCall {
    cues: Vec<PlacedCue>,
    end: Duration,
    pass_ends: Vec<Duration>,
    truncated: bool,
    dropped: Dropped,
    reach: Reach,
    cut: bool,
}

/// The end of a pass of a sequence with a `length:`, where it plays: a cue
/// of it (bar the pass's first), or of anything played inside it, that
/// lands at or past it does not play.
#[derive(Debug, Clone, Copy)]
struct PassEnd<'a> {
    time: Duration,
    /// The sequence whose pass it ends.
    sequence: &'a str,
}

/// The earlier of two pass ends.
fn earlier<'a>(a: Option<&PassEnd<'a>>, b: Option<&PassEnd<'a>>) -> Option<PassEnd<'a>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if b.time < a.time { *b } else { *a }),
        (a, b) => a.or(b).copied(),
    }
}

/// Plays the sequence `call` names from `at` (under an offset of
/// `offset_secs`), where it plays, through `tempo_map`, `inside` what it is
/// played inside.
fn play_call(
    call: &SequenceCall,
    sequence: &Sequence,
    at: Duration,
    offset_secs: f64,
    tempo_map: &Option<TempoMap>,
    inside: &Inside,
) -> Result<PlayedCall, Box<dyn Error>> {
    let passes = determine_loop_count(call.loop_param)?;
    let endless =
        inside.endless || matches!(call.loop_param, Some(SequenceLoop::Loop)) || sequence.endless;
    let expansion = expand_sequence_call(
        sequence,
        at,
        offset_secs,
        tempo_map,
        Some(&call.name),
        passes,
        &Inside { endless, ..*inside },
    )?;
    Ok(PlayedCall {
        cues: expansion.cues,
        end: expansion.end,
        pass_ends: expansion.pass_ends,
        truncated: expansion.truncated,
        dropped: expansion.dropped,
        reach: expansion.reach,
        cut: expansion.cut,
    })
}

/// How far a hollow sequence ([`Sequence::hollow`]) played from a call
/// reaches, in its own units: in beats on its grid from where it is played,
/// or in time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum HollowReach {
    Beats(f64),
    Seconds(Duration),
}

/// How far the hollow sequence `callee`, played by `call` (where there is a
/// tempo map, `with_map`), reaches -- worked out once per sequence and
/// count of passes, with the very arithmetic [`expand_sequence_call`] does,
/// so placing it costs nothing however deep its calls fan out. `None` where
/// that is not a matter of its own units: played for good, a `length:` in
/// seconds on a body in beats, or a sequence in seconds inside one in beats.
fn hollow_reach(
    library: &SequenceLibrary,
    callee: &std::rc::Rc<Sequence>,
    call: &SequenceCall,
    with_map: bool,
    endless: bool,
) -> Option<HollowReach> {
    if endless || matches!(call.loop_param, Some(SequenceLoop::Loop)) || callee.endless {
        return None;
    }
    let passes = determine_loop_count(call.loop_param).ok()?;
    let in_beats = with_map && !callee.in_seconds;
    let key = (std::rc::Rc::as_ptr(callee) as usize, passes, in_beats);
    if let Some(known) = library.hollow_reaches.borrow().get(&key) {
        return *known;
    }
    let reached = hollow_reach_of(library, callee, passes, with_map, in_beats);
    library.hollow_reaches.borrow_mut().insert(key, reached);
    reached
}

fn hollow_reach_of(
    library: &SequenceLibrary,
    sequence: &Sequence,
    passes: usize,
    with_map: bool,
    in_beats: bool,
) -> Option<HollowReach> {
    let base = sequence.start_time();
    // Each cue's calls, and how far each reaches.
    let mut cues = Vec::with_capacity(sequence.cues.len());
    for (index, cue) in sequence.cues.iter().enumerate() {
        let mut reaches = Vec::new();
        for call in sequence.calls.get(index).map_or(&[][..], Vec::as_slice) {
            let callee = library.instance(call).ok()?;
            reaches.push(hollow_reach(library, &callee, call, with_map, false)?);
        }
        cues.push((cue.time, reaches));
    }
    if in_beats {
        if sequence.length.is_some() && !sequence.length_in_beats {
            return None;
        }
        let declared = sequence.declared_beats();
        let anchors: Vec<f64> = cues
            .iter()
            .map(|(time, _)| sequence.beats_between(base, *time))
            .collect();
        let mut start = 0.0;
        for _ in 0..passes {
            if let Some(beats) = declared {
                start += beats;
                continue;
            }
            let mut grid: Option<f64> = None;
            for ((_, reaches), anchor) in cues.iter().zip(&anchors) {
                for reach in reaches {
                    match reach {
                        HollowReach::Beats(g) => {
                            let to = start + anchor + g;
                            grid = Some(grid.map_or(to, |b: f64| b.max(to)));
                        }
                        HollowReach::Seconds(Duration::ZERO) => {
                            let to = start + anchor;
                            grid = Some(grid.map_or(to, |b: f64| b.max(to)));
                        }
                        HollowReach::Seconds(_) => return None,
                    }
                }
            }
            let next = grid.unwrap_or(start);
            if next <= start {
                break;
            }
            start = next;
        }
        Some(HollowReach::Beats(start))
    } else {
        let mut start = Duration::ZERO;
        for _ in 0..passes {
            if let Some(length) = sequence.length {
                start = start.saturating_add(length);
                continue;
            }
            let mut next = start;
            for (time, reaches) in &cues {
                let placed = start.saturating_add(time.saturating_sub(base));
                for reach in reaches {
                    match reach {
                        HollowReach::Seconds(length) => {
                            next = next.max(placed.saturating_add(*length));
                        }
                        HollowReach::Beats(_) => return None,
                    }
                }
            }
            if next <= start {
                break;
            }
            start = next;
        }
        Some(HollowReach::Seconds(start))
    }
}

/// What a body is expanded inside: the sequences it may play; how many more
/// cues may be placed, dropped or passed over (shared by everything one
/// show's call plays: a layout's cap); the end of the enclosing pass it
/// must start before; whether a body around it has effects (then one
/// without any reaches no further for it than nothing); whether it is
/// played for good; and whether each cue that does not play is recorded.
#[derive(Clone, Copy)]
struct Inside<'a> {
    library: &'a SequenceLibrary,
    budget: &'a Cell<usize>,
    /// What the calls that played no cue reached, by where and how they
    /// were played (one show cue's calls share it): played again there,
    /// such a call costs nothing.
    reaches: &'a std::cell::RefCell<std::collections::HashMap<ReachKey, ReachOnly>>,
    /// How many more calls may be expanded, a layout's (generous) bound on
    /// the work that plays no cue; none when played.
    work: &'a Cell<usize>,
    within: Option<&'a PassEnd<'a>>,
    /// Whether its cues' effects are tagged here, as the outermost
    /// sequence's: tagged once, where they are finally played, rather than
    /// again at every level.
    tags: bool,
    effects_around: bool,
    endless: bool,
    record_drops: bool,
}

/// Where and how a call is played, all that what it reaches depends on
/// when it plays no cue: the sequence (and its bindings), its passes,
/// whether it plays for good, where it starts and under what offset, the
/// end of the pass around it, and whether a body around it has effects.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ReachKey {
    sequence: usize,
    passes: usize,
    endless: bool,
    at: Duration,
    offset_bits: u64,
    within: Option<Duration>,
    effects_around: bool,
}

/// What a call that played no cue reached.
#[derive(Clone, Copy)]
struct ReachOnly {
    reach: Reach,
    cut: bool,
}

/// The most calls a layout expands for one show cue: far beyond any real
/// show, short of hanging the page.
const MAX_LAID_OUT_CALLS: usize = 200_000;

impl Inside<'_> {
    /// One more cue placed, dropped or passed over, if the cap allows it.
    fn spend(&self) -> bool {
        let left = self.budget.get();
        if left == 0 {
            return false;
        }
        self.budget.set(left - 1);
        true
    }
}

/// What [`expand_sequence_call`] plays.
pub(super) struct Expansion {
    pub(super) cues: Vec<PlacedCue>,
    /// Where the last pass ends.
    end: Duration,
    /// Where each pass ends.
    pass_ends: Vec<Duration>,
    /// Whether the cap on cues stopped it before its last cue.
    pub(super) truncated: bool,
    /// The cues that land past a pass's `length:`, and do not play.
    dropped: Dropped,
    /// How far what it plays reaches, for a body it is played inside.
    reach: Reach,
    /// Whether it stopped at the end of a show (played for good).
    cut: bool,
}

/// Where a cue's content ends, for where a pass without a `length:` ends:
/// its last effect's end, or -- in a body with no effect at all -- the cue
/// itself. A cue with no effect in a body with some (a layer command, say)
/// does not count. The one rule, for a body in beats as in seconds.
fn content_end_of(cue: &Cue, has_effects: bool) -> Option<Duration> {
    if has_effects {
        cue.effects
            .iter()
            .map(|e| cue.time.saturating_add(e.total_duration()))
            .max()
    } else {
        Some(cue.time)
    }
}

/// Plays `passes` passes of `sequence` from `at`, under an offset of
/// `offset_secs`: a sequence referenced from a cue (`name` given, so its
/// effects can be stopped by it) or an inline loop's body (no name).
///
/// A body purely in seconds ([`body_in_seconds`]) keeps its times: its cues
/// and passes stay in seconds from `at`, whatever the tempo does. Any other
/// body has its cues counted in beats, through its own tempo map, and placed
/// through the tempo map where it is played: from the beat a pass starts on,
/// so a pass that crosses a tempo change (or a ramp) follows it. The
/// sequences a body plays are played where its cue lands, through the same
/// map, in turn: every sequence, however deep, is laid out where it plays.
///
/// This is the one rule for where a pass ends. A pass lasts its declared
/// `length:` -- a cue that lands at or past it, of this sequence or of one
/// played inside it, as one in seconds can where the tempo is faster than
/// the body was written at, does not play, as one written there cannot, and
/// is listed in `dropped`. Without one, a pass of a body in seconds lasts
/// until its content ends ([`content_end_of`]); a pass of a body in beats
/// lasts as its own content does on its beat grid (effects' lengths in
/// seconds counted at the tempo it was written at), as long as a sequence
/// it plays lasts (one in beats on its own grid, one in seconds where it
/// lands, each with its `length:`, as played straight), or until a loop in
/// seconds in it ends where that lands, whichever is latest ([`Reach`]). A cue that does
/// not play makes no pass longer, and a pass that lasts no time is the last.
/// Effects' durations in beats or measures are resolved where they play
/// (#529). Without a tempo map where it is played, a body keeps the times
/// it has.
///
/// A cue or effect reaching past the end a show can have is an error -- but
/// for a sequence played for good (`loop: loop`, holding one, or inside
/// one), which plays what falls inside it: the pass a cue falls past the end
/// in is its last.
fn expand_sequence_call(
    sequence: &Sequence,
    at: Duration,
    offset_secs: f64,
    tempo_map: &Option<TempoMap>,
    name: Option<&str>,
    passes: usize,
    inside: &Inside,
) -> Result<Expansion, Box<dyn Error>> {
    let in_seconds = sequence.in_seconds;
    let endless = inside.endless;
    let base = match sequence.grid.get() {
        Some(grid) => grid.base,
        None => sequence.start_time(),
    };
    let distance = |index: usize| sequence.from_anchor.get(index).copied().flatten();
    let calls_of = |index: usize| sequence.calls.get(index).map_or(&[][..], Vec::as_slice);
    let keeps = |index: usize| sequence.keeps.get(index).copied().unwrap_or(true);
    let has_effects = sequence.has_effects;
    let label = name.unwrap_or("a loop");
    // A time past the end a show can have is an error -- but for a sequence
    // played for good, which plays what falls inside it and stops there.
    let within = |time: Duration| -> Result<(), Box<dyn Error>> {
        if !endless {
            within_show(time, label)?;
        }
        Ok(())
    };
    let past = |time: Duration| past_show(time.as_secs_f64());
    // In beats, each cue is placed by its anchor's beat (its own, for one
    // with none), counted from the start of the body; and its content ends
    // on the body's beat grid where its effects end at the tempo the body
    // was written at.
    let grid = sequence.grid.get_or_init(|| SequenceGrid {
        base,
        anchor_beats: sequence
            .cues
            .iter()
            .enumerate()
            .map(|(index, cue)| {
                sequence.beats_between(
                    base,
                    cue.time.saturating_sub(distance(index).unwrap_or_default()),
                )
            })
            .collect(),
        grid_ends: sequence
            .cues
            .iter()
            .enumerate()
            .map(|(index, cue)| {
                (distance(index).is_none() && keeps(index))
                    .then(|| content_end_of(cue, has_effects))
                    .flatten()
                    .map(|end| sequence.beats_between(base, end))
            })
            .collect(),
        declared_beats: sequence.declared_beats(),
    });
    let anchor_beats = &grid.anchor_beats;
    let grid_ends = &grid.grid_ends;
    let beats_tm = tempo_map.as_ref().filter(|_| !in_seconds);
    let declared_beats = grid.declared_beats;
    // Where `beats` beats from `at` fall.
    let at_beats = |tm: &TempoMap, beats: f64| {
        let beats = snap_beats(beats);
        // No beats on is `at` itself, not its time read back.
        if beats == 0.0 {
            return at;
        }
        at.saturating_add(tm.beats_after(beats, at, offset_secs))
    };
    // The beats from `at` to `time`, on the same footing.
    let beats_to = |tm: &TempoMap, time: Duration| {
        let score = |t: Duration| t.saturating_sub(Duration::from_secs_f64(offset_secs.max(0.0)));
        beats_between(Some(tm), score(at), score(time))
    };
    // How far a time placed here (`placed`, `own` from its anchor in the
    // body) is from the anchor it keeps: only a body in seconds laid out in
    // seconds is anchored where it is played; a body in beats keeps its own
    // anchors (none, for its own cues).
    let anchor_of = |placed: Duration, own: Option<Duration>| {
        if beats_tm.is_none() && in_seconds {
            from_anchor(placed.saturating_sub(at))
        } else {
            own
        }
    };
    // Where a cue of the body (`index`) falls in a pass starting
    // `start_beats` from `at` (in beats) or at `start_time`, and how far
    // that is from its anchor.
    let place = |index: usize, start_beats: f64, start_time: Duration, origin: Option<Duration>| {
        let own = distance(index);
        let placed = match (beats_tm, origin) {
            // A pass that starts at a time (where content in seconds, or a
            // length in seconds, ended) counts its beats from there.
            // Each beat of the pass counted from the same exact start.
            (Some(tm), Some(origin)) => origin
                .saturating_add(tm.beats_after(
                    snap_beats(anchor_beats[index]),
                    origin,
                    offset_secs,
                ))
                .saturating_add(own.unwrap_or_default()),
            (Some(tm), None) => at_beats(tm, start_beats + anchor_beats[index])
                .saturating_add(own.unwrap_or_default()),
            (None, _) => start_time.saturating_add(sequence.cues[index].time.saturating_sub(base)),
        };
        (placed, anchor_of(placed, own))
    };
    // Its effects are this sequence's, to stop with it, and read the tempo
    // map as shifted where it plays.
    let tag = |cue: &mut Cue| {
        for effect in &mut cue.effects {
            if let Some(name) = name {
                effect.sequence_name = Some(name.to_string());
            }
            effect.tempo_offset_secs = offset_secs;
        }
    };
    // How far a sequence it plays, from `time`, `beat` beats into this
    // body's grid, reaches for this pass: one in beats on the grid, from its
    // beat; one in seconds where it lands -- one that reaches no further
    // than where it is played reaching its beat, with no time read back.
    let take_reach = |pass_reach: &mut Reach, reach: Reach, beat: f64, time: Duration| match (
        beats_tm,
        reach.beats,
    ) {
        (Some(_), Some(grid)) => {
            pass_reach.reach_beats(beat + grid);
            if let Some(real) = reach.real {
                pass_reach.reach_time(real);
            }
        }
        (Some(_), None) if reach.real == Some(time) => {
            pass_reach.reach_beats(beat);
            pass_reach.reach_time(time);
        }
        _ => {
            if let Some(real) = reach.real {
                pass_reach.reach_seconds(real);
            }
        }
    };
    // What a sequence it plays is played inside.
    let around = Inside {
        effects_around: inside.effects_around || has_effects,
        tags: false,
        ..*inside
    };

    let mut cues: Vec<PlacedCue> = Vec::new();
    let mut pass_ends = Vec::new();
    let mut dropped = Dropped::default();
    let mut truncated = false;
    let mut cut = false;
    // Where the current pass starts: in beats from `at`, or in time.
    let mut start_beats = 0.0;
    let mut start_time = at;
    let mut end = at;
    // Where the pass starts, when that is a time: the exact end of content
    // in seconds, or of a length in seconds, before it.
    let mut origin: Option<Duration> = None;
    'passes: for pass in 0..passes {
        // A pass that starts past the end of a show (of a sequence played
        // for good) is not played, nor any after it.
        let pass_start = match (beats_tm, origin) {
            (Some(tm), None) => at_beats(tm, start_beats),
            _ => start_time,
        };
        if past(pass_start) {
            within(pass_start)?;
            cut = true;
            break 'passes;
        }
        // Where the pass is stopped by the next, when its length is
        // declared; and where its content ends, for the next when not.
        let stop = match (beats_tm, declared_beats) {
            (Some(tm), Some(beats)) => Some(at_beats(tm, start_beats + beats)),
            // A length in seconds lasts its seconds where it plays, from
            // the exact start of the pass.
            (Some(_), None) => sequence
                .length
                .map(|length| start_time.saturating_add(length)),
            (None, _) => sequence
                .length
                .map(|length| start_time.saturating_add(length)),
        };
        if let Some(stop) = stop {
            within(stop)?;
        }
        // What of this pass must start before: its end, and the enclosing
        // pass's; its first cue, which starts it, the enclosing pass's only.
        let own_end = stop.map(|time| PassEnd {
            time,
            sequence: label,
        });
        let pass_end = earlier(own_end.as_ref(), inside.within);
        let nested = Inside {
            within: pass_end.as_ref(),
            ..around
        };
        let pass_first = cues.len();
        let mut pass_reach = Reach::default();
        // Whether a cue fell past the end of a show: then this pass is the
        // last.
        let mut skipped = false;
        for (index, seq_cue) in sequence.cues.iter().enumerate() {
            let (time, anchored) = place(index, start_beats, start_time, origin);
            // Each cue placed, dropped or passed over counts against the
            // cap; one that only plays sequences does not (theirs do).
            if (keeps(index) || past(time)) && !inside.spend() {
                truncated = true;
                break;
            }
            if past(time) {
                within(time)?;
                skipped = true;
                continue;
            }
            let ends_by = if index > 0 {
                pass_end.as_ref()
            } else {
                inside.within
            };
            // The cue's own content, unless it lands past its pass's end.
            let group = cues.len();
            let mut own = None;
            if keeps(index) {
                match ends_by.filter(|e| past_pass_end(time, e.time)) {
                    Some(e) => dropped.add(inside.record_drops, || DroppedCue {
                        sequence: e.sequence.to_string(),
                        call: String::new(),
                        call_at: Duration::ZERO,
                        time,
                        pass_end: e.time,
                    }),
                    None => {
                        let mut cue = seq_cue.clone();
                        cue.time = time;
                        if inside.tags {
                            tag(&mut cue);
                        }
                        match (beats_tm, distance(index)) {
                            // On the grid the pass counts its beats on, from
                            // the cue's beat there: an effect up to a later
                            // cue ends where that cue starts.
                            (Some(tm), None) => {
                                let (frame, into) = match origin {
                                    Some(origin) => (origin, anchor_beats[index]),
                                    None => (at, start_beats + anchor_beats[index]),
                                };
                                let from = tm.grid_beat_at(frame, offset_secs) + into;
                                for effect in &mut cue.effects {
                                    effect.resolve_musical_durations_on_grid(tm, from);
                                }
                            }
                            _ => {
                                if let Some(tm) = tempo_map {
                                    for effect in &mut cue.effects {
                                        effect.resolve_musical_durations(tm, cue.time, offset_secs);
                                    }
                                }
                            }
                        }
                        if let Some(cue_end) = content_end_of(&cue, has_effects) {
                            // An effect that starts inside a show plays on,
                            // past its end, in a sequence played for good.
                            within(cue_end)?;
                            match (anchored, grid_ends[index]) {
                                (None, Some(grid)) if beats_tm.is_some() => {
                                    pass_reach.reach_time(cue_end);
                                    pass_reach.reach_beats(start_beats + grid);
                                }
                                _ => pass_reach.reach_seconds(cue_end),
                            }
                        }
                        own = Some(cues.len());
                        cues.push(PlacedCue {
                            cue,
                            from_anchor: anchored,
                        });
                    }
                }
            }
            // The sequences it plays, played where it lands, inside this
            // pass: each reaches as far as it ends -- one in beats on its
            // grid, from the beat it is played on; one in seconds where it
            // lands.
            for (which, call) in calls_of(index).iter().enumerate() {
                // A hollow sequence (nothing in it, however deep, keeps a
                // cue of its own) plays nothing: only how far it reaches
                // counts, and that is worked out once, in its own units.
                let callee = match sequence.callees.get(index).and_then(|c| c.get(which)) {
                    Some(callee) => callee.clone(),
                    None => inside.library.instance(call)?,
                };
                if callee.hollow {
                    if nested.effects_around {
                        // It reaches no further than nothing, here.
                        continue;
                    }
                    let reached = hollow_reach(
                        inside.library,
                        &callee,
                        call,
                        tempo_map.is_some(),
                        nested.endless,
                    );
                    if let Some(reached) = reached {
                        let reach = match (beats_tm, reached) {
                            (Some(tm), HollowReach::Beats(grid)) => Reach {
                                real: Some(time.saturating_add(tm.beats_to_duration(
                                    snap_beats(grid),
                                    time,
                                    offset_secs,
                                ))),
                                seconds: None,
                                beats: Some(grid),
                            },
                            (_, HollowReach::Seconds(length)) => {
                                let ends = time.saturating_add(length);
                                Reach {
                                    real: Some(ends),
                                    seconds: Some(ends),
                                    beats: None,
                                }
                            }
                            (None, HollowReach::Beats(_)) => Reach::default(),
                        };
                        if let Some(ends) = reach.real {
                            within(ends)?;
                        }
                        take_reach(
                            &mut pass_reach,
                            reach,
                            start_beats + anchor_beats[index],
                            time,
                        );
                        continue;
                    }
                }
                // A call that played no cue here before reaches as far again.
                let key = ReachKey {
                    sequence: std::rc::Rc::as_ptr(&callee) as usize,
                    passes: determine_loop_count(call.loop_param)?,
                    endless: nested.endless || matches!(call.loop_param, Some(SequenceLoop::Loop)),
                    at: time,
                    offset_bits: offset_secs.to_bits(),
                    within: nested.within.map(|e| e.time),
                    effects_around: nested.effects_around,
                };
                let known = inside.reaches.borrow().get(&key).copied();
                if let Some(known) = known {
                    skipped |= known.cut;
                    take_reach(
                        &mut pass_reach,
                        known.reach,
                        start_beats + anchor_beats[index],
                        time,
                    );
                    continue;
                }
                // A layout's bound on the work, however it fans out.
                let left = inside.work.get();
                if left == 0 {
                    truncated = true;
                    break;
                }
                inside.work.set(left - 1);
                let played = play_call(call, &callee, time, offset_secs, tempo_map, &nested)?;
                if played.cues.is_empty() && played.dropped.count == 0 && !played.truncated {
                    inside.reaches.borrow_mut().insert(
                        key,
                        ReachOnly {
                            reach: played.reach,
                            cut: played.cut,
                        },
                    );
                }
                dropped.extend(played.dropped);
                skipped |= played.cut;
                take_reach(
                    &mut pass_reach,
                    played.reach,
                    start_beats + anchor_beats[index],
                    time,
                );
                for mut placed in played.cues {
                    if inside.tags {
                        tag(&mut placed.cue);
                    }
                    match own.filter(|&i| {
                        cues[i].cue.time == placed.cue.time && distance(index) == placed.from_anchor
                    }) {
                        Some(i) => merge_into(&mut cues[i].cue, placed.cue),
                        None => {
                            let from = anchor_of(placed.cue.time, placed.from_anchor);
                            cues.push(PlacedCue {
                                cue: placed.cue,
                                from_anchor: from,
                            });
                        }
                    }
                }
                if played.truncated {
                    truncated = true;
                    break;
                }
            }
            // The cue and what it plays, in time order (in the order
            // written, at one time).
            cues[group..].sort_by_key(|c| c.cue.time);
            if truncated {
                break;
            }
        }
        // The pass's first cue starts it; each after the first stops the
        // one before, so its effects (a multiply chase, say) do not pile up,
        // and the first clears any earlier stop of the sequence, so it plays
        // again.
        if let (Some(name), Some(first)) = (name, cues.get_mut(pass_first)) {
            if pass > 0 {
                first.cue.stop_sequences.push(name.to_string());
            } else {
                first.cue.start_sequences.push(name.to_string());
            }
        }
        if truncated {
            // Cut short, it ends where what it laid out does (content that
            // reaches no further for it included): not where what was
            // passed over or dropped would have.
            let laid = cues
                .iter()
                .map(|c| content_end_of(&c.cue, true).map_or(c.cue.time, |e| e.max(c.cue.time)))
                .max();
            end = end.max(laid.unwrap_or(start_time));
            pass_ends.push(end);
            break 'passes;
        }
        // The next pass starts where this one stops, or where its content
        // ends; a pass that lasted no time is the last, as every one after
        // it would be the same instant.
        let lasted = match (beats_tm, stop) {
            (_, Some(stop)) => {
                end = stop;
                start_time = stop;
                match (beats_tm, declared_beats) {
                    (Some(_), Some(beats)) => start_beats += beats,
                    (Some(tm), None) => {
                        start_beats = beats_to(tm, stop);
                        origin = Some(stop);
                    }
                    _ => {}
                }
                true
            }
            (Some(tm), None) => {
                let grid_beats = pass_reach.beats.unwrap_or(start_beats);
                match pass_reach
                    .seconds
                    .filter(|t| beats_to(tm, *t) >= grid_beats)
                {
                    // Content in seconds ends it: the next pass starts
                    // there, exactly.
                    Some(t) => {
                        let lasted = t > start_time;
                        if lasted {
                            start_time = t;
                            start_beats = beats_to(tm, t);
                            origin = Some(t);
                        }
                        end = start_time;
                        lasted
                    }
                    None => {
                        let lasted = grid_beats > start_beats;
                        if lasted {
                            start_beats = grid_beats;
                            origin = None;
                            start_time = at_beats(tm, start_beats);
                        }
                        end = start_time;
                        lasted
                    }
                }
            }
            (None, None) => {
                let next = pass_reach.real.unwrap_or(start_time).max(start_time);
                let lasted = next > start_time;
                end = next;
                start_time = next;
                lasted
            }
        };
        pass_ends.push(end);
        if skipped {
            cut = true;
        }
        if skipped || !lasted {
            break 'passes;
        }
    }
    // It reaches as far as it ends: on its grid, in beats; in seconds, where
    // it lands. A body with no effect, inside one with some, reaches no
    // further for it than nothing: its cues' times count for its own passes
    // alone.
    let reach = if !has_effects && inside.effects_around {
        Reach::default()
    } else {
        // One that ended where content in seconds (or a length in seconds)
        // did reaches there, in seconds: what plays it starts after it at
        // that very time, as after it played straight.
        let in_time = beats_tm.is_none() || (origin.is_some() && !truncated);
        Reach {
            real: Some(end),
            seconds: in_time.then_some(end),
            beats: beats_tm.filter(|_| !in_time).map(|tm| {
                if truncated {
                    beats_to(tm, end)
                } else {
                    start_beats
                }
            }),
        }
    };
    Ok(Expansion {
        cues,
        end,
        pass_ends,
        truncated,
        dropped,
        reach,
        cut,
    })
}

/// Whether a body -- a sequence's, or an inline loop's -- is purely in
/// seconds: none of its cues timed in beats (`musical`, as [`parse_cue`]
/// answers it: at a bar and beat, issuing an `offset` in measures, with an
/// effect lasting beats or measures, or holding an inline loop that is not
/// purely in seconds), no sequence played from it that is not purely in
/// seconds itself, and no `length:` in beats or measures. Such a body is laid
/// out in seconds wherever it plays; any other in beats.
pub(super) fn body_in_seconds(
    musical: bool,
    cues: &[UnexpandedSequenceCue],
    length: Option<&str>,
    library: &SequenceLibrary,
) -> Result<bool, Box<dyn Error>> {
    if musical || length.and_then(MusicalLength::parse).is_some() {
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
/// it: its body is an unnamed sequence, `repeats` passes of it. Also answers
/// whether the body is timed in beats.
fn parse_inline_loop(
    pair: Pair<Rule>,
    tempo_map: &Option<TempoMap>,
    at: Duration,
    offset_secs: f64,
) -> Result<(Vec<PlacedCue>, bool), Box<dyn Error>> {
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
                if repeats > LOOP_FOREVER {
                    return Err(
                        format!("Loop repeats {repeats} is more than {LOOP_FOREVER}").into(),
                    );
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
    let mut musical = false;
    for loop_cue_pair in loop_cue_pairs {
        let (cues, cue_musical) = parse_cue(loop_cue_pair, &body_tempo, &mut clock)?;
        parsed.extend(cues);
        musical |= cue_musical;
    }
    // A loop's body references no sequence, so no library is needed.
    let in_seconds = body_in_seconds(musical, &parsed, None, &SequenceLibrary::empty())?;
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
    let has_effects = body.iter().any(|c| !c.effects.is_empty());
    let sequence = Sequence {
        cues: body,
        tempo_map: body_tempo,
        length: None,
        length_in_beats: false,
        in_seconds,
        from_anchor: Vec::new(),
        calls: Vec::new(),
        callees: Vec::new(),
        grid: std::cell::OnceCell::new(),
        keeps: Vec::new(),
        endless: false,
        hollow: false,
        has_effects,
    };
    Ok((
        expand_sequence_call(
            &sequence,
            at,
            offset_secs,
            tempo_map,
            None,
            repeats,
            &Inside {
                library: &SequenceLibrary::empty(),
                budget: &Cell::new(usize::MAX),
                reaches: &std::cell::RefCell::new(std::collections::HashMap::new()),
                work: &Cell::new(usize::MAX),
                within: None,
                tags: true,
                effects_around: false,
                endless: false,
                record_drops: true,
            },
        )?
        .cues,
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

#[cfg(all(test, not(mtrack_wasm)))]
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
            length_in_beats: false,
            in_seconds: false,
            from_anchor: Vec::new(),
            calls: Vec::new(),
            callees: Vec::new(),
            grid: std::cell::OnceCell::new(),
            keeps: Vec::new(),
            endless: false,
            hollow: false,
            has_effects: false,
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
    fn a_pass_without_effects_lasts_to_its_last_cue() {
        // No effect ends anywhere, so a pass lasts until its last cue starts,
        // as an inline loop's always has.
        let seq = make_sequence(vec![
            make_cue(Duration::from_secs(2)),
            make_cue(Duration::from_secs(7)),
        ]);
        let expansion = expand_sequence_call(
            &seq,
            Duration::ZERO,
            0.0,
            &None,
            None,
            2,
            &Inside {
                library: &SequenceLibrary::empty(),
                budget: &Cell::new(usize::MAX),
                reaches: &std::cell::RefCell::new(std::collections::HashMap::new()),
                work: &Cell::new(usize::MAX),
                within: None,
                tags: true,
                effects_around: false,
                endless: false,
                record_drops: true,
            },
        )
        .unwrap();
        let times: Vec<u64> = expansion
            .cues
            .iter()
            .map(|c| c.cue.time.as_secs())
            .collect();
        assert_eq!(times, vec![0, 5, 5, 10]);
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
