//! Repairs `"texCoord": -1` in glTF JSON. Some Blender exports write it next
//! to `KHR_texture_transform`; the `gltf` crate rejects it because the index
//! is unsigned, so the loader retries once with this fix applied.

use std::ops::Range;

/// Returns a copy of `bytes` in which every `"texCoord": -1` of the JSON part
/// is replaced by `0 `, or `None` when there is nothing to fix. The length
/// does not change, so GLB chunk offsets stay valid. Binary chunks are never
/// touched.
pub(super) fn fix_tex_coord_minus_one(bytes: &[u8]) -> Option<Vec<u8>> {
    const KEY: &[u8] = b"\"texCoord\"";
    let json = json_range(bytes);
    let mut out = bytes.to_vec();
    let mut changed = false;
    let mut i = json.start;
    while i + KEY.len() <= json.end {
        if bytes[i..i + KEY.len()] == *KEY
            && let Some(minus) = minus_one_after(bytes, i + KEY.len(), json.end)
        {
            out[minus] = b'0';
            out[minus + 1] = b' ';
            changed = true;
            i = minus + 2;
            continue;
        }
        i += 1;
    }
    changed.then_some(out)
}

/// Position of the `-` of `: -1` that follows a key ending at `from`.
fn minus_one_after(bytes: &[u8], from: usize, end: usize) -> Option<usize> {
    let colon = skip_ws(bytes, from, end);
    if colon >= end || bytes[colon] != b':' {
        return None;
    }
    let minus = skip_ws(bytes, colon + 1, end);
    let is_minus_one = minus + 1 < end
        && bytes[minus] == b'-'
        && bytes[minus + 1] == b'1'
        && !(minus + 2 < end && bytes[minus + 2].is_ascii_digit());
    is_minus_one.then_some(minus)
}

fn skip_ws(bytes: &[u8], mut i: usize, end: usize) -> usize {
    while i < end && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Byte range of the JSON text: the JSON chunk of a GLB, or the whole file.
fn json_range(bytes: &[u8]) -> Range<usize> {
    if bytes.len() >= 20 && &bytes[..4] == b"glTF" {
        let len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        return 20..bytes.len().min(20 + len);
    }
    0..bytes.len()
}
