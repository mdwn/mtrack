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

//! Names that are compared without being read.
//!
//! A frame of a light show is a few hundred values, each for a channel of a
//! fixture, and for years of this engine each was found by hashing the
//! channel's name: `"red"`, `"dimmer"`, `"pan"`. A [`Name`] is a name held
//! once for the life of the process, so two of them are the same name
//! exactly when they are the same pointer: comparing costs a comparison,
//! and nothing is hashed or copied to pass one around. A [`NameMap`] is
//! the few values a fixture has by channel, found by walking them.
//!
//! A string becomes a `Name` where it enters the engine -- a show parsed, a
//! fixture registered -- and a `Name` is read as a string where one leaves
//! it. Asking by string still works everywhere ([`AsName`]), at the price
//! of looking the string up; the frame's own work asks by `Name`.
//!
//! The names held are never freed. They are the channel and fixture names
//! of the venues and shows a process has loaded: a few hundred short
//! strings, a few thousand for a process left running through a season of
//! edits.

use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, RwLock, RwLockReadGuard};

/// A name, held once: equal to another exactly when it is the same one.
#[derive(Clone, Copy)]
pub struct Name(&'static str);

/// The names the engine itself asks for, each held from the start so that
/// asking for one costs nothing.
macro_rules! known {
    ($($ident:ident = $text:literal),* $(,)?) => {
        $(
            #[doc = concat!("The name `", $text, "`.")]
            pub static $ident: Name = Name($text);
        )*
        /// Every name held from the start.
        static KNOWN: &[&Name] = &[$(&$ident),*];
    };
}

known! {
    RED = "red",
    GREEN = "green",
    BLUE = "blue",
    WHITE = "white",
    DIMMER = "dimmer",
    STROBE = "strobe",
    CYAN = "cyan",
    MAGENTA = "magenta",
    YELLOW = "yellow",
    PAN = "pan",
    TILT = "tilt",
    INTENSITY = "_intensity",
}

static HELD: LazyLock<RwLock<HashSet<&'static str>>> =
    LazyLock::new(|| RwLock::new(KNOWN.iter().map(|name| name.0).collect()));

/// The names held, to read.
fn held() -> RwLockReadGuard<'static, HashSet<&'static str>> {
    HELD.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Name {
    /// The name `text`, held from now on if it was not already.
    pub fn of(text: &str) -> Name {
        if let Some(held) = held().get(text) {
            return Name(held);
        }
        // A panic while the set was held leaves it whole: a name is put in
        // by one insert.
        let mut held = HELD
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(held) = held.get(text) {
            return Name(held);
        }
        let kept: &'static str = Box::leak(text.to_string().into_boxed_str());
        held.insert(kept);
        Name(kept)
    }

    /// The name `text` if it is held: `None` says nothing anywhere is
    /// called that, so nothing keyed by a name can hold it. Holds nothing.
    pub fn find(text: &str) -> Option<Name> {
        held().get(text).map(|held| Name(held))
    }

    pub fn as_str(self) -> &'static str {
        self.0
    }
}

impl PartialEq for Name {
    #[inline]
    fn eq(&self, other: &Name) -> bool {
        std::ptr::eq(self.0.as_ptr(), other.0.as_ptr()) && self.0.len() == other.0.len()
    }
}

impl Eq for Name {}

impl Hash for Name {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(self.0.as_ptr() as usize);
    }
}

/// By the text, so a list of names sorts as the strings would.
impl Ord for Name {
    fn cmp(&self, other: &Name) -> std::cmp::Ordering {
        self.0.cmp(other.0)
    }
}

