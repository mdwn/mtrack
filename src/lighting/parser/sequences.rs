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
//! A file's sequence definitions, expanded on demand.
//!
//! A sequence may declare parameters (#540), so what it expands to depends on
//! where it is used. Each definition keeps its source text; using it binds
//! the parameters, substitutes each `$name` in that text, and parses the
//! result like any other sequence. Every check the parser already makes then
//! applies to the substituted values, and each distinct set of values is
//! expanded once.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::rc::Rc;

use pest::iterators::Pair;
use pest::Parser;

use super::super::tempo::TempoMap;
use super::grammar::{LightingParser, Rule};
use super::show::{expand_unexpanded_sequence_cue, parse_sequence_structure};
use super::types::{Sequence, SequenceCall};

/// A sequence as written: its parameters and its source.
struct SequenceTemplate {
    /// The line the definition starts on.
    line: usize,
    /// Each parameter with its default, in declaration order.
    params: Vec<(String, Option<String>)>,
    /// The definition's text, preceded by blank lines and spaces so that
    /// line and column numbers within it are those of the file.
    source: String,
}

impl SequenceTemplate {
    fn standalone(&self) -> bool {
        self.params.iter().all(|(_, default)| default.is_some())
    }
}

/// The parameter values one expansion of a sequence is bound to.
type Bindings = BTreeMap<String, String>;

/// Every sequence a file defines, expanded per distinct set of arguments.
pub(crate) struct SequenceLibrary {
    templates: HashMap<String, SequenceTemplate>,
    global_tempo: Option<TempoMap>,
    instances: RefCell<HashMap<(String, Bindings), Rc<Sequence>>>,
    /// The sequences being expanded, outermost first, to catch cycles.
    expanding: RefCell<Vec<String>>,
}

impl SequenceLibrary {
    /// A library with no sequences in it.
    pub(crate) fn empty() -> Self {
        SequenceLibrary {
            templates: HashMap::new(),
            global_tempo: None,
            instances: RefCell::new(HashMap::new()),
            expanding: RefCell::new(Vec::new()),
        }
    }

    /// Reads each `sequence` definition. Nothing is expanded yet, so a
    /// sequence may use one defined after it.
    pub(crate) fn new(
        pairs: Vec<Pair<Rule>>,
        global_tempo: &Option<TempoMap>,
    ) -> Result<Self, Box<dyn Error>> {
        let mut templates = HashMap::new();
        for pair in pairs {
            let (line, col) = pair.line_col();
            let source = format!(
                "{}{}{}",
                "\n".repeat(line - 1),
                " ".repeat(col - 1),
                pair.as_str()
            );
            let mut name = String::new();
            let mut params: Vec<(String, Option<String>)> = Vec::new();
            for inner in pair.into_inner() {
                match inner.as_rule() {
                    Rule::sequence_name => name = inner.as_str().trim_matches('"').to_string(),
                    Rule::sequence_param_decls => {
                        for decl in inner.into_inner() {
                            let mut parts = decl.into_inner();
                            let param = parts.next().map(|p| p.as_str().to_string());
                            let default = parts.next().map(|p| p.as_str().to_string());
                            let Some(param) = param else { continue };
                            if params.iter().any(|(p, _)| *p == param) {
                                return Err(format!(
                                    "line {line}: Sequence '{name}' declares parameter '{param}' twice"
                                )
                                .into());
                            }
                            if default.as_deref().is_some_and(|d| d.starts_with('$')) {
                                return Err(format!(
                                    "line {line}: Sequence '{name}': the default for '{param}' \
                                     cannot be another parameter"
                                )
                                .into());
                            }
                            params.push((param, default));
                        }
                    }
                    _ => {}
                }
            }
            if let Some((offset, used)) = param_refs(&source)
                .into_iter()
                .find(|(_, used)| !params.iter().any(|(p, _)| p == used))
            {
                return Err(format!(
                    "line {}: Sequence '{name}' uses ${used}, but declares no parameter '{used}'",
                    line_at(&source, offset)
                )
                .into());
            }
            templates.insert(
                name,
                SequenceTemplate {
                    line,
                    params,
                    source,
                },
            );
        }
        Ok(SequenceLibrary {
            templates,
            global_tempo: global_tempo.clone(),
            instances: RefCell::new(HashMap::new()),
            expanding: RefCell::new(Vec::new()),
        })
    }

