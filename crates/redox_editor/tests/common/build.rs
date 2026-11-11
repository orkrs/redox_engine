//! Builders of test data (EDT-A7): a PNG and a GLB written as bytes, scene text in the
//! `SceneFile` form, asset sidecars and deliberately damaged files.
//!
//! Binary fixtures are generated here and written into a project at test time, because the
//! repository keeps only text data (see `tests/data/README.md`).

use std::fmt::Write as _;
use std::path::Path;

use redox_asset::guid::{META_EXTENSION, content_hash};
use redox_asset::{AssetGuid, AssetMeta};

/// Side length of one checker cell in the generated PNG, in pixels.
const CHECKER_CELL: u32 = 4;
/// Largest payload of one stored deflate block.
const STORED_BLOCK_MAX: usize = 65_535;

/// A reference to an asset from a component, as written into a scene.
#[derive(Clone, Debug)]
pub struct AssetRef {
    /// Registered component name, for example `MaterialHandle`.
    pub type_name: String,
    /// GUID of the asset.
    pub guid: u128,
    /// Path relative to the project root, a hint only.
    pub path: String,
}

/// One entity of a scene built by [`make_scene`].
#[derive(Clone, Debug)]
pub struct SceneEntitySpec {
    /// Stable identity of the entity.
    pub guid: u128,
    /// Value of the `Name` component.
    pub name: String,
    /// Value of the `Transform` translation.
    pub translation: [f32; 3],
    /// GUID of the parent, if any.
    pub parent: Option<u128>,
    /// Optional asset reference.
    pub asset: Option<AssetRef>,
}

/// Builds a PNG with a checker pattern. The output is deterministic.
pub fn make_png(width: u32, height: u32) -> Vec<u8> {
    let mut raw = Vec::with_capacity(height as usize * (1 + width as usize * 3));
    for y in 0..height {
        raw.push(0); // filter type: none
        for x in 0..width {
            let dark = (x / CHECKER_CELL + y / CHECKER_CELL).is_multiple_of(2);
            let value = if dark { 32u8 } else { 224u8 };
            raw.extend_from_slice(&[value, value, value]);
        }
    }
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB, deflate, adaptive filters, no interlace

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    push_chunk(&mut png, b"IHDR", &ihdr);
    push_chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    push_chunk(&mut png, b"IEND", &[]);
    png
}

/// Builds a minimal binary glTF with one triangle.
pub fn make_glb(triangle: [[f32; 3]; 3]) -> Vec<u8> {
    let mut bin = Vec::with_capacity(36);
    for vertex in triangle {
        for component in vertex {
            bin.extend_from_slice(&component.to_le_bytes());
        }
    }
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for vertex in triangle {
        for axis in 0..3 {
            min[axis] = min[axis].min(vertex[axis]);
            max[axis] = max[axis].max(vertex[axis]);
        }
    }
    let mut json = GLB_JSON
        .replace("MIN", &list3(min))
        .replace("MAX", &list3(max))
        .into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

const GLB_JSON: &str = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[MIN],"max":[MAX]}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36}],"buffers":[{"byteLength":36}]}"#;

fn list3(values: [f32; 3]) -> String {
    format!("{:?},{:?},{:?}", values[0], values[1], values[2])
}

/// Writes a scene file in the `SceneFile` form (format version 1) for the given entities.
pub fn make_scene(name: &str, entities: &[SceneEntitySpec]) -> String {
    let mut out = String::new();
    writeln!(out, "(").expect("write to string");
    writeln!(out, "    format_version: 1,").expect("write to string");
    writeln!(out, "    name: \"{name}\",").expect("write to string");
    writeln!(out, "    entities: [").expect("write to string");
    for entity in entities {
        write_entity(&mut out, entity);
    }
    writeln!(out, "    ],").expect("write to string");
    writeln!(out, ")").expect("write to string");
    out
}

fn write_entity(out: &mut String, entity: &SceneEntitySpec) {
    writeln!(out, "        (").expect("write to string");
    writeln!(out, "            guid: Some(\"{:032x}\"),", entity.guid).expect("write to string");
    writeln!(out, "            components: [").expect("write to string");
    writeln!(
        out,
        "                Struct(type_name: \"Name\", fields: [(\"name\", String(\"{}\"))]),",
        entity.name
    )
    .expect("write to string");
    let [x, y, z] = entity.translation;
    writeln!(
        out,
        "                Struct(type_name: \"Transform\", fields: [(\"translation\", List([F64({x:?}), F64({y:?}), F64({z:?})])), (\"rotation\", List([F64(0.0), F64(0.0), F64(0.0), F64(1.0)])), (\"scale\", List([F64(1.0), F64(1.0), F64(1.0)]))]),"
    )
    .expect("write to string");
    if let Some(parent) = entity.parent {
        writeln!(
            out,
            "                Struct(type_name: \"ParentRef\", fields: [(\"parent\", String(\"{parent:032x}\"))]),"
        )
        .expect("write to string");
    }
    if let Some(asset) = &entity.asset {
        writeln!(
            out,
            "                Struct(type_name: \"{}\", fields: [(\"guid\", String(\"{:032x}\")), (\"path\", String(\"{}\"))]),",
            asset.type_name, asset.guid, asset.path
        )
        .expect("write to string");
    }
    writeln!(out, "            ],").expect("write to string");
    writeln!(out, "        ),").expect("write to string");
}

/// Writes `bytes` to `path` and a sidecar `<path>.meta` with the given GUID and the content
/// hash of `bytes`, so that the registry sees the asset as imported and unchanged.
pub fn write_asset_with_meta(path: &Path, bytes: &[u8], guid: u128) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the asset directory is created");
    }
    std::fs::write(path, bytes).expect("the asset is written");
    let meta = AssetMeta {
        guid: AssetGuid(guid),
        importer_version: 1,
        source_hash: content_hash(bytes),
        settings: String::new(),
    };
    let text = ron::ser::to_string_pretty(&meta, ron::ser::PrettyConfig::default())
        .expect("the sidecar is serialized");
    let meta_path = sidecar_path(path);
    std::fs::write(meta_path, text).expect("the sidecar is written");
}

/// The sidecar path of an asset: the file name with `.meta` appended.
pub fn sidecar_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".");
    name.push(META_EXTENSION);
    name.into()
}

/// Replaces the file with bytes that do not parse as its format.
pub fn corrupt(path: &Path) {
    std::fs::write(path, b"\x00\xFFnot a valid file\x00").expect("the file is overwritten");
}

/// Copies a directory tree.
pub fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("the target directory is created");
    for entry in std::fs::read_dir(from).expect("the source directory is readable") {
        let entry = entry.expect("the directory entry is readable");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("the file is copied");
        }
    }
}

fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(kind);
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    if data.is_empty() {
        out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    let mut blocks = data.chunks(STORED_BLOCK_MAX).peekable();
    while let Some(block) = blocks.next() {
        let last = blocks.peek().is_none();
        out.push(u8::from(last)); // BFINAL bit, BTYPE 00 (stored)
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    const MODULUS: u32 = 65_521;
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in bytes {
        a = (a + u32::from(byte)) % MODULUS;
        b = (b + a) % MODULUS;
    }
    (b << 16) | a
}
