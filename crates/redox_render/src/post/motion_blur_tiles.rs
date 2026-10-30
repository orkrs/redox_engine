//! Motion blur spreading beyond object edges (PST-03).
//!
//! The velocity buffer is cut into `TILE_SIZE` square tiles. Each tile keeps its fastest
//! velocity, and each tile then takes the fastest over its 3x3 neighbourhood. A pixel that does
//! not move itself but sits next to a fast tile gets blurred along that tile's velocity, when
//! the neighbour reaches the pixel and is not behind it. The functions here are the CPU mirror
//! of `shaders/motion_blur_tiles.wgsl` and of the spreading part of `shaders/motion_blur.wgsl`;
//! keep them in sync. Velocities are in pixels unless a name says otherwise.

use crate::shader::MOTION_BLUR_TILES_WGSL;
use crate::shader::create_shader_module;

/// Tile edge in pixels.
pub const TILE_SIZE: u32 = 20;
/// Workgroup edge of the tile compute passes; must match `@workgroup_size` in the shader.
const WORKGROUP_SIZE: u32 = 8;
/// Depth slack in linear units: a neighbour up to this far behind the pixel still blurs it.
pub const SPREAD_SOFT_Z: f32 = 0.25;
/// Format of the tile textures. `Rg16Float` is not storage-capable in core WebGPU, so the
/// tiles use `Rgba16Float` with the velocity in xy (ASSUMPTION, see PST-03 report).
const TILE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const FILTERABLE: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: true };

fn squared_length(v: [f32; 2]) -> f32 {
    v[0] * v[0] + v[1] * v[1]
}

/// Largest-magnitude velocity in a tile (pixels); ties keep the first. Empty input gives zero.
pub fn tile_max(velocities: &[[f32; 2]]) -> [f32; 2] {
    let mut best = [0.0, 0.0];
    let mut best_sq = -1.0;
    for &v in velocities {
        let sq = squared_length(v);
        if sq > best_sq {
            best = v;
            best_sq = sq;
        }
    }
    best
}

/// Largest-magnitude tile maximum over the 3x3 tiles around `(tx, ty)`, clipped at the image
/// edges. Out-of-range tiles give zero instead of panicking.
pub fn neighbor_max(
    tile_maxima: &[[f32; 2]],
    tiles_x: u32,
    tiles_y: u32,
    tx: u32,
    ty: u32,
) -> [f32; 2] {
    if tiles_x == 0 || tiles_y == 0 || tx >= tiles_x || ty >= tiles_y {
        return [0.0, 0.0];
    }
    let x_range = tx.saturating_sub(1)..=(tx + 1).min(tiles_x - 1);
    let y_range = ty.saturating_sub(1)..=(ty + 1).min(tiles_y - 1);
    let mut best = [0.0, 0.0];
    let mut best_sq = -1.0;
    for y in y_range {
        for x in x_range.clone() {
            let Some(&v) = tile_maxima.get((y * tiles_x + x) as usize) else {
                continue;
            };
            let sq = squared_length(v);
            if sq > best_sq {
                best = v;
                best_sq = sq;
            }
        }
    }
    best
}

/// Weight with which a neighbour's blur reaches pixel `p`, in `0..=1`.
///
/// `neighbor_velocity` is in pixels and is scaled by the exposure `shutter`, giving the reach.
/// The weight is 0 when the reach does not cover `dist_pixels`, and 0 when the neighbour is more
/// than `soft_z` behind the pixel (linear depths). Inside the reach it is 1 up to half the reach
/// and then falls smoothly to 0 at the reach limit, so the blur has no hard edge.
pub fn spread_weight(
    neighbor_velocity: [f32; 2],
    dist_pixels: f32,
    depth_p: f32,
    depth_neighbor: f32,
    shutter: f32,
    soft_z: f32,
) -> f32 {
    let reach = squared_length(neighbor_velocity).sqrt() * shutter;
    // NaN fails every comparison here, so each check also rejects NaN explicitly.
    if reach.is_nan() || reach <= 0.0 || dist_pixels.is_nan() || reach < dist_pixels {
        return 0.0;
    }
    if depth_neighbor.is_nan() || depth_neighbor > depth_p + soft_z {
        return 0.0;
    }
    let t = dist_pixels / reach;
    let u = ((t - 0.5) / 0.5).clamp(0.0, 1.0);
    1.0 - u * u * (3.0 - 2.0 * u)
}

