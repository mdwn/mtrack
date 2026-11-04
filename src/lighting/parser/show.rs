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
use std::rc::Rc;
use std::time::Duration;

use super::super::effects::EffectLayer;
use super::super::tempo::TempoMap;
use super::effect_parse::parse_effect_definition;
use super::error::{analyze_parsing_failure, describe_pest_error};
use super::grammar::{items, LightingParser, Rule};
use super::include::{add_included, Include, IncludingFile};
use super::sequences::{line_at, param_refs, SequenceLibrary};
use super::tempo_parse::parse_tempo_definition;
use super::types::{
    pass_length, pass_start, Cue, LayerCommand, LayerCommandType, LightShow, LightShows,
    ParseContext, Sequence, SequenceCall, SequenceLoop,
};
use super::utils::{
    parse_duration_string, parse_layer, parse_measure_time, parse_percentage_to_f64,
    parse_time_string,
};
use crate::tempo::{snap_beats, DEFAULT_BPM};
use pest::iterators::Pair;
use pest::Parser;

/// What `parse_cue` makes of one cue: what it puts on the timeline of
/// itself, and the sequences it references, placed by [`CueBlock::place`]
/// once the block's cues are all parsed -- a sequence's `length:` is
/// checked against its cues as written before any reference expands.
struct ParsedCue {
    /// Its time, with the offset in force.
    time: Duration,
    /// The offset in force at it, in seconds.
    offset_secs: f64,
    /// The cues it puts on the timeline of itself: its own cue, when it
    /// holds anything of its own, and its inline loops' passes.
    cues: Vec<Cue>,
    /// Its own cue's place in `cues`, when it holds anything of its own.
    base: Option<usize>,
    /// The sequences it references, in order.
    references: Vec<SequenceCall>,
    /// The offset in seconds the cues after it start from, if its `offset`
    /// or `reset` commands changed it.
    new_offset: Option<f64>,
    /// The measure offset the cues after it start from.
    measure_offset: u32,
    /// The latest cue time it produces, the passes it references included:
    /// what a standalone `offset` in a zero-time cue after it anchors to.
    last_time: Duration,
}

impl ParsedCue {
    /// The times of its cues as written, in order: itself, unless it holds
    /// nothing but inline loops (a cue holding only references counts, even
    /// to a sequence that expands to nothing), and its inline loops' cues;
    /// not the passes of the sequences it references.
    fn written(&self) -> Vec<Duration> {
        let mut written: Vec<Duration> = self.cues.iter().map(|c| c.time).collect();
        if self.base.is_none() && !self.references.is_empty() {
            written.push(self.time);
        }
        written.sort();
        written
    }
}

/// What `parse_cue_block` makes of a show's, a sequence's or an inline
/// loop body's cues, its sequence references still to place.
pub(super) struct CueBlock {
    cues: Vec<ParsedCue>,
    /// The block's own `tempo` block, if it has one.
    tempo_map: Option<TempoMap>,
    /// The tempo map the cues were parsed under: its own, else the file's.
    effective_tempo: Option<TempoMap>,
}

impl CueBlock {
    /// The cues' times as written (see [`ParsedCue::written`]), in order.
    fn written(&self) -> Vec<Duration> {
        self.cues.iter().flat_map(ParsedCue::written).collect()
    }

