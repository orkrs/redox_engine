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
///
/// Implementors must also report every component they access through
/// [`QueryData::for_each_access`] (with `mutable = true` for any `&mut`
/// reference they hand out): [`Query::iter_mut`](crate::query::iter::Query::iter_mut)
/// relies on it to reject queries that would alias a mutable reference.
pub unsafe trait QueryData {
    /// The type yielded by the iterator (e.g. `(&'w A, &'w B)`).
    type Item<'w>;

    /// Returns `true` if `archetype` contains every component needed by this query.
    fn matches(archetype: &Archetype) -> bool;

    /// Calls `f` with the [`TypeId`] of every component this fetch can mutate.
    /// Used by [`Query::iter_mut`](crate::query::iter::Query::iter_mut) to stamp
    /// change ticks on the rows it yields.
    fn for_each_mutated(f: &mut dyn FnMut(TypeId));

    /// Calls `f(type_id, mutable)` for every component this fetch accesses.
    ///
    /// Used to reject aliasing fetches such as `(&mut T, &T)` or `(&mut T, &mut T)`.
    /// The default reports only the mutated components (as mutable); all in-crate
    /// implementations override it to also report shared accesses.
    fn for_each_access(f: &mut dyn FnMut(TypeId, bool)) {
        Self::for_each_mutated(&mut |tid| f(tid, true));
    }

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

// SAFETY: `fetch` returns a shared reference into the column of `T` (which exists
// when `matches` holds); it only reads, so shared access suffices and `fetch_mut`
// merely forwards to it. The reference lifetime is tied to the archetype borrow.
unsafe impl<T: 'static + Send + Sync> QueryData for &T {
    type Item<'w> = &'w T;

    #[inline]
    fn matches(archetype: &Archetype) -> bool {
        archetype.table.has_component(TypeId::of::<T>())
    }

    #[inline]
    fn for_each_mutated(_f: &mut dyn FnMut(TypeId)) {}

    #[inline]
    fn for_each_access(f: &mut dyn FnMut(TypeId, bool)) {
        f(TypeId::of::<T>(), false);
    }

    /// # Safety
    /// `archetype` must satisfy `matches` (a `T` column exists) and `row` must be a
    /// valid row; no `&mut T` to that element may be live.
    #[inline]
    unsafe fn fetch(archetype: &Archetype, row: usize) -> Self::Item<'_> {
        // SAFETY: `matches` guarantees the `T` column exists (so `unwrap_unchecked`
        // is sound); `row` is in bounds and the column stores aligned `T`s; the
        // caller guarantees no aliasing `&mut`.
        unsafe {
            let col = archetype
                .table
                .columns
                .get(&TypeId::of::<T>())
                .unwrap_unchecked();
            &*(col.get(row) as *const T)
        }
    }

    /// # Safety
    /// Same as [`QueryData::fetch`].
    #[inline]
    unsafe fn fetch_mut(archetype: &mut Archetype, row: usize) -> Self::Item<'_> {
        // SAFETY: forwarded; `&mut Archetype` rules out any other live reference.
        unsafe { Self::fetch(&*archetype, row) }
    }
}

impl<T: 'static + Send + Sync> ReadOnlyQueryData for &T {}

// --- &mut T ---

// SAFETY: `fetch_mut` hands out `&mut T` into the `T` column; exclusivity is provided
// by the `&mut Archetype` argument and, across a tuple, by `for_each_access`
// rejecting duplicated mutable access (see `Query::iter_mut`).
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
    fn for_each_access(f: &mut dyn FnMut(TypeId, bool)) {
        f(TypeId::of::<T>(), true);
    }

    /// # Safety
    /// `archetype` must satisfy `matches`, `row` must be valid, and the caller must
    /// have *exclusive* access to the whole archetype (no other reference to any
    /// element of the `T` column may be live) even though only `&Archetype` is passed.
    #[inline]
    unsafe fn fetch(archetype: &Archetype, row: usize) -> Self::Item<'_> {
        // SAFETY: `matches` guarantees the column exists; `row` is in bounds and the
        // column stores aligned `T`s; the caller guarantees exclusive access (see
        // `# Safety`), so creating `&mut T` from a shared archetype borrow is sound.
        unsafe {
            let col = archetype
                .table
                .columns
                .get(&TypeId::of::<T>())
                .unwrap_unchecked();
            &mut *(col.get(row) as *mut T)
        }
    }

    /// # Safety
    /// Same as [`QueryData::fetch`] (minus the exclusivity burden, which the
    /// `&mut Archetype` argument discharges for this call).
    #[inline]
    unsafe fn fetch_mut(archetype: &mut Archetype, row: usize) -> Self::Item<'_> {
        // SAFETY: `matches` guarantees the column exists; `row` is in bounds; column
        // stores aligned `T`s; `&mut Archetype` proves exclusive access.
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

// SAFETY: yields an `Entity` by value via a bounds-checked slice index; no
// references into component storage are created.
unsafe impl QueryData for Entity {
    type Item<'w> = Entity;

    #[inline]
    fn matches(_archetype: &Archetype) -> bool {
        // Every archetype stores entities; an empty world simply yields nothing.
        true
    }

    #[inline]
    fn for_each_mutated(_f: &mut dyn FnMut(TypeId)) {}

    /// # Safety
    /// No additional requirements (indexing is bounds-checked); `unsafe` only to
    /// satisfy the trait signature.
    #[inline]
    unsafe fn fetch(archetype: &Archetype, row: usize) -> Self::Item<'_> {
        archetype.entities()[row]
    }

    /// # Safety
    /// See [`QueryData::fetch`].
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
        // SAFETY: each element upholds the `QueryData` contract on its own; the
        // elements are fetched from the same row of the same archetype, and
        // `Query::iter_mut` rejects tuples whose elements access the same component
        // with a mutable reference (see `for_each_access`), so the `&mut`
        // references produced by `fetch_mut` are pairwise disjoint.
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
            fn for_each_access(f: &mut dyn FnMut(TypeId, bool)) {
                $( $name::for_each_access(f); )+
            }

            /// # Safety
            /// Same as [`QueryData::fetch`] for every element.
            #[inline]
            unsafe fn fetch(archetype: &Archetype, row: usize) -> Self::Item<'_> {
                // SAFETY: forwarded; the caller upholds each element contract.
                unsafe { ( $( $name::fetch(archetype, row), )+ ) }
            }

            /// # Safety
            /// Same as [`QueryData::fetch_mut`] for every element; additionally the
            /// elements must not access the same component mutably (checked by
            /// `Query::iter_mut` through `for_each_access`).
            #[inline]
            unsafe fn fetch_mut(archetype: &mut Archetype, row: usize) -> Self::Item<'_> {
                let arch_ptr: *mut Archetype = archetype;
                // SAFETY: `arch_ptr` is derived from a live `&mut Archetype` and is
                // reborrowed once per element. Each element touches a distinct
                // component column (no duplicate mutable access, see above), so the
                // resulting references are pairwise disjoint.
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
