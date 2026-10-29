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
    /// On its way to 0 from [`MidiDmxStore::fade_out`] -- not from a dim
    /// MIDI asked for, which a value alone cannot tell apart. A write ends it.
    store_fade: AtomicBool,
    /// Bumped by every MIDI write, before it marks the slot active: a
    /// slot let go once its fade to dark is out stays MIDI's if this moved
    /// meanwhile.
    writes: AtomicU64,
}

impl Slot {
    fn new() -> Self {
        Self {
            target: AtomicU64::new(0f64.to_bits()),
            current: AtomicU64::new(0f64.to_bits()),
            rate: AtomicU64::new(0f64.to_bits()),
            active: AtomicBool::new(false),
            store_fade: AtomicBool::new(false),
            writes: AtomicU64::new(0),
        }
    }
}

/// Whether `slot` is still on its way to 0 from [`MidiDmxStore::fade_out`].
fn fading_out(slot: &Slot) -> bool {
    slot.store_fade.load(Ordering::Acquire) && load_f64(&slot.current) > 0.0
}

impl Default for MidiDmxStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Lockless store for MIDI DMX values. MIDI events write atomically;
/// the effects loop calls `tick()` each frame to interpolate, then reads
/// interpolated values for injection into the EffectEngine.
pub struct MidiDmxStore {
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

impl MidiDmxStore {
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

    /// Registers a slot for a fixture channel. Called during fixture
    /// registration, which happens at every song start and every venue
    /// reload, so it is idempotent: a channel already registered keeps its
    /// slot (and any value MIDI is holding on it) and only its fixture
    /// mapping is refreshed. Returns the slot index.
    pub fn register_slot(
        &mut self,
        universe: u16,
        dmx_channel: u16,
        fixture_name: &str,
        channel_name: &str,
    ) -> usize {
        if let Some(&slot_index) = self.channel_to_slot.get(&(universe, dmx_channel)) {
            self.slot_to_fixture[slot_index] = (fixture_name.to_string(), channel_name.to_string());
            return slot_index;
        }
        let slot_index = self.slots.len();
        self.slots.push(Slot::new());
        self.channel_to_slot
            .insert((universe, dmx_channel), slot_index);
        self.slot_to_fixture
            .push((fixture_name.to_string(), channel_name.to_string()));
        slot_index
    }

