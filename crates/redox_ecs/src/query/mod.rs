//! Query system for iterating over components stored in archetypes.
//!
//! # Tuple queries
//!
//! The query system supports fetching several components at once via tuple types:
//!
//! ```rust,ignore
//! // Immutable read of two components
//! for (pos, vel) in world.query::<(&Position, &Velocity)>().iter(&world) { ... }
//!
//! // Mix of immutable and mutable references
//! for (pos, mut vel) in world.query::<(&Position, &mut Velocity)>().iter(&world) { ... }
//!
//! // With a filter
//! for (pos, vel) in world.query_filtered::<(&Position, &Velocity), With<Player>>().iter(&world) { ... }
//! ```

pub mod filter;
pub mod iter;
pub mod par_iter;

pub use filter::{QueryFilter, With, Without, NullFilter};
pub use iter::Query;
pub use par_iter::ParallelQuery;

use std::any::TypeId;
use crate::archetype::Archetype;

// ---------------------------------------------------------------------------
// QueryData – describes what data a query fetches
// ---------------------------------------------------------------------------

/// Trait implemented by types that can be fetched from a single archetype row.
///
/// Implemented for:
/// - `&T` and `&mut T` (single component references)
/// - tuples `(A, B)`, `(A, B, C)`, … up to 6 elements (via macro)
///
/// # Safety
/// Implementors must ensure that `fetch` returns references that are valid
/// for the lifetime `'w` tied to the archetype borrow.
pub unsafe trait QueryData {
    /// The type yielded by the iterator (e.g. `(&'w A, &'w B)`).
    type Item<'w>;

    /// Returns `true` if `archetype` contains every component needed by this query.
    fn matches(archetype: &Archetype) -> bool;

    /// Fetches the item for the given `row` from the archetype.
    ///
    /// # Safety
    /// - `archetype` must satisfy `matches(archetype) == true`.
    /// - `row` must be a valid row index.
    /// - The caller must uphold Rust's aliasing rules for any mutable references.
    unsafe fn fetch(archetype: &Archetype, row: usize) -> Self::Item<'_>;
}

// --- &T ---

unsafe impl<T: 'static + Send + Sync> QueryData for &T {
    type Item<'w> = &'w T;

    #[inline]
    fn matches(archetype: &Archetype) -> bool {
        archetype.table.has_component(TypeId::of::<T>())
    }

    #[inline]
    unsafe fn fetch(archetype: &Archetype, row: usize) -> Self::Item<'_> {
        unsafe {
            let col = archetype.table.columns.get(&TypeId::of::<T>()).unwrap_unchecked();
            &*(col.get(row) as *const T)
        }
    }
}

// --- &mut T ---

unsafe impl<T: 'static + Send + Sync> QueryData for &mut T {
    type Item<'w> = &'w mut T;

    #[inline]
    fn matches(archetype: &Archetype) -> bool {
        archetype.table.has_component(TypeId::of::<T>())
    }

    #[inline]
    unsafe fn fetch(archetype: &Archetype, row: usize) -> Self::Item<'_> {
        unsafe {
            let col = archetype.table.columns.get(&TypeId::of::<T>()).unwrap_unchecked();
            &mut *(col.get(row) as *mut T)
        }
    }
}

// ---------------------------------------------------------------------------
// Macro for tuple implementations (2 … 6 components)
// ---------------------------------------------------------------------------

macro_rules! impl_query_data_tuple {
    ($( $name:ident ),+) => {
        unsafe impl<$($name: QueryData),+> QueryData for ($($name,)+) {
            type Item<'w> = ($($name::Item<'w>,)+);

            #[inline]
            fn matches(archetype: &Archetype) -> bool {
                $( $name::matches(archetype) )&&+
            }

            #[inline]
            unsafe fn fetch(archetype: &Archetype, row: usize) -> Self::Item<'_> {
                unsafe { ( $( $name::fetch(archetype, row), )+ ) }
            }
        }
    };
}

impl_query_data_tuple!(A, B);
impl_query_data_tuple!(A, B, C);
impl_query_data_tuple!(A, B, C, D);
impl_query_data_tuple!(A, B, C, D, E);
impl_query_data_tuple!(A, B, C, D, E, F_);
