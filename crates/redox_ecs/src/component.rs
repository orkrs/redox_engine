use std::any::TypeId;

/// Trait implemented by all component types.
///
/// Components must be thread-safe (`Send + Sync`) and have a static lifetime.
pub trait Component: 'static + Send + Sync {}

// Blanket implementation for all types that satisfy the requirements.
impl<T: 'static + Send + Sync> Component for T {}

/// Drops a single value of type `T` stored at the given byte pointer.
///
/// Used as type-erased drop glue by [`ComponentInfo`].
unsafe fn drop_one<T>(ptr: *mut u8) {
    // SAFETY: caller guarantees `ptr` points to a live, properly aligned `T`.
    unsafe {
        std::ptr::drop_in_place(ptr as *mut T);
    }
}

/// Metadata about a component type.
// `PartialEq` on `drop_fn` compares function addresses (unstable across
// codegen units); it exists only to make `ComponentInfo` comparable in tests.
#[derive(Debug, Clone, Copy)]
#[allow(unpredictable_function_pointer_comparisons)]
pub struct ComponentInfo {
    pub type_id: TypeId,
    pub size: usize,
    pub align: usize,
    /// Type-erased drop glue (`None` for `Copy` / drop-free types).
    ///
    /// Columns invoke this when overwriting or removing an element so that
    /// heap-owning components (`String`, `Vec`, …) are not leaked.
    pub drop_fn: Option<unsafe fn(*mut u8)>,
}

impl PartialEq for ComponentInfo {
    fn eq(&self, other: &Self) -> bool {
        self.type_id == other.type_id && self.size == other.size && self.align == other.align
    }
}

impl Eq for ComponentInfo {}

impl ComponentInfo {
    /// Creates `ComponentInfo` for the given type `T`.
    pub fn of<T: Component>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            size: std::mem::size_of::<T>(),
            align: std::mem::align_of::<T>(),
            drop_fn: if std::mem::needs_drop::<T>() {
                Some(drop_one::<T>)
            } else {
                None
            },
        }
    }
}