    /// Expands every sequence that needs no arguments, so an error in one is
    /// reported whether or not a show uses it. One with a required
    /// parameter can only be checked where it is used.
    pub(crate) fn check_all(&self) -> Result<(), Box<dyn Error>> {
        // In file order, so the first error reported is the first in the file.
        let mut names: Vec<&String> = self.templates.keys().collect();
        names.sort_by_key(|name| self.templates[*name].line);
        for name in names {
            let template = &self.templates[name];
            if !template.standalone() {
                continue;
            }
            self.resolve(name, &[]).map_err(|e| -> Box<dyn Error> {
                if template.params.is_empty() {
                    e
                } else {
                    format!("Sequence '{name}' with its default parameters: {e}").into()
                }
            })?;
        }
        Ok(())
    }

    /// The sequence a reference plays, expanded with the arguments it binds.
    pub(crate) fn instance(&self, call: &SequenceCall) -> Result<Rc<Sequence>, Box<dyn Error>> {
        let Some(template) = self.templates.get(&call.name) else {
            return Err(format!("line {}: Sequence '{}' not found", call.line, call.name).into());
        };
        if template.params.is_empty() && call.args.is_empty() {
            // Already checked on its own: an error here is a cycle, which
            // names its whole path.
            return self.resolve(&call.name, &call.args);
        }
        self.resolve(&call.name, &call.args).map_err(|e| {
            format!(
                "line {}: in sequence \"{}\" used here: {}",
                call.line, call.name, e
            )
            .into()
        })
    }

    fn resolve(
        &self,
        name: &str,
        args: &[(String, String)],
    ) -> Result<Rc<Sequence>, Box<dyn Error>> {
        let template = self
            .templates
            .get(name)
            .ok_or_else(|| format!("Sequence '{name}' not found"))?;
        let bindings = bind(name, template, args)?;

        let key = (name.to_string(), bindings);
        if let Some(sequence) = self.instances.borrow().get(&key) {
            return Ok(sequence.clone());
        }

        if self.expanding.borrow().iter().any(|n| n == name) {
            return Err(format!(
                "Circular sequence reference detected: {} -> {}",
                self.expanding.borrow().join(" -> "),
                name
            )
            .into());
        }
        self.expanding.borrow_mut().push(name.to_string());
        let expanded = self.expand(template, &key.1);
        self.expanding.borrow_mut().pop();

        let sequence = Rc::new(expanded?);
        self.instances.borrow_mut().insert(key, sequence.clone());
        Ok(sequence)
    }

    fn expand(
        &self,
        template: &SequenceTemplate,
        bindings: &Bindings,
    ) -> Result<Sequence, Box<dyn Error>> {
        let text = substitute(&template.source, bindings);
        let pair = LightingParser::parse(Rule::sequence_source, &text)
            .map_err(|e| {
                let (line, col) = match e.line_col {
                    pest::error::LineColLocation::Pos(pos) => pos,
                    pest::error::LineColLocation::Span(pos, _) => pos,
                };
                format!(
                    "DSL parsing error at line {line}, column {col} with the parameters \
                     substituted: {}",
                    e.variant.message()
                )
            })?
            .next()
            .and_then(|source| source.into_inner().find(|p| p.as_rule() == Rule::sequence))
            .ok_or("Sequence definition did not parse (internal error)")?;

        let definition = parse_sequence_structure(pair, &self.global_tempo)?;
        let tempo = definition
            .tempo_map
            .clone()
            .or_else(|| self.global_tempo.clone());
        let length = definition.declared_length(tempo.as_ref())?;
        let mut cues = Vec::new();
        for cue in definition.cues {
            cues.extend(expand_unexpanded_sequence_cue(cue, &tempo, self)?);
        }
        Ok(Sequence {
            cues,
            bpm: tempo
                .as_ref()
                .map(|tm| tm.initial_bpm)
                .unwrap_or(crate::lighting::tempo::DEFAULT_BPM),
            length,
        })
    }
}

