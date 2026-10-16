//! Query system for iterating over components stored in archetypes.
//!
//! # Tuple queries
//!
//! The query system supports fetching several components at once via tuple types:
//!
//! ```rust,ignore
//! // Immutable read of two components (shared borrow of the world)
//! for (pos, vel) in world.query::<(&Position, &Velocity)>().iter(&world) { ... }
//!
//! // Mutable access requires an exclusive borrow of the world
//! for (pos, vel) in world.query::<(&Position, &mut Velocity)>().iter_mut(&mut world) { ... }
//!
//! // With a filter
//! for (pos, vel) in world.query_filtered::<(&Position, &Velocity), With<Player>>().iter(&world) { ... }
//! ```
//!
//! # Soundness
//!
//! [`Query::iter`] only accepts **read-only** fetches ([`ReadOnlyQueryData`])
//! and borrows the world shared. Fetches containing `&mut T` must go through
//! [`Query::iter_mut`], which takes the world exclusively — making it
//! impossible to obtain two aliasing `&mut` references to the same component.

pub mod filter;
pub mod iter;
pub mod par_iter;

pub use filter::{NullFilter, QueryFilter, TickCtx, With, Without};
pub use iter::Query;
pub use par_iter::ParallelQuery;

use crate::archetype::Archetype;
use crate::entity::Entity;
use std::any::TypeId;

// ---------------------------------------------------------------------------
// QueryData – describes what data a query fetches
// ---------------------------------------------------------------------------

/// Trait implemented by types that can be fetched from a single archetype row.
///
/// Implemented for:
/// - `&T` and `&mut T` (single component references)
/// - [`Entity`] (row identity)
/// - tuples `(A, B)`, `(A, B, C)`, … up to 8 elements (via macro)
///
/// # Safety
/// Implementors must ensure that `fetch`/`fetch_mut` return references that are
/// valid for the lifetime tied to the archetype borrow. For mutable-fetch
/// implementations, `fetch_mut` additionally requires the caller to hold
/// exclusive access to the archetype.
pub unsafe trait QueryData {
    /// The type yielded by the iterator (e.g. `(&'w A, &'w B)`).
    type Item<'w>;

    /// Returns `true` if `archetype` contains every component needed by this query.
    fn matches(archetype: &Archetype) -> bool;

    /// Calls `f` with the [`TypeId`] of every component this fetch can mutate.
    /// Used by [`Query::iter_mut`](crate::query::iter::Query::iter_mut) to stamp
    /// change ticks on the rows it yields.
    fn for_each_mutated(f: &mut dyn FnMut(TypeId));

    /// Fetches the item for the given `row` from the archetype (shared access).
    ///
    /// # Safety
    /// - `archetype` must satisfy `matches(archetype) == true`.
    /// - `row` must be a valid row index.
    /// - Mutable-fetch implementations require the caller to uphold Rust's
    ///   aliasing rules (exclusive access) — prefer `fetch_mut`.
    unsafe fn fetch(archetype: &Archetype, row: usize) -> Self::Item<'_>;

    /// Fetches the item for the given `row` with exclusive archetype access.
    /// This is the entry point used by mutable iteration.
    ///
    /// # Safety
    /// - Same as [`QueryData::fetch`]; the exclusive `&mut Archetype` proves
    ///   that no other reference to any row of this archetype is live.
    unsafe fn fetch_mut(archetype: &mut Archetype, row: usize) -> Self::Item<'_>;
}

/// Marker trait for fetches that never yield mutable references (`&T`,
/// [`Entity`], tuples thereof). Read-only queries may iterate with a shared
/// world borrow via [`Query::iter`](crate::query::iter::Query::iter).
pub trait ReadOnlyQueryData: QueryData {}

// --- &T ---

unsafe impl<T: 'static + Send + Sync> QueryData for &T {
    type Item<'w> = &'w T;

    #[inline]
    fn matches(archetype: &Archetype) -> bool {
        archetype.table.has_component(TypeId::of::<T>())
    }

    #[inline]
    fn for_each_mutated(_f: &mut dyn FnMut(TypeId)) {}

    #[inline]
    unsafe fn fetch(archetype: &Archetype, row: usize) -> Self::Item<'_> {
        unsafe {
            let col = archetype
                .table
                .columns
                .get(&TypeId::of::<T>())
                .unwrap_unchecked();
            &*(col.get(row) as *const T)
        }
    }

    #[inline]
    unsafe fn fetch_mut(archetype: &mut Archetype, row: usize) -> Self::Item<'_> {
        unsafe { Self::fetch(&*archetype, row) }
    }
}

