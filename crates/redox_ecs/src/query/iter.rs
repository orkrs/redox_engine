//! Sequential iteration over archetype-based queries.
//!
//! [`Query`] is the main entry point. It is generic over:
//! - `Q: QueryData` – the components to fetch (e.g. `(&Transform, &mut Velocity)`)
//! - `F: QueryFilter` – an optional archetype filter (defaults to [`NullFilter`])
//!
//! # Examples
//!
//! ```rust,ignore
//! // Single component (backward-compatible)
//! let q = Query::<&Position>::new();
//! for pos in q.iter(&world) { /* pos: &Position */ }
//!
//! // Tuple of components
//! let q = Query::<(&Position, &Velocity)>::new();
//! for (pos, vel) in q.iter(&world) { /* pos: &Position, vel: &Velocity */ }
//!
//! // Mutable fetches require exclusive world access
//! let q = Query::<(&Position, &mut Velocity)>::new();
//! for (pos, vel) in q.iter_mut(&mut world) { /* pos: &Position, vel: &mut Velocity */ }
//!
//! // With a filter
//! let q = Query::<(&Position, &Velocity), With<Player>>::new();
//! for (pos, vel) in q.iter(&world) { /* only entities that also have Player */ }
//! ```

use super::{NullFilter, QueryData, QueryFilter, ReadOnlyQueryData, TickCtx};
use crate::archetype::Archetype;
use crate::world::World;
use std::marker::PhantomData;

/// A sequential query over components stored in the [`World`].
///
/// `Q` describes what to fetch; `F` narrows which archetypes are visited.
pub struct Query<Q, F = NullFilter>
where
    Q: QueryData,
    F: QueryFilter,
{
    _marker: PhantomData<(Q, F)>,
}

impl<Q, F> Query<Q, F>
where
    Q: QueryData,
    F: QueryFilter,
{
    /// Creates a new query. Zero-cost; no allocation.
    #[inline]
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }

    /// Returns an iterator that yields one `Q::Item<'_>` per matching entity.
    ///
    /// Only available for **read-only** fetches; the world is borrowed shared.
    ///
    /// The iterator visits every archetype that:
    /// 1. contains all components required by `Q`, **and**
    /// 2. passes the additional filter `F`.
    ///
    /// Within each archetype the iteration is dense and cache-friendly.
    pub fn iter<'w>(&self, world: &'w World) -> QueryIter<'w, Q, F>
    where
        Q: ReadOnlyQueryData,
    {
        QueryIter {
            archetypes: &world.archetypes,
            ctx: TickCtx {
                frame_tick: world.frame_tick,
            },
            arch_index: 0,
            row: 0,
            _marker: PhantomData,
        }
    }

    /// Returns an iterator with mutable access to fetched components.
    ///
    /// Takes the world exclusively, so at most one mutable iterator can be live
    /// at a time — this is what makes `&mut T` items sound.
    ///
    /// Every row yielded stamps its change tick on all components that `Q`
    /// mutates, making them visible to [`Changed`](crate::query::filter::Changed).
    pub fn iter_mut<'w>(&self, world: &'w mut World) -> QueryIterMut<'w, Q, F> {
        let ctx = TickCtx {
            frame_tick: world.frame_tick,
        };
        // Bump the global tick so rows yielded by this iterator register as
        // changed relative to the frame-start snapshot.
        let stamp_tick = world.next_tick();
        QueryIterMut {
            archetypes: &mut world.archetypes[..],
            ctx,
            stamp_tick,
            arch_index: 0,
            row: 0,
            _marker: PhantomData,
        }
    }
}

impl<Q, F> Default for Query<Q, F>
where
    Q: QueryData,
    F: QueryFilter,
{
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// QueryIter – shared iteration (read-only fetches)
// ---------------------------------------------------------------------------

/// The iterator returned by [`Query::iter`].
pub struct QueryIter<'w, Q: ReadOnlyQueryData, F: QueryFilter> {
    archetypes: &'w [Archetype],
    ctx: TickCtx,
    arch_index: usize,
    row: usize,
    _marker: PhantomData<(Q, F)>,
}