/// The values a use of `name` binds: each default, overridden by the
/// arguments given, every parameter bound.
fn bind(
    name: &str,
    template: &SequenceTemplate,
    args: &[(String, String)],
) -> Result<Bindings, Box<dyn Error>> {
    let mut bindings = Bindings::new();
    for (param, value) in args {
        if !template.params.iter().any(|(p, _)| p == param) {
            let declared: Vec<&str> = template.params.iter().map(|(p, _)| p.as_str()).collect();
            return Err(if declared.is_empty() {
                format!("Sequence '{name}' takes no parameters, but is given '{param}'")
            } else {
                format!(
                    "Sequence '{name}' has no parameter '{param}' (it takes {})",
                    declared.join(", ")
                )
            }
            .into());
        }
        if bindings.insert(param.clone(), value.clone()).is_some() {
            return Err(format!("Sequence '{name}' is given '{param}' twice").into());
        }
    }
    let mut missing = Vec::new();
    for (param, default) in &template.params {
        if bindings.contains_key(param) {
            continue;
        }
        match default {
            Some(default) => {
                bindings.insert(param.clone(), default.clone());
            }
            None => missing.push(param.as_str()),
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "Sequence '{name}' needs a value for {}",
            missing
                .iter()
                .map(|p| format!("'{p}'"))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into());
    }
    Ok(bindings)
}

/// Each `$name` in `text` outside strings and comments, with its byte offset.
pub(crate) fn param_refs(text: &str) -> Vec<(usize, &str)> {
    let bytes = text.as_bytes();
    let mut refs = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += 1;
                }
                i += 1;
            }
            b'#' => i = end_of_line(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'/') => i = end_of_line(bytes, i),
            b'$' if bytes
                .get(i + 1)
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_') =>
            {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric()
                        || bytes[end] == b'_'
                        || bytes[end] == b'-')
                {
                    end += 1;
                }
                refs.push((i, &text[start..end]));
                i = end;
            }
            _ => i += 1,
        }
    }
    refs
}

fn end_of_line(bytes: &[u8], from: usize) -> usize {
    bytes[from..]
        .iter()
        .position(|b| *b == b'\n')
        .map_or(bytes.len(), |n| from + n)
}

/// The 1-based line of `text` that byte `offset` is on.
pub(crate) fn line_at(text: &str, offset: usize) -> usize {
    text[..offset].matches('\n').count() + 1
}

/// `text` with each `$name` replaced by its bound value.
fn substitute(text: &str, bindings: &Bindings) -> String {
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for (offset, name) in param_refs(text) {
        out.push_str(&text[copied..offset]);
        match bindings.get(name) {
            Some(value) => out.push_str(value),
            None => out.push_str(&text[offset..offset + 1 + name.len()]),
        }
        copied = offset + 1 + name.len();
    }
    out.push_str(&text[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_refs_skip_strings_and_comments() {
        let text = "a: $x, b: \"$y\" # $z\n c: $w-2 // $v\n$_u";
        let names: Vec<&str> = param_refs(text).into_iter().map(|(_, n)| n).collect();
        assert_eq!(names, vec!["x", "w-2", "_u"]);
    }

    #[test]
    fn substitute_replaces_bound_names() {
        let bindings: Bindings = [("hit".to_string(), "\"#FFD870\"".to_string())].into();
        assert_eq!(
            substitute("color: $hit, other: $hit2", &bindings),
            "color: \"#FFD870\", other: $hit2"
        );
    }

    #[test]
    fn line_at_counts_newlines() {
        assert_eq!(line_at("a\nb\nc", 0), 1);
        assert_eq!(line_at("a\nb\nc", 4), 3);
    }
}