impl PartialOrd for Name {
    fn partial_cmp(&self, other: &Name) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::ops::Deref for Name {
    type Target = str;
    fn deref(&self) -> &str {
        self.0
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        self.0
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.0, f)
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl PartialEq<str> for Name {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Name {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for Name {
    fn eq(&self, other: &String) -> bool {
        self.0 == other.as_str()
    }
}

impl PartialEq<Name> for String {
    fn eq(&self, other: &Name) -> bool {
        self.as_str() == other.0
    }
}

impl PartialEq<Name> for &str {
    fn eq(&self, other: &Name) -> bool {
        *self == other.0
    }
}

impl From<Name> for String {
    fn from(name: Name) -> String {
        name.0.to_string()
    }
}

impl From<&str> for Name {
    fn from(text: &str) -> Name {
        Name::of(text)
    }
}

impl From<&String> for Name {
    fn from(text: &String) -> Name {
        Name::of(text)
    }
}

impl From<String> for Name {
    fn from(text: String) -> Name {
        Name::of(&text)
    }
}

/// Something a value can be asked for by: a [`Name`], which costs nothing,
/// or a string, which is looked up.
pub trait AsName {
    /// The name, if it is held ([`Name::find`]).
    fn find_name(&self) -> Option<Name>;
    /// The name, held from now on ([`Name::of`]).
    fn to_name(&self) -> Name;
}

impl AsName for Name {
    #[inline]
    fn find_name(&self) -> Option<Name> {
        Some(*self)
    }
    #[inline]
    fn to_name(&self) -> Name {
        *self
    }
}

impl AsName for str {
    fn find_name(&self) -> Option<Name> {
        Name::find(self)
    }
    fn to_name(&self) -> Name {
        Name::of(self)
    }
}

impl AsName for String {
    fn find_name(&self) -> Option<Name> {
        Name::find(self)
    }
    fn to_name(&self) -> Name {
        Name::of(self)
    }
}

impl<T: AsName + ?Sized> AsName for &T {
    #[inline]
    fn find_name(&self) -> Option<Name> {
        (**self).find_name()
    }
    #[inline]
    fn to_name(&self) -> Name {
        (**self).to_name()
    }
}

/// A few values by name, found by walking them: a fixture's channels. The
/// calls are a map's, and take a [`Name`] or a string ([`AsName`]). Kept in
/// the order they were put in.
#[derive(Clone)]
pub struct NameMap<V> {
    entries: Vec<(Name, V)>,
}

impl<V> Default for NameMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> NameMap<V> {
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    #[inline]
    fn at(&self, name: Name) -> Option<usize> {
        self.entries.iter().position(|(held, _)| *held == name)
    }

    #[inline]
    pub fn get(&self, name: impl AsName) -> Option<&V> {
        let at = self.at(name.find_name()?)?;
        Some(&self.entries[at].1)
    }

    #[inline]
    pub fn get_mut(&mut self, name: impl AsName) -> Option<&mut V> {
        let at = self.at(name.find_name()?)?;
        Some(&mut self.entries[at].1)
    }

    #[inline]
    pub fn contains_key(&self, name: impl AsName) -> bool {
        name.find_name().is_some_and(|name| self.at(name).is_some())
    }

    /// Puts `value` at `name`, answering what was there.
    #[inline]
    pub fn insert(&mut self, name: impl AsName, value: V) -> Option<V> {
        let name = name.to_name();
        match self.at(name) {
            Some(at) => Some(std::mem::replace(&mut self.entries[at].1, value)),
            None => {
                self.entries.push((name, value));
                None
            }
        }
    }

    /// The value at `name`, put there by `make` if there was none.
    pub fn get_or_insert_with(&mut self, name: impl AsName, make: impl FnOnce() -> V) -> &mut V {
        let name = name.to_name();
        let at = match self.at(name) {
            Some(at) => at,
            None => {
                self.entries.push((name, make()));
                self.entries.len() - 1
            }
        };
        &mut self.entries[at].1
    }

    pub fn remove(&mut self, name: impl AsName) -> Option<V> {
        let at = self.at(name.find_name()?)?;
        Some(self.entries.remove(at).1)
    }

    pub fn retain(&mut self, mut keep: impl FnMut(Name, &mut V) -> bool) {
        self.entries.retain_mut(|(name, value)| keep(*name, value));
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (Name, &V)> + Clone {
        self.entries.iter().map(|(name, value)| (*name, value))
    }

    pub fn iter_mut(&mut self) -> impl ExactSizeIterator<Item = (Name, &mut V)> {
        self.entries.iter_mut().map(|(name, value)| (*name, value))
    }

    pub fn keys(&self) -> impl ExactSizeIterator<Item = Name> + Clone + '_ {
        self.entries.iter().map(|(name, _)| *name)
    }

    pub fn values(&self) -> impl ExactSizeIterator<Item = &V> + Clone {
        self.entries.iter().map(|(_, value)| value)
    }

    pub fn values_mut(&mut self) -> impl ExactSizeIterator<Item = &mut V> {
        self.entries.iter_mut().map(|(_, value)| value)
    }
}

impl<V> std::ops::Index<&str> for NameMap<V> {
    type Output = V;
    fn index(&self, name: &str) -> &V {
        self.get(name)
            .unwrap_or_else(|| panic!("nothing is held as {name:?}"))
    }
}

impl<V> std::ops::Index<Name> for NameMap<V> {
    type Output = V;
    fn index(&self, name: Name) -> &V {
        self.get(name)
            .unwrap_or_else(|| panic!("nothing is held as {name:?}"))
    }
}

impl<K: AsName, V> Extend<(K, V)> for NameMap<V> {
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, entries: I) {
        for (name, value) in entries {
            self.insert(name, value);
        }
    }
}

impl<K: AsName, V> FromIterator<(K, V)> for NameMap<V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(entries: I) -> Self {
        let mut map = NameMap::new();
        map.extend(entries);
        map
    }
}

impl<K: AsName, V, const N: usize> From<[(K, V); N]> for NameMap<V> {
    fn from(entries: [(K, V); N]) -> Self {
        entries.into_iter().collect()
    }
}

impl<K: AsName, V, S> From<std::collections::HashMap<K, V, S>> for NameMap<V> {
    fn from(map: std::collections::HashMap<K, V, S>) -> Self {
        map.into_iter().collect()
    }
}

impl<V> IntoIterator for NameMap<V> {
    type Item = (Name, V);
    type IntoIter = std::vec::IntoIter<(Name, V)>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl<'a, V> IntoIterator for &'a NameMap<V> {
    type Item = (Name, &'a V);
    type IntoIter =
        std::iter::Map<std::slice::Iter<'a, (Name, V)>, fn(&'a (Name, V)) -> (Name, &'a V)>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter().map(|(name, value)| (*name, value))
    }
}

/// The same values at the same names, whatever order they went in.
impl<V: PartialEq> PartialEq for NameMap<V> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .all(|(name, value)| other.get(name) == Some(value))
    }
}

impl<V: Eq> Eq for NameMap<V> {}

/// As a map, by name, so two that are equal print the same.
impl<V: fmt::Debug> fmt::Debug for NameMap<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut sorted: Vec<&(Name, V)> = self.entries.iter().collect();
        sorted.sort_by_key(|(name, _)| *name);
        f.debug_map()
            .entries(sorted.into_iter().map(|(name, value)| (name, value)))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_the_same_one_however_it_is_asked_for() {
        let asked = Name::of("names-test-front-wash");
        assert_eq!(asked, Name::of(&String::from("names-test-front-wash")));
        assert_eq!(Name::find("names-test-front-wash"), Some(asked));
        assert_eq!(asked.as_str(), "names-test-front-wash");
        assert_ne!(asked, Name::of("names-test-front-wash "));
        assert_eq!(Name::find("names-test-never-held"), None);
    }