/// Number of tiles needed to cover `size` pixels.
fn tile_count(size: u32) -> u32 {
    size.max(1).div_ceil(TILE_SIZE)
}

/// Compute pipelines of the two tile passes and their shared bind group layout.
pub(crate) struct TilePasses {
    layout: wgpu::BindGroupLayout,
    tile_max: wgpu::ComputePipeline,
    neighbor_max: wgpu::ComputePipeline,
}

impl TilePasses {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("motion_blur_tiles"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: FILTERABLE,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: TILE_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let shader = create_shader_module(device, "motion_blur_tiles", MOTION_BLUR_TILES_WGSL);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("motion_blur_tiles"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |entry: &'static str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                cache: crate::pipeline_cache::get(),
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
            })
        };
        Self {
            tile_max: make("tile_max_main"),
            neighbor_max: make("neighbor_max_main"),
            layout,
        }
    }

    /// Records the tile maximum pass, then the neighbourhood pass, into `encoder`.
    pub(crate) fn record(&self, encoder: &mut wgpu::CommandEncoder, targets: &TileTargets) {
        let groups_x = targets.tiles_x.div_ceil(WORKGROUP_SIZE);
        let groups_y = targets.tiles_y.div_ceil(WORKGROUP_SIZE);
        let steps = [
            (
                "motion_blur_tile_max",
                &self.tile_max,
                &targets.bind_groups[0],
            ),
            (
                "motion_blur_neighbor_max",
                &self.neighbor_max,
                &targets.bind_groups[1],
            ),
        ];
        for (label, pipeline, bind_group) in steps {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.dispatch_workgroups(groups_x, groups_y, 1);
        }
    }

    /// Layout the tile bind groups are built against.
    pub(crate) fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }
}

/// Tile textures and the bind groups of the two passes.
///
/// `bind_groups[0]` runs the tile maximum over the velocity buffer; `bind_groups[1]` runs the
/// neighbourhood maximum over the tile maxima. `neighbor_view` is what motion blur reads.
pub(crate) struct TileTargets {
    _textures: [wgpu::Texture; 2],
    /// Tile grid width in tiles.
    pub(crate) tiles_x: u32,
    /// Tile grid height in tiles.
    pub(crate) tiles_y: u32,
    neighbor_view: wgpu::TextureView,
    bind_groups: [wgpu::BindGroup; 2],
}

