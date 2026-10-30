//! Raw imports of the host module `env`. Every function takes addresses and lengths into the
//! module's memory; the safe wrappers in `api` and `world` build them.
//!
//! Outside `wasm32` the same names exist as stubs that answer like an empty world.

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    pub(crate) fn get_f32(name: i32, name_len: i32) -> f32;
    pub(crate) fn set_f32(name: i32, name_len: i32, value: f32);
    pub(crate) fn emit_event(name: i32, name_len: i32, value: f32);
    pub(crate) fn emit_command(name: i32, name_len: i32, a: f32, b: f32, c: f32);
    pub(crate) fn raycast(ox: f32, oy: f32, oz: f32, dx: f32, dy: f32, dz: f32, max: f32) -> f32;
    pub(crate) fn find_by_name(name: i32, name_len: i32) -> i32;
    pub(crate) fn find_by_tag(tag: i32, tag_len: i32, out: i32, out_cap: i32) -> i32;
    pub(crate) fn position_of(id: i32, out: i32) -> i32;
    pub(crate) fn raycast_entity(origin: i32, direction: i32, max: f32, out: i32) -> i32;
    pub(crate) fn overlap_sphere(cx: f32, cy: f32, cz: f32, radius: f32, out: i32, out_cap: i32)
    -> i32;
    pub(crate) fn overlap_box(
        cx: f32,
        cy: f32,
        cz: f32,
        hx: f32,
        hy: f32,
        hz: f32,
        out: i32,
        out_cap: i32,
    ) -> i32;
    pub(crate) fn get_value_of(id: i32, key: i32, key_len: i32, out: i32) -> i32;
    pub(crate) fn nav_path(from: i32, to: i32, out: i32, out_cap: i32) -> i32;
}

/// Stubs for host builds: no world, no values, no paths.
#[cfg(not(target_arch = "wasm32"))]
mod stub {
    pub(crate) unsafe fn get_f32(_: i32, _: i32) -> f32 {
        0.0
    }
    pub(crate) unsafe fn set_f32(_: i32, _: i32, _: f32) {}
    pub(crate) unsafe fn emit_event(_: i32, _: i32, _: f32) {}
    pub(crate) unsafe fn emit_command(_: i32, _: i32, _: f32, _: f32, _: f32) {}
    pub(crate) unsafe fn raycast(_: f32, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) -> f32 {
        -1.0
    }
    pub(crate) unsafe fn find_by_name(_: i32, _: i32) -> i32 {
        -1
    }
    pub(crate) unsafe fn find_by_tag(_: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    pub(crate) unsafe fn position_of(_: i32, _: i32) -> i32 {
        0
    }
    pub(crate) unsafe fn raycast_entity(_: i32, _: i32, _: f32, _: i32) -> i32 {
        0
    }
    pub(crate) unsafe fn overlap_sphere(_: f32, _: f32, _: f32, _: f32, _: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) unsafe fn overlap_box(
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: f32,
        _: i32,
        _: i32,
    ) -> i32 {
        0
    }
    pub(crate) unsafe fn get_value_of(_: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    pub(crate) unsafe fn nav_path(_: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use stub::*;