    /// Drops the slots of channels the venue no longer patches, keeping
    /// (and re-indexing) the rest with their values. Called before the
    /// venue's fixtures are re-registered, so a fixture unhung between two
    /// reloads does not linger in the effects loop's per-frame scan.
    pub fn retain_channels(&mut self, keep: impl Fn(u16, u16) -> bool) {
        let old_slots = std::mem::take(&mut self.slots);
        let old_map = std::mem::take(&mut self.slot_to_fixture);
        let mut old_index_by_slot: Vec<Option<(u16, u16)>> = vec![None; old_slots.len()];
        for (&channel, &index) in &self.channel_to_slot {
            old_index_by_slot[index] = Some(channel);
        }
        self.channel_to_slot.clear();
        for ((slot, mapping), channel) in old_slots.into_iter().zip(old_map).zip(old_index_by_slot)
        {
            let Some((universe, dmx_channel)) = channel else {
                continue;
            };
            if !keep(universe, dmx_channel) {
                continue;
            }
            self.channel_to_slot
                .insert((universe, dmx_channel), self.slots.len());
            self.slots.push(slot);
            self.slot_to_fixture.push(mapping);
        }
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// How many channels have slots.
    pub fn slot_count(&self) -> usize {
        self.slots.len()
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
            // MIDI's again, whatever a fade to dark was doing.
            slot.store_fade.store(false, Ordering::Release);

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

            // Counted before it is active: a release that let the slot go
            // in between sees the count move, and takes it back.
            slot.writes.fetch_add(1, Ordering::SeqCst);
            slot.active.store(true, Ordering::SeqCst);
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
            let writes = slot.writes.load(Ordering::SeqCst);
            let current = load_f64(&slot.current);
            let target = load_f64(&slot.target);
            let rate = load_f64(&slot.rate);
            // A fade to dark that reached 0 a frame ago, its 0 out: the slot
            // is MIDI's no more, as a clear leaves it -- held at 0, it would
            // be a base under the next song's effects (a multiply over it
            // black).
            if slot.store_fade.load(Ordering::Acquire) && current == 0.0 && target == 0.0 {
                slot.store_fade.store(false, Ordering::Release);
                slot.active.store(false, Ordering::SeqCst);
                // A MIDI write that landed meanwhile -- of 0 too, a
                // blackout cue -- keeps its slot.
                if slot.writes.load(Ordering::SeqCst) != writes {
                    slot.active.store(true, Ordering::SeqCst);
                }
                any_changed = true;
                continue;
            }

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

    /// Fades every slot MIDI holds to 0 over `ticks` frames, from where it
    /// is: every engine renders the store, so each sees the same value go
    /// down -- nothing cut, nothing crossed twice. A MIDI write after takes
    /// its channel back.
    pub fn fade_out(&self, ticks: f64) {
        let mut any = false;
        for (slot, (_, channel)) in self.slots.iter().zip(&self.slot_to_fixture) {
            if !slot.active.load(Ordering::Acquire) || fading_out(slot) {
                // A fade under way goes on as it is, not restarted slower.
                continue;
            }
            if !crate::lighting::engine::fades_to_dark(channel) {
                // A position, gobo, wheel or mode stays as MIDI set it: run
                // down to 0 it would swing, step through every slot, or pass
                // a lamp-off or reset on the way.
                continue;
            }
            let current = load_f64(&slot.current);
            store_f64(&slot.target, 0.0);
            store_f64(&slot.rate, -current / ticks.max(1.0));
            slot.store_fade.store(true, Ordering::Release);
            any = true;
        }
        if any {
            self.generation.fetch_add(1, Ordering::Release);
        }
    }

    /// Resets all slots to inactive (called on song transition) -- but for
    /// one fading out (see [`Self::fade_out`]), which goes on to 0: cleared,
    /// nothing would write its channel again, and the rig would hold it
    /// where the fade had got to.
    pub fn clear(&self) {
        for slot in &self.slots {
            if fading_out(slot) {
                continue;
            }
            store_f64(&slot.current, 0.0);
            store_f64(&slot.target, 0.0);
            store_f64(&slot.rate, 0.0);
            slot.store_fade.store(false, Ordering::Release);
            // Release: ensures zeroed values are visible before active is seen as false.
            slot.active.store(false, Ordering::Release);
        }
        self.generation.fetch_add(1, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store with one slot at (1, 1), dimming over `ticks` frames.
    fn one_slot(ticks: f64) -> MidiDmxStore {
        let mut store = MidiDmxStore::new();
        store.register_slot(1, 1, "par", "dimmer");
        store.register_universe(1);
        store.set_dim_rate(1, ticks);
        store
    }

    fn value(store: &MidiDmxStore) -> f64 {
        store.iter_active().next().map_or(0.0, |(_, v)| v * 255.0)
    }

    #[test]
    fn a_dim_midi_asked_for_is_not_taken_for_a_fade_to_dark() {
        // A slow dim to 0 from MIDI looked like the store's own fade: a
        // song start let it run on under the next song, and a fade to dark
        // left it to its own pace.
        let store = one_slot(1.0);
        store.write(1, 1, 200, false);
        store.tick();
        store.set_dim_rate(1, 440.0);
        store.write(1, 1, 0, true);
        store.tick();
        assert!(value(&store) > 150.0, "dimming down slowly");
        store.fade_out(44.0);
        for _ in 0..45 {
            store.tick();
        }
        assert_eq!(value(&store), 0.0, "dark in the fade's time");

        store.write(1, 1, 200, false);
        store.tick();
        store.write(1, 1, 0, true);
        store.tick();
        store.clear();
        assert_eq!(store.iter_active().count(), 0, "a song start cuts it");
    }

    #[test]
    fn a_fade_to_dark_takes_levels_down_and_leaves_the_rest() {
        // Run down to 0, a mover's pan swung and a mode channel passed
        // whatever its low values do.
        let mut store = MidiDmxStore::new();
        store.register_slot(1, 1, "mover", "dimmer");
        store.register_slot(1, 2, "mover", "pan");
        store.register_slot(1, 3, "mover", "mode");
        store.register_universe(1);
        for channel in 1..=3 {
            store.write(1, channel, 200, false);
        }
        store.tick();
        store.fade_out(10.0);
        for _ in 0..20 {
            store.tick();
        }
        let values: Vec<f64> = store.iter_active().map(|(_, v)| v * 255.0).collect();
        assert_eq!(values, vec![200.0, 200.0], "pan and mode as they were");
    }

    #[test]
    fn a_fade_to_dark_knows_levels_by_any_name_they_come_in() {
        // Section-suffixed GDTF channels, GDTF's amber and lime as their
        // attributes come through, and colour temperature were not taken
        // for levels, and stayed lit.
        let names = [
            "dimmer:background",
            "red:ring",
            "coloradd_ry",
            "coloradd_gy",
            "color_temp",
            "indigo",
            "zoom",
        ];
        let mut store = MidiDmxStore::new();
        for (i, name) in names.iter().enumerate() {
            store.register_slot(1, i as u16 + 1, "fixture", name);
        }
        store.register_universe(1);
        for i in 0..names.len() {
            store.write(1, i as u16 + 1, 200, false);
        }
        store.tick();
        store.fade_out(10.0);
        for _ in 0..20 {
            store.tick();
        }
        let left: Vec<&str> = store
            .iter_active()
            .map(|(slot, _)| store.fixture_info(slot).1.as_str())
            .collect();
        assert_eq!(left, vec!["zoom"], "only the optic left as it was");
    }

    #[test]
    fn a_fade_to_dark_lets_its_slot_go_once_out() {
        // Held at 0 after a fade to dark that ran through a song start, the
        // slot stayed MIDI's: a base of 0 under the next song's effects.
        let store = one_slot(1.0);
        store.write(1, 1, 200, false);
        store.tick();
        store.fade_out(10.0);
        store.clear();
        for _ in 0..10 {
            store.tick();
        }
        assert_eq!(store.iter_active().count(), 1, "its 0 goes out once");
        assert_eq!(value(&store), 0.0);
        store.tick();
        assert_eq!(store.iter_active().count(), 0, "then it is let go");
    }

    #[test]
    fn a_fade_to_dark_runs_through_a_clear_and_a_write_takes_it_back() {
        let store = one_slot(1.0);
        store.write(1, 1, 200, false);
        store.tick();
        store.fade_out(44.0);
        for _ in 0..10 {
            store.tick();
        }
        store.clear();
        assert!(value(&store) > 0.0, "still on its way down");
        for _ in 0..40 {
            store.tick();
        }
        assert_eq!(value(&store), 0.0);

        store.write(1, 1, 200, false);
        store.tick();
        store.fade_out(44.0);
        store.tick();
        store.write(1, 1, 120, false);
        store.tick();
        for _ in 0..50 {
            store.tick();
        }
        assert_eq!(value(&store), 120.0, "MIDI's again");
    }

    #[test]
    fn re_registering_a_channel_reuses_its_slot_and_keeps_its_value() {
        let mut store = MidiDmxStore::new();
        let first = store.register_slot(1, 1, "A", "red");
        store.write(1, 1, 200, false);
        let again = store.register_slot(1, 1, "A", "red");
        assert_eq!(first, again);
        assert_eq!(store.slot_count(), 1);
        assert!(store.has_active_slots(), "the held MIDI value survives");
        // A different fixture on the same channel takes the mapping over.
        store.register_slot(1, 1, "B", "dimmer");
        assert_eq!(
            store.fixture_info(first),
            &("B".to_string(), "dimmer".to_string())
        );
    }

    #[test]
    fn retaining_channels_drops_the_rest_and_reindexes() {
        let mut store = MidiDmxStore::new();
        store.register_slot(1, 1, "A", "red");
        store.register_slot(1, 2, "A", "green");
        store.register_slot(1, 5, "B", "red");
        store.write(1, 5, 100, false);
        store.tick();
        store.retain_channels(|_, channel| channel != 2);
        assert_eq!(store.slot_count(), 2);
        assert_eq!(store.lookup(1, 2), None);
        let b = store.lookup(1, 5).expect("B's channel kept");
        assert_eq!(store.fixture_info(b), &("B".to_string(), "red".to_string()));
        assert!(
            store
                .iter_active()
                .any(|(index, value)| index == b && value > 0.0),
            "B's held value survives the re-index"
        );
    }

    fn create_test_store() -> MidiDmxStore {
        let mut store = MidiDmxStore::new();
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

    #[test]
    fn test_generation_increments_on_write() {
        let store = create_test_store();
        let gen0 = store.generation();

        store.write(1, 10, 100, false);
        let gen1 = store.generation();
        assert!(gen1 > gen0);

        store.write(1, 11, 50, false);
        let gen2 = store.generation();
        assert!(gen2 > gen1);
    }

    #[test]
    fn test_generation_increments_on_tick_change() {
        let store = create_test_store();
        store.set_dim_rate(1, 10.0);
        store.write(1, 10, 200, true);

        let gen_before = store.generation();
        let changed = store.tick();
        let gen_after = store.generation();

        assert!(changed);
        assert!(gen_after > gen_before);
    }

    #[test]
    fn test_generation_increments_on_clear() {
        let store = create_test_store();
        store.write(1, 10, 100, false);
        let gen_before = store.generation();

        store.clear();
        let gen_after = store.generation();
        assert!(gen_after > gen_before);
    }

    #[test]
    fn test_fixture_info() {
        let store = create_test_store();

        let (fixture, channel) = store.fixture_info(0);
        assert_eq!(fixture, "wash1");
        assert_eq!(channel, "dimmer");

        let (fixture, channel) = store.fixture_info(2);
        assert_eq!(fixture, "wash1");
        assert_eq!(channel, "green");
    }

    #[test]
    fn test_tick_no_active_slots_returns_false() {
        let store = create_test_store();
        assert!(!store.tick());
    }

    #[test]
    fn test_tick_converged_slots_returns_false() {
        let store = create_test_store();
        store.write(1, 10, 100, false);
        store.tick(); // First tick sets to target

        // Already at target — tick should return false
        assert!(!store.tick());
    }

    #[test]
    fn test_write_to_unmapped_channel_is_noop() {
        let store = create_test_store();
        let gen_before = store.generation();

        store.write(2, 99, 255, false); // non-existent universe/channel

        // Nothing should change
        assert_eq!(store.generation(), gen_before);
        assert!(!store.has_active_slots());
    }

    #[test]
    fn test_default_trait() {
        let store = MidiDmxStore::default();
        assert_eq!(store.iter_active().count(), 0);
        assert_eq!(store.generation(), 0);
    }

    #[test]
    fn test_multiple_universes() {
        let mut store = MidiDmxStore::new();
        store.register_slot(1, 1, "fixture_a", "dimmer");
        store.register_slot(2, 1, "fixture_b", "dimmer");
        store.register_universe(1);
        store.register_universe(2);

        store.write(1, 1, 100, false);
        store.write(2, 1, 200, false);
        store.tick();

        let values: Vec<(usize, f64)> = store.iter_active().collect();
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn test_instant_write_zero_dim_rate() {
        let store = create_test_store();
        store.set_dim_rate(1, 0.0);

        store.write(1, 10, 128, true); // dim=true but rate=0
        store.tick();

        let values: Vec<(usize, f64)> = store.iter_active().collect();
        assert_eq!(values.len(), 1);
        // Should be at target immediately (rate was 0)
        assert!((values[0].1 - 128.0 / 255.0).abs() < 0.01);
    }

    #[test]
    fn test_dim_down() {
        let store = create_test_store();
        store.set_dim_rate(1, 44.0);

        // Set to high value first
        store.write(1, 10, 200, false);
        store.tick();

        // Now dim down
        store.write(1, 10, 50, true);

        // After several ticks, should be between current and target
        for _ in 0..22 {
            store.tick();
        }

        let values: Vec<(usize, f64)> = store.iter_active().collect();
        let normalized = values[0].1;
        // Should be between 50/255 and 200/255
        assert!(normalized > 50.0 / 255.0 - 0.05);
        assert!(normalized < 200.0 / 255.0 + 0.05);
    }

    #[test]
    fn test_lookup_returns_correct_slot_index() {
        let store = create_test_store();

        assert_eq!(store.lookup(1, 10), Some(0));
        assert_eq!(store.lookup(1, 11), Some(1));
        assert_eq!(store.lookup(1, 12), Some(2));
        assert_eq!(store.lookup(1, 13), Some(3));
    }
}
