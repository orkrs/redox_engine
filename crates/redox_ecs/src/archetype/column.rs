use crate::component::ComponentInfo;
use std::alloc::{self, Layout};
use std::ptr::{self, NonNull};

/// A column of component data stored in a dense array.
///
/// Owns:
/// - the raw byte storage (`data`),
/// - per-row change-detection stamps (`ticks`, `added_ticks`).
///
/// Drop glue comes from [`ComponentInfo::drop_fn`]; it is invoked whenever an
/// element is overwritten or removed so heap-owning components are not leaked.
pub struct Column {
    info: ComponentInfo,
    data: NonNull<u8>,
    len: usize,
    cap: usize,
    ticks: Vec<u32>,
    added_ticks: Vec<u32>,
}

// SAFETY: `Column` uniquely owns its heap allocation (no aliasing handles escape),
// and every element stored in it was created from a `Component`
// (`ComponentInfo::of` requires `T: Send + Sync`), so moving the column to another
// thread (`Send`) cannot expose a non-thread-safe value. Mutation of the bytes
// requires `&mut Column` or goes through `unsafe` entry points whose callers must
// uphold exclusivity.
unsafe impl Send for Column {}
// SAFETY: see the `Send` impl above; `&Column` only permits reads of metadata and
// ticks, and raw element access is via `unsafe fn get` whose callers uphold aliasing.
unsafe impl Sync for Column {}

impl Column {
    /// Creates a new empty column for the given component type.
    pub fn new(info: ComponentInfo) -> Self {
        Self {
            // Dangling-but-aligned: zero-sized components with `align > 1` are still
            // read through `&*(ptr as *const T)`, which requires alignment.
            data: NonNull::new(std::ptr::without_provenance_mut(info.align))
                .unwrap_or(NonNull::dangling()),
            info,
            len: 0,
            cap: 0,
            ticks: Vec::new(),
            added_ticks: Vec::new(),
        }
    }

    /// Pushes a component value onto the column.
    ///
    /// `tick` is stamped as both the change tick and the added tick of the row.
    ///
    /// # Safety
    /// - `component_ptr` must be valid for reads of `info.size` bytes and point to a
    ///   valid instance of the component type.
    /// - The source range must not overlap this column's storage.
    /// - Ownership of the value moves into the column: the caller must not drop the
    ///   source value afterwards (e.g. `mem::forget`).
    pub unsafe fn push(&mut self, component_ptr: *const u8, tick: u32) {
        if self.info.size > 0 && self.len == self.cap {
            self.grow();
        }

        // SAFETY: after `grow`, `len < cap` (or `size == 0`), so the destination slot
        // `[len * size, (len + 1) * size)` lies inside the allocation; the caller
        // guarantees the source is readable for `size` bytes and non-overlapping.
        unsafe {
            let dst = self.data.as_ptr().add(self.len * self.info.size);
            ptr::copy_nonoverlapping(component_ptr, dst, self.info.size);
        }
        self.ticks.push(tick);
        self.added_ticks.push(tick);
        self.len += 1;
    }

    /// Overwrites the element at `index`, dropping the previous value.
    ///
    /// # Safety
    /// - `component_ptr` must be valid for reads of `info.size` bytes, point to a
    ///   valid instance of the component type, and not overlap this column's storage.
    /// - `index` must be `< len`, so the slot holds a live value.
    /// - Ownership of the new value moves into the column (caller must not drop it).
    pub unsafe fn replace(&mut self, index: usize, component_ptr: *const u8, tick: u32) {
        /// Writes the new value into the slot even when the old value's destructor
        /// panics, so the slot never keeps an already-dropped value (double drop).
        struct WriteBack {
            dst: *mut u8,
            src: *const u8,
            size: usize,
        }
        impl Drop for WriteBack {
            fn drop(&mut self) {
                // SAFETY: the creator guarantees `src`/`dst` are valid, non-overlapping
                // ranges of `size` bytes.
                unsafe { ptr::copy_nonoverlapping(self.src, self.dst, self.size) };
            }
        }

        debug_assert!(index < self.len);
        // SAFETY: `index < len` (caller contract), so `dst` is in bounds and holds a
        // live value which `drop_fn` consumes exactly once before the slot is
        // overwritten (by `WriteBack`, also on unwind); source validity and
        // non-overlap are caller-guaranteed.
        unsafe {
            let dst = self.get(index);
            let _write_back = WriteBack {
                dst,
                src: component_ptr,
                size: self.info.size,
            };
            if let Some(drop_fn) = self.info.drop_fn {
                drop_fn(dst);
            }
        }
        self.ticks[index] = tick;
    }

