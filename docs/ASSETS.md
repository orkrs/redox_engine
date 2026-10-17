# Assets and textures

## Preparing textures

| Task | Command |
|---|---|
| Look inside a texture | `redox_cook texture-info file.ktx2` |
| PNG/JPEG to Basis KTX2 (UASTC or ETC1S, with mips) | `redox_cook texture out.ktx2 in.png` (`--etc1s`, `--linear`, `--no-mips`, `--no-zstd`, `--array`, `--cube`) |
| Bake IBL cubes (irradiance, prefiltered, BRDF) from an HDR | `redox_cook bake-ibl env.hdr out_dir` (feature `bake`) |
| Pack a folder with textures converted on the way | `redox_cook pack assets out.pak --textures` (also `--compress`, `--incremental`) |

Pak v2 entries are LZ4-compressed or raw; KTX2 files go in raw so mips can be read by range.

## Loading

- Whole texture: `assets.load_async::<Ktx2Texture>("a.ktx2")`, then `sync_ktx2_textures_to_render`.
  Materials refer to it with `MaterialData::with_albedo_ktx2` (and normal/MR variants).
- Streamed: `TextureStreamer::add(&mut assets, "a.ktx2", Priority::HIGH)`. Each frame call
  `update_from_scene(&renderer, &objects)` and `update(&mut renderer, &mut assets)`. The texture is
  usable (blurry) as soon as `slot_of` is `Some`; reference it from a material with
  `Handle::<Ktx2Texture>::new(handle.id())`.
- Settings: `StreamConfig { upload_budget_bytes, texture_budget_bytes, floor_edge, demote_after_frames }`.

## Format choice

`Renderer::basis_target` decides what Basis data becomes: BC7 on devices with BC support (textures
whose size is a multiple of 4), RGBA8 otherwise. Plain KTX2 block formats fall back to RGBA8 the
same way when the device cannot hold them.

## Priorities

`Priority::URGENT` (255), `HIGH`, `NORMAL`, `LOW`. The streamer reads the floor mips urgently and the
mip it wants next at `HIGH`. Queued reads can be re-prioritised or cancelled.
