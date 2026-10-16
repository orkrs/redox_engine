//! Parallel iteration over archetype-based queries using Rayon.
//!
//! [`ParallelQuery`] extends [`Query`] with a `par_iter` method that returns a
//! [`rayon::iter::ParallelIterator`]. Only **read-only** queries are supported
//! for parallel iteration to avoid data races: the [`ReadOnlyQueryData`] bound
//! on `Q` statically forbids mutable fetches from being distributed across
//! threads.

use super::iter::Query;
use super::{NullFilter, QueryFilter, ReadOnlyQueryData, TickCtx};
use crate::archetype::Archetype;
use crate::world::World;
use rayon::iter::plumbing::UnindexedConsumer;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use std::marker::PhantomData;

// ---------------------------------------------------------------------------
// Public trait
// ---------------------------------------------------------------------------

/// Extension trait that adds parallel iteration to a [`Query`].
pub trait ParallelQuery<'w, Q, F = NullFilter>
where
    Q: ReadOnlyQueryData + Send,
    Q::Item<'w>: Send,
    F: QueryFilter + Send,
{
    /// Returns a parallel iterator over all matching entities.
    fn par_iter(&self, world: &'w World) -> QueryParIter<'w, Q, F>;
}

impl<'w, Q, F> ParallelQuery<'w, Q, F> for Query<Q, F>
where
    Q: ReadOnlyQueryData + Send,
    Q::Item<'w>: Send,
    F: QueryFilter + Send,
{
    fn par_iter(&self, world: &'w World) -> QueryParIter<'w, Q, F> {
        QueryParIter {
            archetypes: &world.archetypes,
            ctx: TickCtx {
                frame_tick: world.frame_tick,
            },
            _marker: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------------
// QueryParIter – top-level parallel iterator
// ---------------------------------------------------------------------------

/// The parallel iterator returned by [`ParallelQuery::par_iter`].
///
/// Parallelism is achieved at the archetype level: each matching archetype
/// is processed as an independent chunk by Rayon.
pub struct QueryParIter<'w, Q: ReadOnlyQueryData, F: QueryFilter> {
    archetypes: &'w [Archetype],
    ctx: TickCtx,
    _marker: PhantomData<(Q, F)>,
}

// SAFETY: `QueryParIter` holds only a shared slice reference and PhantomData.
// The query types `Q` and `F` are zero-sized marker types; we only ever
// access the data through shared references to `Archetype`, and `Q` is
// statically guaranteed read-only.
unsafe impl<'w, Q: ReadOnlyQueryData + Send, F: QueryFilter + Send> Send
    for QueryParIter<'w, Q, F>
{
}

impl<'w, Q, F> ParallelIterator for QueryParIter<'w, Q, F>
where
    Q: ReadOnlyQueryData + Send,
    Q::Item<'w>: Send,
    F: QueryFilter + Send,
{
    type Item = Q::Item<'w>;

    fn drive_unindexed<C>(self, consumer: C) -> C::Result
    where
        C: UnindexedConsumer<Self::Item>,
    {
        // Collect matching archetype slices upfront (cheap; just references).
        let chunks: Vec<ArchChunk<'w, Q, F>> = self
            .archetypes
            .iter()
            .filter(|arch| {
                // Whole-archetype filter first; row-level filters (Changed/Added)
                // are evaluated per row inside ArchRowIter.
                Q::matches(arch) && F::matches(arch, self.ctx)
            })
            .map(|arch| ArchChunk {
                arch,
                ctx: self.ctx,
                _marker: PhantomData,
            })
            .collect();

        chunks
            .into_par_iter()
            .flat_map_iter(|chunk| ArchRowIter {
                arch: chunk.arch,
                ctx: chunk.ctx,
                row: 0,
                _marker: PhantomData::<(Q, F)>,
            })
            .drive_unindexed(consumer)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Wraps a single matching archetype for use as a Rayon work item.
struct ArchChunk<'w, Q: ReadOnlyQueryData, F: QueryFilter> {
    arch: &'w Archetype,
    ctx: TickCtx,
    _marker: PhantomData<(Q, F)>,
}

unsafe impl<'w, Q: ReadOnlyQueryData + Send, F: QueryFilter + Send> Send for ArchChunk<'w, Q, F> {}

/// A sequential iterator over a single archetype's rows (used inside Rayon workers).
struct ArchRowIter<'w, Q: ReadOnlyQueryData, F: QueryFilter> {
    arch: &'w Archetype,
    ctx: TickCtx,
    row: usize,
    _marker: PhantomData<(Q, F)>,
}

impl<'w, Q, F> Iterator for ArchRowIter<'w, Q, F>
where
    Q: ReadOnlyQueryData,
    F: QueryFilter,
{
    type Item = Q::Item<'w>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.row >= self.arch.table.row_count() {
                return None;
            }
            let row = self.row;
            self.row += 1;
            if !F::row_matches(self.arch, row, self.ctx) {
                continue;
            }
            // SAFETY: arch matches Q (checked before constructing ArchChunk), row in bounds.
            return Some(unsafe { Q::fetch(self.arch, row) });
        }
    }
}
