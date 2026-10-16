use super::*;

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Minimal standard base64 encoder for building data-URI fixtures.
fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64_ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(BASE64_ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            BASE64_ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64_ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Binary payload of the fixture triangle: 3 positions (f32), optional
/// 3 UVs (f32), 3 indices (u32). Returns (buffer, uv_offset, total_len).
fn triangle_buffer(with_uvs: bool) -> (Vec<u8>, usize, usize) {
    let mut buf = Vec::new();
    for p in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for c in p {
            buf.extend_from_slice(&c.to_le_bytes());
        }
    }
    let uv_offset = buf.len();
    if with_uvs {
        for uv in [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0]] {
            for c in uv {
                buf.extend_from_slice(&c.to_le_bytes());
            }
        }
    }
    for i in 0u32..3 {
        buf.extend_from_slice(&i.to_le_bytes());
    }
    let total = buf.len();
    (buf, uv_offset, total)
}

/// Builds a minimal self-contained .gltf document (data-URI buffer)
/// containing a single named node + mesh + triangle.
fn triangle_gltf_json(with_uvs: bool) -> String {
    let (buf, _uv_offset, total) = triangle_buffer(with_uvs);
    let uri = base64(&buf);

    let uv_parts = if with_uvs {
        r#","TEXCOORD_0": 1},
        "indices": 2"#
            .to_string()
    } else {
        String::from("},\n            \"indices\": 1")
    };
    let uv_accessor = if with_uvs {
        r#",
{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2" }"#
    } else {
        ""
    };
    let uv_view = if with_uvs {
        r#",
{ "buffer": 0, "byteOffset": 36, "byteLength": 24 }"#
    } else {
        ""
    };
    let (idx_view, idx_accessor) = if with_uvs {
        (
            r#",
{ "buffer": 0, "byteOffset": 60, "byteLength": 12 }"#,
            2,
        )
    } else {
        (
            r#",
{ "buffer": 0, "byteOffset": 36, "byteLength": 12 }"#,
            1,
        )
    };

    format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "scene": 0,
  "scenes": [{{ "nodes": [0] }}],
  "nodes": [{{ "name": "tri", "mesh": 0 }}],
  "meshes": [{{ "name": "triangle", "primitives": [{{ "attributes": {{ "POSITION": 0{uv_parts} }}] }}],
  "accessors": [
{{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0] }}{uv_accessor},
{{ "bufferView": {idx_accessor}, "componentType": 5125, "count": 3, "type": "SCALAR" }}
  ],
  "bufferViews": [
{{ "buffer": 0, "byteOffset": 0, "byteLength": 36 }}{uv_view}{idx_view}
  ],
  "buffers": [{{ "byteLength": {total}, "uri": "data:application/octet-stream;base64,{uri}" }}]
}}"#
    )
}

/// Wraps a JSON document in a minimal binary glTF (GLB) container.
fn wrap_glb(json: &str) -> Vec<u8> {
    let json_bytes = json.as_bytes();
    let pad = (4 - json_bytes.len() % 4) % 4;
    let json_padded = json_bytes.len() + pad;
    let total = 12 + 8 + json_padded;

    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_padded as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(json_bytes);
    out.extend(std::iter::repeat_n(b' ', pad));
    out
}

