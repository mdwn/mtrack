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

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Stores an f64 in an AtomicU64 via bit representation.
fn store_f64(atom: &AtomicU64, val: f64) {
    atom.store(val.to_bits(), Ordering::Relaxed);
}

/// Loads an f64 from an AtomicU64 via bit representation.
fn load_f64(atom: &AtomicU64) -> f64 {
    f64::from_bits(atom.load(Ordering::Relaxed))
}

/// A single channel slot with lockless atomic storage for target, current
/// (interpolated), and rate values.
struct Slot {
    /// Target value (0.0–255.0) set by MIDI writes.
    target: AtomicU64,
    /// Current interpolated value (0.0–255.0), approaches target each tick.
    current: AtomicU64,
    /// Approach rate per tick (positive = dimming up, negative = dimming down, 0 = instant).
    rate: AtomicU64,
    /// Whether this slot has been written to at least once.
    active: AtomicBool,
}

impl Slot {
    fn new() -> Self {
        Self {
            target: AtomicU64::new(0f64.to_bits()),
            current: AtomicU64::new(0f64.to_bits()),
            rate: AtomicU64::new(0f64.to_bits()),
            active: AtomicBool::new(false),
        }
    }
}

/// Lockless store for legacy MIDI DMX values. MIDI events write atomically;
/// the effects loop calls `tick()` each frame to interpolate, then reads
/// interpolated values for injection into the EffectEngine.
impl Default for LegacyDmxStore {
    fn default() -> Self {
        Self::new()
    }
}

pub struct LegacyDmxStore {
    /// Pre-allocated slots indexed by position.
    slots: Vec<Slot>,
    /// (universe_id, dmx_channel) → slot index.
    channel_to_slot: HashMap<(u16, u16), usize>,
    /// slot index → (fixture_name, channel_name) for EffectEngine injection.
    slot_to_fixture: Vec<(String, String)>,
    /// Per-universe dim rate (number of ticks for a full 0→255 transition).
    /// Stored as AtomicU64 holding f64 bits.
    dim_rates: HashMap<u16, AtomicU64>,
    /// Generation counter — incremented on every write() or clear().
    /// Used by the EffectEngine to detect when recomputation is needed.
    generation: AtomicU32,
    /// True while any slot has current != target (dimming in progress).
    /// Set by tick(), cleared when all slots have converged.
    interpolating: AtomicBool,
}

