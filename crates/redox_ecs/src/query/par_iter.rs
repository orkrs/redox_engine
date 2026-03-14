//! Parallel iteration over archetype-based queries using Rayon.
//!
//! [`ParallelQuery`] extends [`Query`] with a `par_iter` method that returns a
//! [`rayon::iter::ParallelIterator`]. Only **read-only** queries are supported
//! for parallel iteration to avoid data races.
//!
//! # Example
//!
//! ```rust,ignore
//! use rayon::iter::ParallelIterator;
//!
//! let q = Query::<(&Position, &Velocity)>::new();
//! q.par_iter(&world).for_each(|(pos, vel)| {
//!     // read pos and vel in parallel
//! });
//! ```

use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rayon::iter::plumbing::UnindexedConsumer;
use std::marker::PhantomData;
use crate::world::World;
use crate::archetype::Archetype;
use super::{QueryData, QueryFilter, NullFilter};
use super::iter::Query;

// ---------------------------------------------------------------------------
// Public trait
// ---------------------------------------------------------------------------

/// Extension trait that adds parallel iteration to a [`Query`].
pub trait ParallelQuery<'w, Q, F = NullFilter>
where
    Q: QueryData + Send,
    Q::Item<'w>: Send,
    F: QueryFilter + Send,
{
    /// Returns a parallel iterator over all matching entities.
    fn par_iter(&self, world: &'w World) -> QueryParIter<'w, Q, F>;
}

impl<'w, Q, F> ParallelQuery<'w, Q, F> for Query<Q, F>
where
    Q: QueryData + Send,
    Q::Item<'w>: Send,
    F: QueryFilter + Send,
{
    fn par_iter(&self, world: &'w World) -> QueryParIter<'w, Q, F> {
        QueryParIter {
            archetypes: &world.archetypes,
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
pub struct QueryParIter<'w, Q: QueryData, F: QueryFilter> {
    archetypes: &'w [Archetype],
    _marker: PhantomData<(Q, F)>,
}

// SAFETY: `QueryParIter` holds only a shared slice reference and PhantomData.
// The query types `Q` and `F` are zero-sized marker types; we only ever
// access the data through shared references to `Archetype`.
unsafe impl<'w, Q: QueryData + Send, F: QueryFilter + Send> Send for QueryParIter<'w, Q, F> {}

impl<'w, Q, F> ParallelIterator for QueryParIter<'w, Q, F>
where
    Q: QueryData + Send,
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
            .filter(|arch| Q::matches(arch) && F::matches(arch))
            .map(|arch| ArchChunk {
                arch,
                _marker: PhantomData,
            })
            .collect();

        chunks
            .into_par_iter()
            .flat_map_iter(|chunk| ArchRowIter {
                arch: chunk.arch,
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
struct ArchChunk<'w, Q: QueryData, F: QueryFilter> {
    arch: &'w Archetype,
    _marker: PhantomData<(Q, F)>,
}

unsafe impl<'w, Q: QueryData + Send, F: QueryFilter + Send> Send for ArchChunk<'w, Q, F> {}

/// A sequential iterator over a single archetype's rows (used inside Rayon workers).
struct ArchRowIter<'w, Q: QueryData, F: QueryFilter> {
    arch: &'w Archetype,
    row: usize,
    _marker: PhantomData<(Q, F)>,
}

impl<'w, Q, F> Iterator for ArchRowIter<'w, Q, F>
where
    Q: QueryData,
    F: QueryFilter,
{
    type Item = Q::Item<'w>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.row >= self.arch.table.row_count() {
            return None;
        }
        let row = self.row;
        self.row += 1;
        // SAFETY: arch matches Q (checked before constructing ArchChunk), row in bounds.
        Some(unsafe { Q::fetch(self.arch, row) })
    }
}