fn assert_triangle_payload(scene: &GltfSceneData) {
    assert_eq!(scene.meshes.len(), 1, "expected exactly one mesh");
    let mesh = &scene.meshes[0];
    assert_eq!(mesh.name, "triangle");
    assert_eq!(
        mesh.positions,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
    );
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert_eq!(
        mesh.normals,
        vec![[0.0, 1.0, 0.0]; 3],
        "missing normals default to up"
    );
    assert_eq!(mesh.tangents.len(), 3);

    assert_eq!(scene.nodes.len(), 1);
    let node = &scene.nodes[0];
    assert_eq!(node.name, "tri");
    assert_eq!(node.translation, [0.0, 0.0, 0.0]);
    assert_eq!(node.rotation_quat, [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(node.scale, [1.0, 1.0, 1.0]);
    assert_eq!(node.mesh_index, Some(0));

    assert!(scene.materials.is_empty());
    assert!(scene.textures.is_empty());
}

#[test]
fn extensions_include_gltf_and_glb() {
    let exts = GltfLoader.extensions();
    assert!(exts.contains(&"gltf") && exts.contains(&"glb"));
}

/// Loads the inline triangle from a temp file on disk (exercising the
/// manager-style bytes path) and checks the full payload.
#[test]
fn loads_triangle_gltf_from_temp_file() {
    let json = triangle_gltf_json(false);
    let path =
        std::env::temp_dir().join(format!("redox_gltf_triangle_a_{}.gltf", std::process::id()));
    std::fs::write(&path, &json).expect("write fixture");
    let bytes = std::fs::read(&path).expect("read fixture");
    let _ = std::fs::remove_file(&path);

    let scene = GltfLoader.load(&bytes).expect("load should succeed");
    assert_triangle_payload(&scene);

    // No UVs in this fixture: tangent generation degenerates to fallback.
    assert_eq!(scene.meshes[0].uvs, vec![[0.0, 0.0]; 3]);
    assert_eq!(scene.meshes[0].tangents, vec![[1.0, 0.0, 0.0, 1.0]; 3]);
}

/// Same triangle with UVs: tangents must be computed as +X / handed +1.
#[test]
fn computes_tangents_from_uvs() {
    let json = triangle_gltf_json(true);
    let path =
        std::env::temp_dir().join(format!("redox_gltf_triangle_b_{}.gltf", std::process::id()));
    std::fs::write(&path, &json).expect("write fixture");
    let bytes = std::fs::read(&path).expect("read fixture");
    let _ = std::fs::remove_file(&path);

    let scene = GltfLoader.load(&bytes).expect("load should succeed");
    assert_eq!(
        scene.meshes[0].uvs,
        vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]
    );
    for t in &scene.meshes[0].tangents {
        assert!((t[0] - 1.0).abs() < 1e-5, "tx should be 1, got {:?}", t);
        assert!(t[1].abs() < 1e-5 && t[2].abs() < 1e-5, "got {:?}", t);
        assert_eq!(t[3], 1.0);
    }
}

/// A binary glTF (GLB) container must parse identically.
#[test]
fn loads_glb_container() {
    let glb = wrap_glb(&triangle_gltf_json(false));
    let scene = GltfLoader.load(&glb).expect("load should succeed");
    assert_triangle_payload(&scene);
}

#[test]
fn loads_gltf_with_external_png() {
    let dir = std::env::temp_dir().join(format!("redox_gltf_ext_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    image::RgbImage::from_pixel(1, 1, image::Rgb([255, 0, 0]))
        .save(dir.join("tex.png"))
        .unwrap();

    let (buf, _uv, total) = triangle_buffer(true);
    let bin_name = "tri.bin";
    std::fs::write(dir.join(bin_name), &buf).unwrap();

    let json = format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "scene": 0,
  "scenes": [{{ "nodes": [0] }}],
  "nodes": [{{ "name": "tri", "mesh": 0 }}],
  "meshes": [{{ "name": "triangle", "primitives": [{{ "attributes": {{ "POSITION": 0, "TEXCOORD_0": 1 }}, "indices": 2, "material": 0 }}] }}],
  "materials": [{{ "pbrMetallicRoughness": {{ "baseColorTexture": {{ "index": 0 }} }} }}],
  "textures": [{{ "source": 0 }}],
  "images": [{{ "uri": "tex.png" }}],
  "accessors": [
{{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0] }},
{{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2" }},
{{ "bufferView": 2, "componentType": 5125, "count": 3, "type": "SCALAR" }}
  ],
  "bufferViews": [
{{ "buffer": 0, "byteOffset": 0, "byteLength": 36 }},
{{ "buffer": 0, "byteOffset": 36, "byteLength": 24 }},
{{ "buffer": 0, "byteOffset": 60, "byteLength": 12 }}
  ],
  "buffers": [{{ "byteLength": {total}, "uri": "{bin_name}" }}]
}}"#
    );
    let gltf_path = dir.join("scene.gltf");
    std::fs::write(&gltf_path, json).unwrap();

    let scene = load_scene_from_path(&gltf_path).expect("external glTF should load");
    assert_eq!(scene.textures.len(), 1);
    assert_eq!(scene.textures[0].width, 1);
    assert_eq!(scene.textures[0].height, 1);
    assert_eq!(scene.materials.len(), 1);
    assert!(
        scene.materials[0]
            .base_color_texture_path
            .as_ref()
            .is_some_and(|p| p.ends_with("tex.png")),
        "expected external texture path, got {:?}",
        scene.materials[0].base_color_texture_path
    );
    let _ = std::fs::remove_dir_all(&dir);
}