    /// Removes the element at `index` by swapping with the last element.
    ///
    /// When `drop` is set the removed element is dropped in place (despawn
    /// path). When clear, ownership of the bytes has moved elsewhere (archetype
    /// migration) and no destructor runs.
    ///
    /// Returns `true` if the last element was moved into the vacated slot.
    ///
    /// # Safety
    /// - `index` must be `< len` (the column must be non-empty).
    /// - If `drop` is clear, the caller must already have moved the value at `index`
    ///   out (or accept that it is never dropped), to avoid double drops.
    pub unsafe fn swap_remove(&mut self, index: usize, drop: bool) -> bool {
        debug_assert!(index < self.len);
        let size = self.info.size;
        let last_index = self.len - 1;

        // Drop the removed element in place before its bytes are overwritten.
        if drop && let Some(drop_fn) = self.info.drop_fn {
            // SAFETY: `index < len` (caller contract) so the slot holds a live,
            // aligned value of the column's type, dropped exactly once here; it is
            // overwritten or discarded (len shrinks) below.
            unsafe { drop_fn(self.get(index)) };
        }

        if index != last_index {
            // SAFETY: both indices are `< len` and distinct, so the two `size`-byte
            // ranges are in bounds and do not overlap.
            unsafe {
                ptr::copy_nonoverlapping(self.get(last_index), self.get(index), size);
            }
            self.ticks.swap_remove(index);
            self.added_ticks.swap_remove(index);
            self.len -= 1;
            true
        } else {
            self.ticks.pop();
            self.added_ticks.pop();
            self.len -= 1;
            false
        }
    }

    /// Returns a raw pointer to the element at `index`.
    ///
    /// # Safety
    /// `index` must be `<= len` (one-past-the-end is allowed for pointer arithmetic
    /// only); dereferencing requires `index < len`, and the caller must respect Rust
    /// aliasing rules (no `&mut` alongside any other reference to the element).
    pub unsafe fn get(&self, index: usize) -> *mut u8 {
        // SAFETY: `index * size` stays within (or one past) the allocation of
        // `cap * size` bytes given `index <= len <= cap`; for zero-sized components
        // the offset is 0.
        unsafe { self.data.as_ptr().add(index * self.info.size) }
    }

    fn grow(&mut self) {
        if self.info.size == 0 {
            // Zero-sized components occupy no bytes; nothing to allocate.
            return;
        }
        let (new_cap, new_layout) = if self.cap == 0 {
            (
                8,
                Layout::from_size_align(8 * self.info.size, self.info.align)
                    .expect("column layout comes from a Rust type and is always valid"),
            )
        } else {
            let new_cap = self.cap * 2;
            let new_layout = Layout::from_size_align(new_cap * self.info.size, self.info.align)
                .expect("grown column layout stays below isize::MAX");
            (new_cap, new_layout)
        };

        // SAFETY: `size > 0` (early return above) so `new_layout` has non-zero size as
        // `alloc` requires. For `realloc`, `data` was allocated by the global allocator
        // with exactly `old_layout` (same size/align, `cap > 0`), and
        // `new_layout.size()` is non-zero and fits `isize` (checked by
        // `Layout::from_size_align`).
        let new_ptr = unsafe {
            if self.cap == 0 {
                alloc::alloc(new_layout)
            } else {
                let old_layout =
                    Layout::from_size_align(self.cap * self.info.size, self.info.align)
                        .expect("existing column layout was valid when allocated");
                alloc::realloc(self.data.as_ptr(), old_layout, new_layout.size())
            }
        };

        self.data = NonNull::new(new_ptr).expect("Allocation failed");
        self.cap = new_cap;
    }

    /// Returns the number of elements in the column.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` when the column holds no elements.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Stamps the change tick of the row at `index` (mutation marker).
    pub fn set_tick(&mut self, index: usize, tick: u32) {
        if let Some(slot) = self.ticks.get_mut(index) {
            *slot = tick;
        }
    }

    /// Change tick of the row at `index`.
    pub fn tick(&self, index: usize) -> Option<u32> {
        self.ticks.get(index).copied()
    }

    /// Added tick of the row at `index`.
    pub fn added_tick(&self, index: usize) -> Option<u32> {
        self.added_ticks.get(index).copied()
    }
}

impl Drop for Column {
    fn drop(&mut self) {
        // Run drop glue over every live element before deallocating storage.
        if let Some(drop_fn) = self.info.drop_fn {
            for i in 0..self.len {
                // SAFETY: every index `< len` holds a live value that has not been
                // dropped or moved out; each is dropped exactly once here.
                unsafe { drop_fn(self.get(i)) };
            }
        }
        if self.cap > 0 {
            let layout = Layout::from_size_align(self.cap * self.info.size, self.info.align)
                .expect("existing column layout was valid when allocated");
            // SAFETY: `cap > 0` implies `data` was allocated by the global allocator with
            // size `cap * size` and alignment `align`, matching `layout`; it is freed
            // once, here. (`cap > 0` only occurs when `size > 0`.)
            unsafe {
                alloc::dealloc(self.data.as_ptr(), layout);
            }
        }
    }
}
