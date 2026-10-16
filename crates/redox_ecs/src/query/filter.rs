//! Archetype filters for queries.
//!
//! Filters are zero-sized marker types that constrain which archetypes a query
//! will visit **without** fetching any extra data. Several filters can be
//! combined by using a tuple: `(With<Player>, Without<Dead>)`.
//!
//! # Change detection
//!
//! [`Changed<T>`] and [`Added<T>`] are **row-level** filters evaluated against
//! the [`TickCtx`] captured when iteration starts:
//!
//! - `Changed<T>` — rows whose component was written since the current frame
//!   began (see [`World::begin_frame`](crate::World::begin_frame)).
//! - `Added<T>` — rows whose component was inserted since the current frame began.

use std::marker::PhantomData;
use std::any::TypeId;
use crate::archetype::Archetype;
use crate::component::Component;

// ---------------------------------------------------------------------------
// Tick context
// ---------------------------------------------------------------------------

/// Change-detection context captured from the [`World`](crate::World) when a
/// query starts iterating. Carries the frame-start tick so `Changed`/`Added`
/// can compare row stamps against "this frame".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickCtx {
    /// Value of `World::change_tick` at the last `World::begin_frame()`.
    pub frame_tick: u32,
}

impl TickCtx {
    /// Context that treats every row as changed/added (used by filters that
    /// ignore ticks, so they never need world access).
    pub const ANY: TickCtx = TickCtx { frame_tick: 0 };
}

// ---------------------------------------------------------------------------
// QueryFilter trait
// ---------------------------------------------------------------------------

/// Constrains which archetypes (and rows) are visited by a query.
///
/// A filter must be a zero-sized type. `matches` accepts/rejects whole
/// archetypes; `row_matches` additionally rejects individual rows (used by
/// change-detection filters).
pub trait QueryFilter: 'static {
    /// Returns `true` if the archetype may contain matching rows.
    fn matches(archetype: &Archetype, ctx: TickCtx) -> bool;

    /// Returns `true` if the specific `row` passes the filter.
    /// Defaults to `true` (whole-archetype filters).
    fn row_matches(archetype: &Archetype, row: usize, ctx: TickCtx) -> bool {
        let _ = (archetype, row, ctx);
        true
    }
}

// ---------------------------------------------------------------------------
// NullFilter – default no-op filter
// ---------------------------------------------------------------------------

/// The default filter that accepts every archetype.
pub struct NullFilter;

impl QueryFilter for NullFilter {
    #[inline]
    fn matches(_archetype: &Archetype, _ctx: TickCtx) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// With<T> – archetype must contain T
// ---------------------------------------------------------------------------

/// Filter that requires component `T` to be present.
///
/// # Example
/// ```rust,ignore
/// world.query_filtered::<&Position, With<Player>>().iter()
/// ```
pub struct With<T: Component>(PhantomData<T>);

impl<T: Component> QueryFilter for With<T> {
    #[inline]
    fn matches(archetype: &Archetype, _ctx: TickCtx) -> bool {
        archetype.table.has_component(TypeId::of::<T>())
    }
}

// ---------------------------------------------------------------------------
// Without<T> – archetype must NOT contain T
// ---------------------------------------------------------------------------

/// Filter that requires component `T` to be absent.
///
/// # Example
/// ```rust,ignore
/// world.query_filtered::<&Position, Without<Dead>>().iter()
/// ```
pub struct Without<T: Component>(PhantomData<T>);

impl<T: Component> QueryFilter for Without<T> {
    #[inline]
    fn matches(archetype: &Archetype, _ctx: TickCtx) -> bool {
        !archetype.table.has_component(TypeId::of::<T>())
    }
}

// ---------------------------------------------------------------------------
// Changed<T> / Added<T> – row-level change detection
// ---------------------------------------------------------------------------

/// Filter that only visits rows whose component `T` was **written since the
/// current frame began**.
///
/// Writes are detected through `World::change_tick` stamps maintained by
/// `get_component_mut` and mutable query iteration.
pub struct Changed<T: Component>(PhantomData<T>);

impl<T: Component> QueryFilter for Changed<T> {
    #[inline]
    fn matches(archetype: &Archetype, _ctx: TickCtx) -> bool {
        archetype.table.has_component(TypeId::of::<T>())
    }

    #[inline]
    fn row_matches(archetype: &Archetype, row: usize, ctx: TickCtx) -> bool {
        archetype
            .table
            .column(TypeId::of::<T>())
            .and_then(|col| col.tick(row))
            .is_some_and(|tick| tick > ctx.frame_tick)
    }
}

/// Filter that only visits rows whose component `T` was **inserted since the
/// current frame began**.
pub struct Added<T: Component>(PhantomData<T>);

impl<T: Component> QueryFilter for Added<T> {
    #[inline]
    fn matches(archetype: &Archetype, _ctx: TickCtx) -> bool {
        archetype.table.has_component(TypeId::of::<T>())
    }

    #[inline]
    fn row_matches(archetype: &Archetype, row: usize, ctx: TickCtx) -> bool {
        archetype
            .table
            .column(TypeId::of::<T>())
            .and_then(|col| col.added_tick(row))
            .is_some_and(|tick| tick > ctx.frame_tick)
    }
}

// ---------------------------------------------------------------------------
// Tuple combinations of filters (AND semantics)
// ---------------------------------------------------------------------------

macro_rules! impl_query_filter_tuple {
    ($( $name:ident ),+) => {
        #[allow(non_snake_case)]
        impl<$($name: QueryFilter),+> QueryFilter for ($($name,)+) {
            #[inline]
            fn matches(archetype: &Archetype, ctx: TickCtx) -> bool {
                $( $name::matches(archetype, ctx) )&&+
            }

            #[inline]
            fn row_matches(archetype: &Archetype, row: usize, ctx: TickCtx) -> bool {
                $( $name::row_matches(archetype, row, ctx) )&&+
            }
        }
    };
}

impl_query_filter_tuple!(A, B);
impl_query_filter_tuple!(A, B, C);
impl_query_filter_tuple!(A, B, C, D);
impl_query_filter_tuple!(A, B, C, D, E);
impl_query_filter_tuple!(A, B, C, D, E, F_);