    /// The block's cues, each with the passes of the sequences it
    /// references placed from it, in time order within each.
    pub(super) fn place(self, library: &SequenceLibrary) -> Result<Vec<Cue>, Box<dyn Error>> {
        let mut cues = Vec::new();
        for mut parsed in self.cues {
            for call in &parsed.references {
                place_sequence(
                    call,
                    library,
                    &self.effective_tempo,
                    parsed.time,
                    parsed.offset_secs,
                    &mut parsed.cues,
                    parsed.base,
                )?;
            }
            // Generated cues come in source order, not time order (an
            // inline loop may write @0.5 before @0.0).
            parsed.cues.sort_by_key(|c| c.time);
            cues.extend(parsed.cues);
        }
        Ok(cues)
    }
}

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
    for pair in items(pairs) {
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
            _ => {}
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
                let block = parse_cue_block(inner_pair.into_inner(), global_tempo, library)?;
                tempo_map = block.tempo_map.clone();
                cues = block.place(library)?;
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
/// A sequence definition as written, its cues expanded.
pub(super) struct SequenceDefinition {
    name: String,
    pub(super) tempo_map: Option<TempoMap>,
    /// The `length:` value as written, e.g. `1measure`.
    length: Option<String>,
    /// Its cues, their references still to place.
    pub(super) block: CueBlock,
}

impl SequenceDefinition {
    /// The declared `length:` in time from the first cue, or `None` when the
    /// sequence declares none. Every cue as written must start inside it:
    /// one starting at or past the end would land on the next pass's first
    /// cue. (The passes of the sequences it references may run past it; a
    /// pass is cut off by the next, as a longer effect is.)
    pub(super) fn declared_length(
        &self,
        tempo_map: Option<&TempoMap>,
    ) -> Result<Option<Duration>, Box<dyn Error>> {
        let Some(written) = &self.length else {
            return Ok(None);
        };
        let written_times = self.block.written();
        let start = written_times.iter().min().copied().unwrap_or_default();
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
        if let Some(late) = written_times.iter().find(|&&t| t - start >= length) {
            return Err(format!(
                "Sequence '{}': a cue starts {:.3}s in, at or past its length of {} ({:.3}s)",
                self.name,
                (*late - start).as_secs_f64(),
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
    library: &SequenceLibrary,
) -> Result<SequenceDefinition, Box<dyn Error>> {
    let mut name = String::new();
    let mut length = None;
    let mut block = None;

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
                block = Some(parse_cue_block(
                    inner_pair.into_inner(),
                    global_tempo,
                    library,
                )?);
            }
            _ => {}
        }
    }

    let block = block.unwrap_or_else(|| CueBlock {
        cues: Vec::new(),
        tempo_map: None,
        effective_tempo: global_tempo.clone(),
    });
    Ok(SequenceDefinition {
        name,
        tempo_map: block.tempo_map.clone(),
        length,
        block,
    })
}

/// The cues of a show, a sequence or an inline loop body, in file order,
/// each placed after the `offset` and `reset` commands of the cues before
/// it. A `tempo` block among them governs all of them (the last one, if
/// there are several), else `global_tempo`; the block's own map is returned
/// with the cues. The sequences the cues reference are not placed yet.
fn parse_cue_block<'i>(
    pairs: impl Iterator<Item = Pair<'i, Rule>>,
    global_tempo: &Option<TempoMap>,
    library: &SequenceLibrary,
) -> Result<CueBlock, Box<dyn Error>> {
    let mut tempo_map: Option<TempoMap> = None;
    let mut cue_pairs = Vec::new();
    for pair in pairs {
        match pair.as_rule() {
            Rule::tempo => tempo_map = Some(parse_tempo_definition(pair)?),
            Rule::cue | Rule::sequence_cue | Rule::inline_loop_cue => cue_pairs.push(pair),
            _ => {}
        }
    }
    let effective_tempo = tempo_map.clone().or_else(|| global_tempo.clone());

    let mut cues = Vec::new();
    // What the cues so far leave in force: the offset in seconds, the
    // measure offset, and the last cue time standalone offsets anchor to.
    let mut offset_secs: f64 = 0.0;
    let mut cumulative_measure_offset: u32 = 0;
    let mut last_abs_time: Option<Duration> = None;
    for cue_pair in cue_pairs {
        let parsed = parse_cue(
            cue_pair,
            &effective_tempo,
            library,
            offset_secs,
            cumulative_measure_offset,
            last_abs_time,
        )?;
        if let Some(change) = parsed.new_offset {
            offset_secs = change;
        }
        cumulative_measure_offset = parsed.measure_offset;
        last_abs_time = Some(parsed.last_time);
        cues.push(parsed);
    }
    Ok(CueBlock {
        cues,
        tempo_map,
        effective_tempo,
    })
}
/// Calculate relative time from an absolute time and base time
#[inline]
fn relative_time(absolute_time: Duration, base_time: Duration) -> Duration {
    absolute_time.saturating_sub(base_time)
}

/// Apply time offset to a base time
#[inline]
fn apply_time_offset(base_time: Duration, offset_secs: f64) -> Duration {
    base_time + Duration::from_secs_f64(offset_secs)
}

/// Remove time offset from an absolute time
#[inline]
fn remove_time_offset(absolute_time: Duration, offset_secs: f64) -> Duration {
    absolute_time.saturating_sub(Duration::from_secs_f64(offset_secs))
}

/// Calculate iteration offset for sequence/loop expansion
#[inline]
fn iteration_offset(base_time: Duration, duration: Duration, iteration: usize) -> Duration {
    base_time + (duration * iteration as u32)
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

/// Adds a cue generated from a cue -- a pass of a sequence it references,
/// or of an inline loop it holds -- to that cue's expansion `cues`: into the
/// written cue itself, at `base`, when it falls at the same time, else as a
/// cue of its own.
fn merge_into(cues: &mut Vec<Cue>, base: Option<usize>, cue: Cue) {
    match base.filter(|&i| cues[i].time == cue.time) {
        Some(i) => {
            cues[i].effects.extend(cue.effects);
            cues[i].layer_commands.extend(cue.layer_commands);
            cues[i].stop_sequences.extend(cue.stop_sequences);
            cues[i].start_sequences.extend(cue.start_sequences);
        }
        None => cues.push(cue),
    }
}

/// The passes of the sequence `call` references, placed from `at`, the
/// time of the cue it is written in, under a show `offset` of
/// `offset_secs`: where each cue of each pass lands. A sequence is so many
/// beats long with each cue so many beats in, counted on the map it was
/// written against; the passes are laid on `tempo_map` from `at` by the
/// same count, so they follow every tempo change they run through, inside
/// a sequence as in a show.
struct Passes<'a> {
    sequence: Rc<Sequence>,
    tempo_map: &'a Option<TempoMap>,
    at: Duration,
    offset_secs: f64,
    count: usize,
    /// When the sequence's first pass starts, on its own timeline.
    base_time: Duration,
    /// How long one pass is, on its own timeline, and in beats.
    pass: Duration,
    pass_beats: f64,
}

impl<'a> Passes<'a> {
    fn new(
        call: &SequenceCall,
        library: &SequenceLibrary,
        tempo_map: &'a Option<TempoMap>,
        at: Duration,
        offset_secs: f64,
    ) -> Result<Self, Box<dyn Error>> {
        let sequence = library.instance(call)?;
        let count = determine_loop_count(call.loop_param)?;
        let base_time = pass_start(&sequence.cues);
        let pass = sequence.pass_length();
        let mut passes = Passes {
            sequence,
            tempo_map,
            at,
            offset_secs,
            count,
            base_time,
            pass,
            pass_beats: 0.0,
        };
        passes.pass_beats = passes.beats_in(base_time + pass);
        Ok(passes)
    }

    /// How many beats into the sequence a time on its own timeline is.
    /// Snapped: a time is whole nanoseconds, and its rounding error
    /// multiplied by the iteration count crept the loop later every pass
    /// (#534).
    fn beats_in(&self, time: Duration) -> f64 {
        match &self.sequence.tempo_map {
            Some(tm) => tm.beats_between(self.base_time, time),
            // In this order of operations, as it always was: a position
            // must come out bit for bit the same.
            None => {
                snap_beats(relative_time(time, self.base_time).as_secs_f64() * (DEFAULT_BPM / 60.0))
            }
        }
    }

    /// Where a cue at `time` on the sequence's own timeline lands in pass
    /// `iteration`.
    fn time_of(&self, iteration: usize, time: Duration) -> Duration {
        self.at
            + match self.tempo_map {
                Some(tm) => tm.beats_to_duration(
                    self.pass_beats * iteration as f64 + self.beats_in(time),
                    self.at,
                    self.offset_secs,
                ),
                None => Duration::from_secs_f64(
                    iteration as f64 * self.pass.as_secs_f64()
                        + relative_time(time, self.base_time).as_secs_f64(),
                ),
            }
    }

    /// Where the last cue of the last pass lands, if there is one.
    fn furthest(&self) -> Option<Duration> {
        let last = self.sequence.cues.iter().map(|c| c.time).max()?;
        Some(self.time_of(self.count.checked_sub(1)?, last))
    }
}

/// Places each pass of the sequence `call` references from `at`, the time
/// of the cue it is written in, under a show `offset` of `offset_secs`
/// (see [`Passes`]). Each pass after the first stops
/// the one before it, so its effects do not pile up; the first starts the
/// sequence afresh after any earlier `stop sequence`. An effect belongs to
/// every sequence it is started through, this one after the ones inside
/// it, and its tempo-aware rates and its durations in beats or measures
/// are read at the tempo where it plays (#525, #529).
fn place_sequence(
    call: &SequenceCall,
    library: &SequenceLibrary,
    tempo_map: &Option<TempoMap>,
    at: Duration,
    offset_secs: f64,
    cues: &mut Vec<Cue>,
    base: Option<usize>,
) -> Result<(), Box<dyn Error>> {
    let passes = Passes::new(call, library, tempo_map, at, offset_secs)?;
    let name = &call.name;
    for iteration in 0..passes.count {
        for (index, seq_cue) in passes.sequence.cues.iter().enumerate() {
            let mut cue = seq_cue.clone();
            cue.time = passes.time_of(iteration, seq_cue.time);
            if iteration > 0 && index == 0 {
                cue.stop_sequences.push(name.clone());
            }
            if iteration == 0 && index == 0 {
                cue.start_sequences.push(name.clone());
            }
            for effect in &mut cue.effects {
                effect.sequences.push(name.clone());
                effect.tempo_offset_secs = offset_secs;
                if let Some(tm) = tempo_map {
                    effect.resolve_musical_durations(tm, cue.time, offset_secs);
                }
            }
            merge_into(cues, base, cue);
        }
    }
    Ok(())
}

/// Parse and expand an inline loop into multiple cues
/// Returns a vector of cues representing all iterations of the loop
fn parse_and_expand_inline_loop(
    pair: Pair<Rule>,
    tempo_map: &Option<TempoMap>,
    base_cue_time: Duration,
) -> Result<Vec<Cue>, Box<dyn Error>> {
    let mut repeats = 1;
    if let Some(count) = pair
        .clone()
        .into_inner()
        .find(|p| p.as_rule() == Rule::number_value)
    {
        repeats = match parse_sequence_loop_param(count.as_str())? {
            SequenceLoop::Count(n) => n,
            _ => unreachable!("a number_value is a count"),
        };
    }

    // The body's cues are timed from 0, relative to the loop's start; no
    // sequences are available in an inline loop.
    let library = SequenceLibrary::empty();
    let loop_cues = parse_cue_block(pair.into_inner(), tempo_map, &library)?.place(&library)?;
    if loop_cues.is_empty() {
        return Ok(Vec::new());
    }

    let loop_base_time = pass_start(&loop_cues);
    let loop_duration = pass_length(&loop_cues);

    // Expand the loop by repeating it N times
    let mut expanded_cues = Vec::new();
    for iteration in 0..repeats {
        let iter_offset = iteration_offset(base_cue_time, loop_duration, iteration);

        for loop_cue in &loop_cues {
            let mut expanded_cue = loop_cue.clone();
            // Convert relative loop cue time to absolute time
            let rel_time = relative_time(loop_cue.time, loop_base_time);
            expanded_cue.time = iter_offset + rel_time;
            expanded_cues.push(expanded_cue);
        }
    }

    Ok(expanded_cues)
}

/// One cue of a show, a sequence or an inline loop body, as the cues it
/// puts on the timeline: itself, with each pass of the sequences it
/// references and the inline loops it holds placed from its time, in time
/// order. Also returns what it leaves in force for the cues after it: the
/// offset in seconds and in measures, if its `offset` or `reset` commands
/// changed them, and the latest cue time it produced.
fn parse_cue(
    pair: Pair<Rule>,
    tempo_map: &Option<TempoMap>,
    library: &SequenceLibrary,
    offset_secs: f64,
    cumulative_measure_offset: u32,
    last_abs_time: Option<Duration>,
) -> Result<ParsedCue, Box<dyn Error>> {
    let mut score_time = Duration::ZERO;
    let mut effect_pairs = Vec::new();
    let mut layer_command_pairs = Vec::new();
    let mut sequence_ref_pairs = Vec::new();
    let mut stop_sequence_pairs = Vec::new();
    let mut offset_commands = Vec::new();
    let mut reset_commands = Vec::new();
    let mut inline_loop_pairs = Vec::new();
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
            Rule::inline_loop => {
                inline_loop_pairs.push(inner_pair);
            }
            _ => {}
        }
    }

    // Resolve measure_time to score-space time: the position on the
    // unshifted map, kept as score_anchor too. The offset is applied to the
    // absolute time below.
    let mut unshifted_score_time = Duration::ZERO;
    if let Some(measure_pair) = measure_time_pair {
        let (measure, beat) = parse_measure_time(measure_pair.as_str())?;
        let Some(tm) = tempo_map else {
            return Err("Measure-based timing requires a tempo section".into());
        };
        score_time = tm
            .measure_to_time_with_offset(measure, beat, 0, 0.0)
            .ok_or_else(|| format!("Invalid measure/beat position: {}/{}", measure, beat))?;
        unshifted_score_time = score_time;
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
                let abs_time = remove_time_offset(t, applied_offset_secs);
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
                total_offset += tm.offset_duration(offset_measures, score_anchor);
            } else {
                return Err("Offset command requires a tempo section".into());
            }
        }
        new_offset = Some(total_offset);
    }

    // Absolute time uses the currently applied offset (not the newly computed one)
    let abs_time = apply_time_offset(score_time, applied_offset_secs);

    // The cue as written: its own effects, layer commands and stops.
    let mut base = Cue {
        time: abs_time,
        effects: Vec::new(),
        layer_commands: Vec::new(),
        stop_sequences: Vec::new(),
        start_sequences: Vec::new(),
    };
    for stop_pair in stop_sequence_pairs {
        let seq_name = stop_pair
            .into_inner()
            .find(|p| p.as_rule() == Rule::sequence_name)
            .ok_or("Stop sequence command missing sequence name")?
            .as_str()
            .trim_matches('"')
            .trim()
            .to_string();
        base.stop_sequences.push(seq_name);
    }
    let effect_ctx = ParseContext {
        tempo_map: tempo_map.clone(),
        cue_time: abs_time,
        offset_secs: applied_offset_secs,
    };
    for effect_pair in effect_pairs {
        base.effects
            .push(parse_effect_definition(effect_pair, &effect_ctx)?);
    }
    for layer_command_pair in layer_command_pairs {
        base.layer_commands
            .push(parse_layer_command(layer_command_pair)?);
    }

    // The written cue comes first, so what it holds is scheduled at its own
    // time whatever order the generated cues come in; a cue holding only
    // loops or references is nothing in itself.
    let has_base_content = !base.effects.is_empty()
        || !base.layer_commands.is_empty()
        || !base.stop_sequences.is_empty();
    let mut cues = Vec::new();
    let mut base_index = None;
    if has_base_content || (inline_loop_pairs.is_empty() && sequence_ref_pairs.is_empty()) {
        cues.push(base);
        base_index = Some(0);
    }
    for inline_loop_pair in inline_loop_pairs {
        for cue in parse_and_expand_inline_loop(inline_loop_pair, tempo_map, abs_time)? {
            merge_into(&mut cues, base_index, cue);
        }
    }
    let references = sequence_ref_pairs
        .into_iter()
        .map(parse_sequence_reference_pair)
        .collect::<Result<Vec<_>, _>>()?;
    // Reaching to where the referenced passes will land, though they are
    // placed later; a reference that fails is reported when it is.
    let furthest_pass = references
        .iter()
        .filter_map(|call| {
            Passes::new(call, library, tempo_map, abs_time, applied_offset_secs)
                .ok()?
                .furthest()
        })
        .max();
    let last_time = cues
        .iter()
        .map(|c| c.time)
        .chain(furthest_pass)
        .max()
        .unwrap_or(abs_time);

    Ok(ParsedCue {
        time: abs_time,
        offset_secs: applied_offset_secs,
        cues,
        base: base_index,
        references,
        new_offset,
        measure_offset: cumulative_measure_offset,
        last_time: last_time.max(abs_time),
    })
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
                                layer =
                                    Some(parse_layer(&param_value).ok_or_else(|| {
                                        format!("Invalid layer: {}", param_value)
                                    })?);
                            }
                            "intensity" => {
                                intensity =
                                    Some(parse_percentage_to_f64(&param_value)?.clamp(0.0, 1.0));
                            }
                            "speed" => {
                                speed = Some(parse_percentage_to_f64(&param_value)?.max(0.0));
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

    // ── relative_time ────────────────────────────────────────────

    #[test]
    fn relative_time_basic() {
        let result = relative_time(Duration::from_secs(5), Duration::from_secs(2));
        assert_eq!(result, Duration::from_secs(3));
    }

    #[test]
    fn relative_time_saturates_at_zero() {
        let result = relative_time(Duration::from_secs(1), Duration::from_secs(5));
        assert_eq!(result, Duration::ZERO);
    }

    #[test]
    fn relative_time_same() {
        let result = relative_time(Duration::from_secs(3), Duration::from_secs(3));
        assert_eq!(result, Duration::ZERO);
    }

    // ── apply_time_offset ────────────────────────────────────────

    #[test]
    fn apply_time_offset_basic() {
        let result = apply_time_offset(Duration::from_secs(1), 2.5);
        assert_eq!(result, Duration::from_secs_f64(3.5));
    }

    #[test]
    fn apply_time_offset_zero() {
        let result = apply_time_offset(Duration::from_secs(5), 0.0);
        assert_eq!(result, Duration::from_secs(5));
    }

    // ── remove_time_offset ───────────────────────────────────────

    #[test]
    fn remove_time_offset_basic() {
        let result = remove_time_offset(Duration::from_secs(5), 2.0);
        assert_eq!(result, Duration::from_secs(3));
    }

    #[test]
    fn remove_time_offset_saturates() {
        let result = remove_time_offset(Duration::from_secs(1), 5.0);
        assert_eq!(result, Duration::ZERO);
    }

    // ── iteration_offset ─────────────────────────────────────────

    #[test]
    fn iteration_offset_first() {
        let result = iteration_offset(Duration::from_secs(10), Duration::from_secs(5), 0);
        assert_eq!(result, Duration::from_secs(10));
    }

    #[test]
    fn iteration_offset_third() {
        let result = iteration_offset(Duration::from_secs(10), Duration::from_secs(5), 2);
        assert_eq!(result, Duration::from_secs(20));
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