impl TileTargets {
    /// Creates the tile textures for a `width` x `height` output and the bind groups over
    /// `velocity_view`.
    pub(crate) fn new(
        device: &wgpu::Device,
        passes: &TilePasses,
        width: u32,
        height: u32,
        velocity_view: &wgpu::TextureView,
    ) -> Self {
        let tiles_x = tile_count(width);
        let tiles_y = tile_count(height);
        let make = |label: &str| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: tiles_x,
                    height: tiles_y,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TILE_FORMAT,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        };
        let (tile_tex, tile_view) = make("motion_blur_tile_max");
        let (neighbor_tex, neighbor_view) = make("motion_blur_neighbor_max");
        let bind = |label: &'static str, src: &wgpu::TextureView, dst: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: passes.layout(),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(dst),
                    },
                ],
            })
        };
        let bind_groups = [
            bind("motion_blur_tile_max", velocity_view, &tile_view),
            bind("motion_blur_neighbor_max", &tile_view, &neighbor_view),
        ];
        Self {
            _textures: [tile_tex, neighbor_tex],
            tiles_x,
            tiles_y,
            neighbor_view,
            bind_groups,
        }
    }

    /// Neighbourhood-maximum view that motion blur samples.
    pub(crate) fn neighbor_view(&self) -> &wgpu::TextureView {
        &self.neighbor_view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tile_max_is_the_longest_velocity() {
        let v = [[1.0, 0.0], [0.0, -3.0], [2.0, 2.0]];
        assert_eq!(tile_max(&v), [0.0, -3.0]);
        assert_eq!(tile_max(&[]), [0.0, 0.0]);
    }

    #[test]
    fn ties_in_the_tile_max_keep_the_first_velocity() {
        assert_eq!(tile_max(&[[3.0, 0.0], [0.0, 3.0]]), [3.0, 0.0]);
    }

    #[test]
    fn the_neighbour_max_covers_all_eight_neighbours_and_the_centre() {
        // 3x3 tiles, a fast velocity in each position in turn; the centre tile sees all of them.
        let tiles_x = 3;
        let tiles_y = 3;
        for fast in 0..9usize {
            let mut maxima = [[1.0, 0.0]; 9];
            maxima[fast] = [10.0, 0.0];
            let got = neighbor_max(&maxima, tiles_x, tiles_y, 1, 1);
            assert_eq!(got, [10.0, 0.0], "fast tile {fast}");
        }
    }

    #[test]
    fn the_edges_of_the_image_have_fewer_neighbours_without_a_panic() {
        let maxima = [[0.0, 0.0], [4.0, 0.0], [0.0, 0.0], [0.0, 0.0]];
        // Top-left tile sees only the 2x2 block; the fast tile at (1, 0) is inside it.
        assert_eq!(neighbor_max(&maxima, 2, 2, 0, 0), [4.0, 0.0]);
        // Out-of-range tile index gives zero.
        assert_eq!(neighbor_max(&maxima, 2, 2, 5, 0), [0.0, 0.0]);
        assert_eq!(neighbor_max(&[], 0, 0, 0, 0), [0.0, 0.0]);
    }

    #[test]
    fn a_neighbour_too_slow_to_reach_the_pixel_has_zero_weight() {
        // Reach is 10 px * 0.5 = 5 px; a pixel 8 px away is out of it.
        let w = spread_weight([10.0, 0.0], 8.0, 1.0, 1.0, 0.5, SPREAD_SOFT_Z);
        assert_eq!(w, 0.0);
        assert!(spread_weight([10.0, 0.0], 2.0, 1.0, 1.0, 0.5, SPREAD_SOFT_Z) > 0.0);
    }

    #[test]
    fn a_neighbour_behind_the_pixel_does_not_blur_it() {
        let soft_z = SPREAD_SOFT_Z;
        let depth_p = 5.0;
        let behind = depth_p + soft_z + 0.1;
        assert_eq!(
            spread_weight([10.0, 0.0], 1.0, depth_p, behind, 1.0, soft_z),
            0.0
        );
        // Just inside the slack it still counts.
        let inside = depth_p + soft_z - 0.01;
        assert!(spread_weight([10.0, 0.0], 1.0, depth_p, inside, 1.0, soft_z) > 0.0);
    }

    #[test]
    fn the_weight_falls_smoothly_to_zero_at_the_reach_limit() {
        // Reach 40 px, so the 0.15 bound per pixel is the one the falloff must keep.
        let mut previous = spread_weight([40.0, 0.0], 0.0, 1.0, 1.0, 1.0, SPREAD_SOFT_Z);
        assert!((previous - 1.0).abs() < 1e-6);
        for d in 1..=40 {
            let w = spread_weight([40.0, 0.0], d as f32, 1.0, 1.0, 1.0, SPREAD_SOFT_Z);
            assert!(w <= previous + 1e-6, "not monotone at {d}");
            assert!(
                previous - w <= 0.15 + 1e-4,
                "jump at {d}: {previous} -> {w}"
            );
            previous = w;
        }
        assert_eq!(previous, 0.0);
    }
}
