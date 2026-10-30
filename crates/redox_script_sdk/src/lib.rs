//! SDK for WebAssembly gameplay scripts of the RedOx Engine (host API of `redox_script`, see
//! `docs/SCRIPTING_GUIDE.md`).
//!
//! A script is a `cdylib` that exports `on_update(dt)`. Write the logic as a plain function and
//! register it with [`script!`]:
//!
//! ```ignore
//! use redox_script_sdk as sdk;
//!
//! fn update(_dt: f32) {
//!     if sdk::input("px") < 1.0 {
//!         sdk::emit("entered", 1.0);
//!     }
//! }
//! sdk::script!(update);
//! ```
//!
//! Names are passed to the host as `(pointer, length)` pairs into the module's own memory. They
//! must be at most [`MAX_NAME_BYTES`] bytes long; the host ignores longer names.
//! The crate is `no_std` and needs no allocator. Outside `wasm32` the functions answer like an
//! empty world (there is no host), so the workspace builds on every target.

#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]

mod api;
mod host;
mod world;

pub use api::{MAX_NAME_BYTES, command, emit, input, output, raycast, system_command};
pub use world::{
    EntityId, find_by_name, find_by_tag, get_value_of, nav_path, overlap_box, overlap_sphere,
    position_of, raycast_entity,
};

/// Exports `on_update(dt: f32)`, which calls the given `fn(f32)` once per update.
#[macro_export]
macro_rules! script {
    ($update:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn on_update(dt: f32) {
            $update(dt)
        }
    };
}

/// Address and length of a name or buffer in the module's memory, as the host expects them.
pub(crate) fn addr<T>(value: &T) -> i32 {
    core::ptr::from_ref(value) as usize as i32
}

/// Address of the first element of a slice, as the host expects it.
pub(crate) fn addr_of_slice<T>(items: &[T]) -> i32 {
    items.as_ptr() as usize as i32
}

/// Length as the host expects it; lengths beyond `i32::MAX` cannot occur for module memory.
pub(crate) fn len_of<T>(items: &[T]) -> i32 {
    i32::try_from(items.len()).unwrap_or(i32::MAX)
}
