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

//! A file's palette: named colour slots with a default colour each, and the
//! `palette.<slot>` references effects make to them.
//!
//! The parser keeps a reference to a slot rather than only the colour it
//! stands for (`ColorRef`), so the colour can be settled again before a song
//! plays (`resolve_shows`): the show's defaults, with an override for any
//! slot replacing its default. Resolving happens once, ahead of the first
//! frame; the engine only ever sees the literal colours that leaves in the
//! effects.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;

use pest::iterators::Pair;

use super::super::effects::{Color, EffectType};
use super::grammar::Rule;
use super::types::{Effect, LightShow};
use super::utils::{parse_color_parameter, parse_color_string};

/// The word that begins a reference to a slot: `palette.accent`.
pub const REFERENCE_PREFIX: &str = "palette.";

/// A palette's slots, each with its colour, in the order declared.
pub type Palette = indexmap::IndexMap<String, Color>;

/// A colour an effect was written with: its own value, or whichever a
/// palette slot is.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorRef {
    Literal(Color),
    Slot(String),
}

/// The slot a written value refers to (`palette.accent` is `accent`), or
/// `None` for a value that is not a reference.
pub fn slot_of(value: &str) -> Option<&str> {
    value
        .trim()
        .strip_prefix(REFERENCE_PREFIX)
        .filter(|slot| !slot.is_empty())
}

/// `color` as the DSL writes it, which the colour parser reads back.
pub fn hex(color: &Color) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
}

/// The palette a `palette { }` block declares, or why it does not read.
pub(super) fn read(pair: Pair<Rule>) -> Result<Palette, Box<dyn Error>> {
    let mut palette = Palette::new();
    for slot in pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::palette_slot)
    {
        let (line, _) = slot.line_col();
        let mut parts = slot.into_inner();
        let name = parts
            .next()
            .map(|p| p.as_str().to_string())
            .unwrap_or_default();
        let written = match parts.find(|p| p.as_rule() == Rule::color_parameter) {
            Some(value) => parse_color_parameter(value)?,
            None => String::new(),
        };
        let color = parse_color_string(&written).ok_or_else(|| {
            format!(
                "line {line}: palette slot '{name}': invalid color '{}' (expected #RRGGBB, \
                 rgb(r, g, b) or a color name)",
                written.trim_matches('"')
            )
        })?;
        if palette.insert(name.clone(), color).is_some() {
            return Err(format!("line {line}: the palette declares '{name}' twice").into());
        }
    }
    Ok(palette)
}

impl Effect {
    /// Settles the colours this effect took from palette slots against
    /// `palette`, a slot it does not name left as it is.
    pub fn resolve_palette(&mut self, palette: &Palette) {
        for (index, color) in self.color_refs.iter().enumerate() {
            let ColorRef::Slot(slot) = color else {
                continue;
            };
            let Some(&color) = palette.get(slot) else {
                continue;
            };
            match &mut self.effect_type {
                EffectType::ColorCycle { colors, .. } => {
                    if let Some(at) = colors.get_mut(index) {
                        *at = color;
                    }
                }
                EffectType::Static { parameters, .. } => {
                    parameters.insert("red".to_string(), f64::from(color.r) / 255.0);
                    parameters.insert("green".to_string(), f64::from(color.g) / 255.0);
                    parameters.insert("blue".to_string(), f64::from(color.b) / 255.0);
                }
                _ => {}
            }
        }
    }

    /// The palette slots this effect's colours refer to.
    pub fn palette_slots(&self) -> impl Iterator<Item = &str> {
        self.color_refs.iter().filter_map(|c| match c {
            ColorRef::Slot(slot) => Some(slot.as_str()),
            ColorRef::Literal(_) => None,
        })
    }
}

impl LightShow {
    /// The show with each slot in `overrides` settled to its colour in
    /// place of the show's default, and every other slot at its default.
    /// An override for a slot the show does not declare is ignored. With
    /// no overrides the show is as parsed, which already holds its
    /// defaults.
    pub fn resolve_palette(&mut self, overrides: &HashMap<String, Color>) {
        let mut palette = self.palette.clone();
        let mut changed = false;
        for (slot, color) in overrides {
            if let Some(default) = palette.get_mut(slot) {
                *default = *color;
                changed = true;
            }
        }
        if !changed {
            return;
        }
        for cue in &mut self.cues {
            for effect in &mut cue.effects {
                effect.resolve_palette(&palette);
            }
        }
        self.palette = palette;
    }
}

/// `shows` with their palettes resolved (see [`LightShow::resolve_palette`]):
/// a slot is a show's own default unless `overrides` names it, so an
/// override names a slot, not a file, and recolours it in every show that
/// declares it.
pub fn resolve_shows(
    mut shows: Vec<LightShow>,
    overrides: &HashMap<String, Color>,
) -> Vec<LightShow> {
    for show in &mut shows {
        show.resolve_palette(overrides);
    }
    shows
}

/// Every slot the shows declare, with its default; the first show to
/// declare a slot sets the colour listed.
pub fn declared_slots<'a>(shows: impl IntoIterator<Item = &'a LightShow>) -> Palette {
    let mut slots = Palette::new();
    for show in shows {
        for (slot, color) in &show.palette {
            slots.entry(slot.clone()).or_insert(*color);
        }
    }
    slots
}

/// The colours a song or playlist entry overrides, as written in its YAML
/// (slot to colour string), read with the DSL's colour parser, so
/// `"#2040ff"`, `rgb(…)` and the names all work. A colour that does not
/// read is an error naming its slot.
pub fn parse_overrides(
    written: &BTreeMap<String, String>,
) -> Result<HashMap<String, Color>, String> {
    written
        .iter()
        .map(|(slot, value)| {
            parse_color_string(value.trim())
                .map(|color| (slot.clone(), color))
                .ok_or_else(|| {
                    format!(
                        "palette slot '{slot}': invalid color '{value}' (expected #RRGGBB, \
                         rgb(r, g, b) or a color name)"
                    )
                })
        })
        .collect()
}

/// `later` laid over `earlier`, slot by slot: the colour a slot is given
/// last wins.
pub fn layer_overrides(
    earlier: &HashMap<String, Color>,
    later: Option<&HashMap<String, Color>>,
) -> HashMap<String, Color> {
    let mut layered = earlier.clone();
    if let Some(later) = later {
        layered.extend(later.iter().map(|(slot, color)| (slot.clone(), *color)));
    }
    layered
}

/// The slots `overrides` names that none of `shows` declares, sorted.
pub fn unknown_slots<'a, 'b>(
    shows: impl IntoIterator<Item = &'a LightShow>,
    overrides: impl IntoIterator<Item = &'b String>,
) -> Vec<String> {
    let declared = declared_slots(shows);
    let mut unknown: Vec<String> = overrides
        .into_iter()
        .filter(|slot| !declared.contains_key(*slot))
        .cloned()
        .collect();
    unknown.sort();
    unknown.dedup();
    unknown
}
