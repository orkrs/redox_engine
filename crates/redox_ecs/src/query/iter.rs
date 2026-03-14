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
//! // With a filter
//! let q = Query::<(&Position, &mut Velocity), With<Player>>::new();
//! for (pos, vel) in q.iter(&world) { /* only entities that also have Player */ }
//! ```

use std::marker::PhantomData;
use crate::world::World;
use crate::archetype::Archetype;
use super::{QueryData, QueryFilter, NullFilter};

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
        Self { _marker: PhantomData }
    }

    /// Returns an iterator that yields one `Q::Item<'_>` per matching entity.
    ///
    /// The iterator visits every archetype that:
    /// 1. contains all components required by `Q`, **and**
    /// 2. passes the additional filter `F`.
    ///
    /// Within each archetype the iteration is dense and cache-friendly.
    pub fn iter<'w>(&self, world: &'w World) -> QueryIter<'w, Q, F> {
        QueryIter {
            archetypes: &world.archetypes,
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
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// QueryIter
// ---------------------------------------------------------------------------

/// The iterator returned by [`Query::iter`].
pub struct QueryIter<'w, Q: QueryData, F: QueryFilter> {
    archetypes: &'w [Archetype],
    arch_index: usize,
    row: usize,
    _marker: PhantomData<(Q, F)>,
}

impl<'w, Q: QueryData, F: QueryFilter> Iterator for QueryIter<'w, Q, F> {
    type Item = Q::Item<'w>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let arch = self.archetypes.get(self.arch_index)?;

            if !Q::matches(arch) || !F::matches(arch) {
                self.arch_index += 1;
                self.row = 0;
                continue;
            }

            let row_count = arch.table.row_count();
            if self.row >= row_count {
                self.arch_index += 1;
                self.row = 0;
                continue;
            }

            let row = self.row;
            self.row += 1;

            // SAFETY:
            // - `arch` satisfies `Q::matches`, so all required columns exist.
            // - `row` is within bounds (checked above).
            // - We borrow `arch` through `'w`, which ties the returned
            //   references to the world borrow, preventing mutation while
            //   iteration is in progress.
            let item = unsafe { Q::fetch(arch, row) };
            return Some(item);
        }
    }
}
