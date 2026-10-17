//! Parallel iteration over archetype-based queries using Rayon.
//!
//! [`ParallelQuery`] extends [`Query`] with a `par_iter` method that returns a
//! [`rayon::iter::ParallelIterator`]. Only **read-only** queries are supported
//! for parallel iteration to avoid data races: the [`ReadOnlyQueryData`] bound
//! on `Q` statically forbids mutable fetches from being distributed across
//! threads.

use super::QueryData;
use super::iter::Query;
use super::{NullFilter, QueryFilter, ReadOnlyQueryData, TickCtx};
use crate::archetype::Archetype;
use crate::world::World;
use rayon::iter::plumbing::UnindexedConsumer;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use rayon::slice::ParallelSlice;
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
            // SAFETY: `arch` satisfies `Q::matches` (filtered before constructing the
            // `ArchChunk`), and `row < row_count()` (checked above). `Q` is read-only
            // and the world is borrowed shared for `'w`, so concurrent workers only
            // create shared references and nothing can mutate the data meanwhile.
            return Some(unsafe { Q::fetch(self.arch, row) });
        }
    }
}

/// Rows handed to one Rayon task by [`Query::par_for_each_mut`].
const PAR_MUT_CHUNK: usize = 2048;

impl<Q, F> Query<Q, F>
where
    Q: QueryData,
    F: QueryFilter,
{
    /// Calls `f` for every matching entity, in parallel, with mutable access.
    ///
    /// Work is split by archetype and, inside an archetype, into chunks of rows, so a
    /// single large archetype is processed by many threads. Like
    /// [`Query::iter_mut`], this takes the world exclusively, rejects queries that
    /// alias a component mutably, and stamps the change tick of every mutated
    /// component on the visited rows (before `f` runs).
    ///
    /// # Panics
    /// If `Q` accesses the same component twice with at least one mutable access.
    pub fn par_for_each_mut<'w>(&self, world: &'w mut World, f: impl Fn(Q::Item<'w>) + Send + Sync)
    where
        Q::Item<'w>: Send,
    {
        Self::assert_no_aliasing();
        let ctx = TickCtx {
            frame_tick: world.frame_tick,
        };
        let stamp_tick = world.next_tick();
        let archetypes: &'w mut [Archetype] = &mut world.archetypes[..];

        // Sequential pre-pass: evaluate row filters against the *old* ticks, then stamp.
        let mut work: Vec<(usize, Vec<u32>)> = Vec::new();
        for (idx, arch) in archetypes.iter_mut().enumerate() {
            if !Q::matches(arch) || !F::matches(arch, ctx) {
                continue;
            }
            let rows: Vec<u32> = (0..arch.table.row_count())
                .filter(|&row| F::row_matches(arch, row, ctx))
                .map(|row| row as u32)
                .collect();
            if rows.is_empty() {
                continue;
            }
            Q::for_each_mutated(&mut |tid| {
                if let Some(col) = arch.table.column_mut(tid) {
                    for &row in &rows {
                        col.set_tick(row as usize, stamp_tick);
                    }
                }
            });
            work.push((idx, rows));
        }

        let archetypes: &'w [Archetype] = archetypes;
        work.par_iter().for_each(|(idx, rows)| {
            let arch = &archetypes[*idx];
            rows.par_chunks(PAR_MUT_CHUNK).for_each(|chunk| {
                for &row in chunk {
                    // SAFETY: `arch` satisfies `Q::matches` and every `row` is below
                    // its row count. The world is borrowed exclusively for `'w`, rows
                    // are unique (each appears once in exactly one chunk), and
                    // `assert_no_aliasing` guarantees no component is fetched mutably
                    // twice per row, so the yielded `&mut` references are disjoint
                    // across threads and rows.
                    let item = unsafe { Q::fetch(arch, row as usize) };
                    f(item);
                }
            });
        });
    }
}
