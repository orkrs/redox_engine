use std::alloc::{self, Layout};
use std::ptr::{self, NonNull};
use crate::component::ComponentInfo;

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

// SAFETY: Column stores raw bytes and metadata. Immutable access from multiple threads
// is safe because data is never mutated after creation (except during controlled moves).
unsafe impl Send for Column {}
unsafe impl Sync for Column {}

impl Column {
    /// Creates a new empty column for the given component type.
    pub fn new(info: ComponentInfo) -> Self {
        Self {
            info,
            data: NonNull::dangling(),
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
    /// `component_ptr` must point to a valid instance of the component type.
    pub unsafe fn push(&mut self, component_ptr: *const u8, tick: u32) {
        if self.info.size > 0 && self.len == self.cap {
            self.grow();
        }

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
    /// `component_ptr` must point to a valid instance of the component type and
    /// `index` must be within bounds.
    pub unsafe fn replace(&mut self, index: usize, component_ptr: *const u8, tick: u32) {
        debug_assert!(index < self.len);
        unsafe {
            let dst = self.get(index);
            if let Some(drop_fn) = self.info.drop_fn {
                drop_fn(dst);
            }
            ptr::copy_nonoverlapping(component_ptr, dst, self.info.size);
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
    /// `index` must be within bounds.
    pub unsafe fn swap_remove(&mut self, index: usize, drop: bool) -> bool {
        debug_assert!(index < self.len);
        let size = self.info.size;
        let last_index = self.len - 1;

        // Drop the removed element in place before its bytes are overwritten.
        if drop {
            if let Some(drop_fn) = self.info.drop_fn {
                unsafe { drop_fn(self.get(index)) };
            }
        }

        if index != last_index {
            unsafe {
                ptr::copy_nonoverlapping(
                    self.get(last_index),
                    self.get(index),
                    size,
                );
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
    /// `index` must be within bounds.
    pub unsafe fn get(&self, index: usize) -> *mut u8 {
        unsafe {
            self.data.as_ptr().add(index * self.info.size)
        }
    }

    fn grow(&mut self) {
        if self.info.size == 0 {
            // Zero-sized components occupy no bytes; nothing to allocate.
            return;
        }
        let (new_cap, new_layout) = if self.cap == 0 {
            (8, Layout::from_size_align(8 * self.info.size, self.info.align).unwrap())
        } else {
            let new_cap = self.cap * 2;
            let new_layout = Layout::from_size_align(new_cap * self.info.size, self.info.align).unwrap();
            (new_cap, new_layout)
        };

        let new_ptr = unsafe {
            if self.cap == 0 {
                alloc::alloc(new_layout)
            } else {
                let old_layout = Layout::from_size_align(self.cap * self.info.size, self.info.align).unwrap();
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
                unsafe { drop_fn(self.get(i)) };
            }
        }
        if self.cap > 0 {
            let layout = Layout::from_size_align(self.cap * self.info.size, self.info.align).unwrap();
            unsafe {
                alloc::dealloc(self.data.as_ptr(), layout);
            }
        }
    }
}
