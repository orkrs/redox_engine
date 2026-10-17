# ADR 0004: KTX2 textures — Zstd, Basis Universal, cube maps, streaming

Status: accepted (2026-10-17)

## Decision

- **Container:** KTX2 is the only GPU texture container. `Ktx2Texture` holds format, size, layers,
  faces and the mip chain (each level layer-major, then face).
- **Zstd:** decoded with the pure-Rust `ruzstd`; declared sizes are checked before allocating so a
  small file cannot demand gigabytes. `zstd` is a dev-dependency only, as a reference encoder.
- **Basis Universal:** the C++ transcoder through `basis-universal` (feature `basis`, on by
  default). UASTC and ETC1S/BasisLZ are supported in `.basis` files and in KTX2. The wrapper only
  exposes `.basis` transcoding, so KTX2 UASTC/BasisLZ is converted in memory to a `.basis` file and
  transcoded through that path. The low-level UASTC transcoder is not used (it crashed on RGBA32).
- **Target format:** BC7 when the device has `TEXTURE_COMPRESSION_BC` and the size is a multiple of
  4, otherwise RGBA8. The application sets it once (`Renderer::basis_target`).
- **Safety:** C++ code aborts on corrupt input, so `.basis` data is checked (header and data CRC16,
  structure, bounded counts) before it reaches the transcoder, loaders run under `catch_unwind`,
  and all arithmetic on file-controlled values is checked. A test flips every byte of the fixtures.
- **Cube maps and arrays:** a cube is six array layers with a Cube view; bind layouts exist for
  2D, 2D array, cube and cube array. Baked IBL (`redox_cook bake-ibl`) is stored as KTX2 cubes.
- **Streaming:** `AssetManager` has a priority queue with cancel and a worker pool. A
  `StreamedTexture` reads only the header at open and each mip by range read. `TextureStreamer`
  keeps a small floor mip resident, chooses the wanted mip from on-screen size, rebuilds the GPU
  texture on promotion or demotion (GPU copy of shared levels), stays under a byte budget by
  demoting the least recently needed textures, and holds a sharp mip for a number of frames
  before giving it up.

## Why

- No pure-Rust Basis transcoder exists; the C++ one is mature and Apache-2.0.
- A texture with only the resident mips costs memory proportional to what is shown; rebuild by GPU
  copy is cheap and keeps the old texture rendering until the new one is complete.
- Hysteresis stops flicker when an object moves in and out of view.

## Consequences

- The build needs a C++ compiler for `basis-universal`.
- Basis input from untrusted sources is mitigated, not proven safe.
- BasisLZ and UASTC+Zstd KTX2 files made by Khronos KTX-Software (6 files, including 2048 px cube maps) load correctly: `REDOX_THIRD_PARTY_KTX2=<folder> cargo test -p redox_asset --test third_party`. The ETC1S and UASTC versions of one picture decode to within 2.2/255 of each other. The files are not committed (fetch them from KTX-Software `tests/resources/ktx2`, Git LFS).