    /// The names held from the start are the ones asking by their text
    /// finds, on every thread at once.
    #[test]
    fn a_known_name_is_the_one_its_text_finds() {
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    assert_eq!(Name::of("red"), RED);
                    assert_eq!(Name::find("dimmer"), Some(DIMMER));
                    assert_eq!(Name::of("names-test-raced"), Name::of("names-test-raced"));
                });
            }
        });
        assert_ne!(RED, GREEN);
        assert_eq!(RED, "red");
    }

    #[test]
    fn names_sort_as_their_text_does() {
        let mut names = [Name::of("pan"), BLUE, Name::of("Zoom"), DIMMER];
        names.sort();
        assert_eq!(names.map(Name::as_str), ["Zoom", "blue", "dimmer", "pan"]);
    }

    #[test]
    fn a_map_is_asked_by_name_or_by_text() {
        let mut map: NameMap<u8> = NameMap::new();
        assert_eq!(map.insert("red", 1), None);
        assert_eq!(map.insert(GREEN, 2), None);
        assert_eq!(map.insert(String::from("red"), 3), Some(1));
        assert_eq!((map.get(RED), map.get("green")), (Some(&3), Some(&2)));
        assert_eq!(map["red"], 3);
        assert!(map.contains_key(&String::from("green")));
        // Asking for a name nothing holds finds nothing, and holds nothing.
        assert_eq!(map.get("names-test-asked-only"), None);
        assert_eq!(Name::find("names-test-asked-only"), None);
        *map.get_or_insert_with(BLUE, || 0) += 9;
        assert_eq!(map.remove("green"), Some(2));
        assert_eq!(map.keys().collect::<Vec<_>>(), [RED, BLUE]);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn maps_are_equal_whatever_order_they_were_filled_in() {
        let one: NameMap<u8> = [("red", 1), ("blue", 2)].into();
        let other: NameMap<u8> = [("blue", 2), ("red", 1)].into();
        assert_eq!(one, other);
        assert_eq!(format!("{one:?}"), format!("{other:?}"));
        assert_ne!(one, NameMap::from([("red", 1)]));
        assert_ne!(one, NameMap::from([("red", 1), ("blue", 3)]));
    }
}