impl<'w, Q: ReadOnlyQueryData, F: QueryFilter> Iterator for QueryIter<'w, Q, F> {
    type Item = Q::Item<'w>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Copy the shared slice reference out of `self` so the returned
            // items can borrow for `'w` rather than the `&mut self` call.
            let archetypes: &'w [Archetype] = self.archetypes;
            let arch = archetypes.get(self.arch_index)?;

            if !Q::matches(arch) || !F::matches(arch, self.ctx) {
                self.arch_index += 1;
                self.row = 0;
                continue;
            }

            let row_count = arch.table.row_count();
            while self.row < row_count {
                let row = self.row;
                self.row += 1;

                // Row-level filters (Changed/Added).
                if !F::row_matches(arch, row, self.ctx) {
                    continue;
                }

                // SAFETY:
                // - `arch` satisfies `Q::matches`, so all required columns exist.
                // - `row` is within bounds.
                // - `Q` is read-only and the world borrow is shared, so the
                //   yielded references cannot alias anything mutable.
                let item = unsafe { Q::fetch(arch, row) };
                return Some(item);
            }

            self.arch_index += 1;
            self.row = 0;
        }
    }
}

/// The iterator returned by [`Query::iter_mut`].
///
/// Holds exclusive provenance over the world's archetypes for `'w` (captured
/// as a raw pointer from the `&'w mut World` taken by [`Query::iter_mut`]);
/// yields items tied to the same lifetime. Rows yielded are stamped as changed
/// on every component type that `Q` mutates.
///
/// # Safety invariant
/// The pointer is derived from an exclusive borrow that is consumed by
/// [`Query::iter_mut`], so nothing else can access the archetypes during `'w`.
/// The iterator's own `&mut self` methods never touch the pointee.
pub struct QueryIterMut<'w, Q: QueryData, F: QueryFilter> {
    archetypes: *mut [Archetype],
    ctx: TickCtx,
    stamp_tick: u32,
    arch_index: usize,
    row: usize,
    _marker: PhantomData<(&'w mut [Archetype], (Q, F))>,
}

impl<'w, Q: QueryData, F: QueryFilter> Iterator for QueryIterMut<'w, Q, F> {
    type Item = Q::Item<'w>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // SAFETY: see the struct-level safety invariant; the pointee is
            // exclusively owned by this iterator for `'w` and `&mut self`
            // aliases nothing (the raw pointer field is never read as a
            // reference outside this deref).
            let archetypes: &'w mut [Archetype] = unsafe { &mut *self.archetypes };
            let arch = archetypes.get_mut(self.arch_index)?;

            if !Q::matches(arch) || !F::matches(arch, self.ctx) {
                self.arch_index += 1;
                self.row = 0;
                continue;
            }

            let row_count = arch.table.row_count();
            while self.row < row_count {
                let row = self.row;
                self.row += 1;

                if !F::row_matches(arch, row, self.ctx) {
                    continue;
                }

                // Stamp change ticks on mutated component types before handing
                // out references (the stamp does not alias the value bytes).
                let stamp_tick = self.stamp_tick;
                Q::for_each_mutated(&mut |tid| {
                    if let Some(col) = arch.table.column_mut(tid) {
                        col.set_tick(row, stamp_tick);
                    }
                });

                let arch_ptr: *mut Archetype = arch;
                // SAFETY: `arch_ptr` points to an archetype that satisfies
                // `Q::matches` with a valid in-bounds `row`; the exclusive
                // borrow of the world guarantees aliasing rules hold.
                let item = unsafe { Q::fetch_mut(&mut *arch_ptr, row) };
                return Some(item);
            }

            self.arch_index += 1;
            self.row = 0;
        }
    }
}
