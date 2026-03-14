//! Archetype filters for queries.
//!
//! Filters are zero-sized marker types that constrain which archetypes a query
//! will visit **without** fetching any extra data. Several filters can be
//! combined by using a tuple: `(With<Player>, Without<Dead>)`.

use std::marker::PhantomData;
use crate::archetype::Archetype;
use crate::component::Component;

// ---------------------------------------------------------------------------
// QueryFilter trait
// ---------------------------------------------------------------------------

/// Constrains which archetypes are visited by a query.
///
/// A filter must be a zero-sized type. The only thing it does is tell
/// [`Query`](super::iter::Query) whether a particular archetype should be
/// included in the iteration.
pub trait QueryFilter: 'static {
    /// Returns `true` if `archetype` passes this filter.
    fn matches(archetype: &Archetype) -> bool;
}

// ---------------------------------------------------------------------------
// NullFilter – default no-op filter
// ---------------------------------------------------------------------------

/// The default filter that accepts every archetype.
pub struct NullFilter;

impl QueryFilter for NullFilter {
    #[inline]
    fn matches(_archetype: &Archetype) -> bool {
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
    fn matches(archetype: &Archetype) -> bool {
        use std::any::TypeId;
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
    fn matches(archetype: &Archetype) -> bool {
        use std::any::TypeId;
        !archetype.table.has_component(TypeId::of::<T>())
    }
}

// ---------------------------------------------------------------------------
// Changed<T> / Added<T> – placeholders for future change-detection
// ---------------------------------------------------------------------------

/// Placeholder filter for change-detection queries (not yet implemented).
pub struct Changed<T: Component>(PhantomData<T>);

impl<T: Component> QueryFilter for Changed<T> {
    #[inline]
    fn matches(_archetype: &Archetype) -> bool {
        // Not yet implemented – currently behaves like NullFilter
        true
    }
}

/// Placeholder filter for "added this frame" queries (not yet implemented).
pub struct Added<T: Component>(PhantomData<T>);

impl<T: Component> QueryFilter for Added<T> {
    #[inline]
    fn matches(_archetype: &Archetype) -> bool {
        // Not yet implemented – currently behaves like NullFilter
        true
    }
}

// ---------------------------------------------------------------------------
// Tuple combinations of filters (AND semantics)
// ---------------------------------------------------------------------------

macro_rules! impl_query_filter_tuple {
    ($( $name:ident ),+) => {
        impl<$($name: QueryFilter),+> QueryFilter for ($($name,)+) {
            #[inline]
            fn matches(archetype: &Archetype) -> bool {
                $( $name::matches(archetype) )&&+
            }
        }
    };
}

impl_query_filter_tuple!(A, B);
impl_query_filter_tuple!(A, B, C);
impl_query_filter_tuple!(A, B, C, D);
impl_query_filter_tuple!(A, B, C, D, E);
impl_query_filter_tuple!(A, B, C, D, E, F_);