impl LegacyDmxStore {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            channel_to_slot: HashMap::new(),
            slot_to_fixture: Vec::new(),
            dim_rates: HashMap::new(),
            generation: AtomicU32::new(0),
            interpolating: AtomicBool::new(false),
        }
    }

    /// Registers a slot for a fixture channel. Called during fixture registration.
    /// Returns the slot index.
    pub fn register_slot(
        &mut self,
        universe: u16,
        dmx_channel: u16,
        fixture_name: &str,
        channel_name: &str,
    ) -> usize {
        let slot_index = self.slots.len();
        self.slots.push(Slot::new());
        self.channel_to_slot
            .insert((universe, dmx_channel), slot_index);
        self.slot_to_fixture
            .push((fixture_name.to_string(), channel_name.to_string()));
        slot_index
    }

    /// Ensures a dim_rate entry exists for the given universe.
    pub fn register_universe(&mut self, universe_id: u16) {
        self.dim_rates
            .entry(universe_id)
            .or_insert_with(|| AtomicU64::new(1.0f64.to_bits()));
    }

    /// Writes a value from the MIDI thread. Lockless (atomics only).
    pub fn write(&self, universe: u16, channel: u16, value: u8, dim: bool) {
        if let Some(&slot_index) = self.channel_to_slot.get(&(universe, channel)) {
            let slot = &self.slots[slot_index];
            let value_f64 = f64::from(value);
            store_f64(&slot.target, value_f64);

            if dim {
                let current = load_f64(&slot.current);
                let dim_rate = self.dim_rates.get(&universe).map(load_f64).unwrap_or(1.0);
                let rate = if dim_rate > f64::EPSILON {
                    (value_f64 - current) / dim_rate
                } else {
                    0.0 // instant
                };
                store_f64(&slot.rate, rate);
            } else {
                store_f64(&slot.rate, 0.0); // instant
            }

            // Release: ensures target/rate writes are visible to readers
            // that Acquire on active or generation.
            slot.active.store(true, Ordering::Release);
            self.generation.fetch_add(1, Ordering::Release);
        }
    }

    /// Sets the dim rate for a universe (number of ticks for a full transition).
    pub fn set_dim_rate(&self, universe: u16, rate_ticks: f64) {
        if let Some(dim_rate) = self.dim_rates.get(&universe) {
            store_f64(dim_rate, rate_ticks);
        }
    }

    /// Interpolates all active slots one tick toward their targets.
    /// Called each frame from the effects loop (~44Hz).
    /// Returns true if any slot value actually changed this tick.
    pub fn tick(&self) -> bool {
        let mut any_changed = false;
        let mut still_interpolating = false;
        for slot in &self.slots {
            // Acquire: pairs with Release in write() to see latest target/rate.
            if !slot.active.load(Ordering::Acquire) {
                continue;
            }
            let current = load_f64(&slot.current);
            let target = load_f64(&slot.target);
            let rate = load_f64(&slot.rate);

            let new_current = if rate > f64::EPSILON {
                (current + rate).min(target)
            } else if rate < -f64::EPSILON {
                (current + rate).max(target)
            } else {
                target // instant (rate == 0)
            };

            if (new_current - current).abs() > f64::EPSILON {
                any_changed = true;
            }
            if (new_current - target).abs() > f64::EPSILON {
                still_interpolating = true;
            }
            store_f64(&slot.current, new_current);
        }
        self.interpolating
            .store(still_interpolating, Ordering::Relaxed);
        if any_changed {
            // Release: ensures updated current values are visible to readers
            // that Acquire on generation.
            self.generation.fetch_add(1, Ordering::Release);
        }
        any_changed
    }

    /// Returns the current generation counter.
    /// Used by EffectEngine to detect when recomputation can be skipped.
    pub fn generation(&self) -> u32 {
        // Acquire: pairs with Release in write()/tick()/clear() to see
        // all slot data that was written before the generation bump.
        self.generation.load(Ordering::Acquire)
    }

    /// Returns true if any slot has been written to.
    #[cfg(test)]
    pub fn has_active_slots(&self) -> bool {
        self.slots.iter().any(|s| s.active.load(Ordering::Relaxed))
    }

    /// Iterates over active slots, yielding (slot_index, normalized_value 0.0–1.0).
    pub fn iter_active(&self) -> impl Iterator<Item = (usize, f64)> + '_ {
        self.slots.iter().enumerate().filter_map(|(i, slot)| {
            // Acquire: pairs with Release in write() to see latest slot data.
            if slot.active.load(Ordering::Acquire) {
                let current = load_f64(&slot.current);
                Some((i, current / 255.0))
            } else {
                None
            }
        })
    }

    /// Returns the (fixture_name, channel_name) for a slot index.
    pub fn fixture_info(&self, slot_index: usize) -> &(String, String) {
        &self.slot_to_fixture[slot_index]
    }

    /// Checks if a (universe, channel) is registered in the store.
    pub fn lookup(&self, universe: u16, channel: u16) -> Option<usize> {
        self.channel_to_slot.get(&(universe, channel)).copied()
    }

    /// Resets all slots to inactive (called on song transition).
    pub fn clear(&self) {
        for slot in &self.slots {
            store_f64(&slot.current, 0.0);
            store_f64(&slot.target, 0.0);
            store_f64(&slot.rate, 0.0);
            // Release: ensures zeroed values are visible before active is seen as false.
            slot.active.store(false, Ordering::Release);
        }
        self.generation.fetch_add(1, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_store() -> LegacyDmxStore {
        let mut store = LegacyDmxStore::new();
        store.register_slot(1, 10, "wash1", "dimmer");
        store.register_slot(1, 11, "wash1", "red");
        store.register_slot(1, 12, "wash1", "green");
        store.register_slot(1, 13, "wash1", "blue");
        store.register_universe(1);
        store
    }

    #[test]
    fn test_write_and_read_instant() {
        let store = create_test_store();

        // Write with dim=false → instant
        store.write(1, 10, 200, false);
        store.tick();

        // Current should equal target immediately
        let values: Vec<(usize, f64)> = store.iter_active().collect();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].0, 0); // slot index 0 = dimmer
        assert!((values[0].1 - 200.0 / 255.0).abs() < 0.01);
    }

    #[test]
    fn test_write_and_read_dimmed() {
        let store = create_test_store();

        // Set dim rate to 44 ticks (1 second at 44Hz)
        store.set_dim_rate(1, 44.0);

        // Write with dim=true
        store.write(1, 10, 220, true);

        // After 22 ticks, should be roughly halfway
        for _ in 0..22 {
            store.tick();
        }

        let values: Vec<(usize, f64)> = store.iter_active().collect();
        assert_eq!(values.len(), 1);
        let normalized = values[0].1;
        // Halfway to 220/255 ≈ 0.431
        assert!(
            normalized > 0.3 && normalized < 0.6,
            "expected roughly halfway, got {}",
            normalized
        );

        // After 44 total ticks, should be at target
        for _ in 0..22 {
            store.tick();
        }

        let values: Vec<(usize, f64)> = store.iter_active().collect();
        let normalized = values[0].1;
        assert!(
            (normalized - 220.0 / 255.0).abs() < 0.02,
            "expected ~0.863, got {}",
            normalized
        );
    }

    #[test]
    fn test_dim_rate_change() {
        let store = create_test_store();

        store.set_dim_rate(1, 44.0);
        store.write(1, 10, 200, true);

        // Tick a few times
        for _ in 0..10 {
            store.tick();
        }

        // Change dim rate and write a new value
        store.set_dim_rate(1, 88.0);
        store.write(1, 11, 100, true);

        // Tick more - both should be interpolating
        for _ in 0..34 {
            store.tick();
        }

        let values: Vec<(usize, f64)> = store.iter_active().collect();
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn test_clear_resets_slots() {
        let store = create_test_store();

        store.write(1, 10, 200, false);
        store.write(1, 11, 100, false);
        store.tick();

        assert!(store.has_active_slots());

        store.clear();

        assert!(!store.has_active_slots());
        assert_eq!(store.iter_active().count(), 0);
    }

    #[test]
    fn test_unmapped_channel_returns_none() {
        let store = create_test_store();

        // Channel 99 is not registered
        assert!(store.lookup(1, 99).is_none());
        // Wrong universe
        assert!(store.lookup(2, 10).is_none());
        // Valid lookup
        assert!(store.lookup(1, 10).is_some());
    }
}