impl<T: 'static + Send + Sync> ReadOnlyQueryData for &T {}

// --- &mut T ---

unsafe impl<T: 'static + Send + Sync> QueryData for &mut T {
    type Item<'w> = &'w mut T;

    #[inline]
    fn matches(archetype: &Archetype) -> bool {
        archetype.table.has_component(TypeId::of::<T>())
    }

    #[inline]
    fn for_each_mutated(f: &mut dyn FnMut(TypeId)) {
        f(TypeId::of::<T>());
    }

    #[inline]
    unsafe fn fetch(archetype: &Archetype, row: usize) -> Self::Item<'_> {
        // SAFETY: caller guarantees exclusive access; documented as such.
        unsafe {
            let col = archetype
                .table
                .columns
                .get(&TypeId::of::<T>())
                .unwrap_unchecked();
            &mut *(col.get(row) as *mut T)
        }
    }

    #[inline]
    unsafe fn fetch_mut(archetype: &mut Archetype, row: usize) -> Self::Item<'_> {
        unsafe {
            let col = archetype
                .table
                .columns
                .get(&TypeId::of::<T>())
                .unwrap_unchecked();
            &mut *(col.get(row) as *mut T)
        }
    }
}

// --- Entity (row identity, always available) ---

unsafe impl QueryData for Entity {
    type Item<'w> = Entity;

    #[inline]
    fn matches(_archetype: &Archetype) -> bool {
        // Every archetype stores entities; an empty world simply yields nothing.
        true
    }

    #[inline]
    fn for_each_mutated(_f: &mut dyn FnMut(TypeId)) {}

    #[inline]
    unsafe fn fetch(archetype: &Archetype, row: usize) -> Self::Item<'_> {
        archetype.entities()[row]
    }

    #[inline]
    unsafe fn fetch_mut(archetype: &mut Archetype, row: usize) -> Self::Item<'_> {
        archetype.entities()[row]
    }
}

impl ReadOnlyQueryData for Entity {}

// ---------------------------------------------------------------------------
// Macro for tuple implementations (2 … 8 components)
// ---------------------------------------------------------------------------

macro_rules! impl_query_data_tuple {
    ($( $name:ident ),+) => {
        #[allow(non_snake_case)]
        unsafe impl<$($name: QueryData),+> QueryData for ($($name,)+) {
            type Item<'w> = ($($name::Item<'w>,)+);

            #[inline]
            fn matches(archetype: &Archetype) -> bool {
                $( $name::matches(archetype) )&&+
            }

            #[inline]
            fn for_each_mutated(f: &mut dyn FnMut(TypeId)) {
                $( $name::for_each_mutated(f); )+
            }

            #[inline]
            unsafe fn fetch(archetype: &Archetype, row: usize) -> Self::Item<'_> {
                unsafe { ( $( $name::fetch(archetype, row), )+ ) }
            }

            // SAFETY: each element of the tuple fetches a *distinct* component
            // type, and distinct component types live in distinct columns, so
            // the derived `&mut` references are pairwise disjoint.
            #[inline]
            unsafe fn fetch_mut(archetype: &mut Archetype, row: usize) -> Self::Item<'_> {
                let arch_ptr: *mut Archetype = archetype;
                unsafe { ( $( $name::fetch_mut(&mut *arch_ptr, row), )+ ) }
            }
        }

        impl<$($name: ReadOnlyQueryData),+> ReadOnlyQueryData for ($($name,)+) {}
    };
}

impl_query_data_tuple!(A, B);
impl_query_data_tuple!(A, B, C);
impl_query_data_tuple!(A, B, C, D);
impl_query_data_tuple!(A, B, C, D, E);
impl_query_data_tuple!(A, B, C, D, E, F_);
impl_query_data_tuple!(A, B, C, D, E, F_, G);
impl_query_data_tuple!(A, B, C, D, E, F_, G, H);
