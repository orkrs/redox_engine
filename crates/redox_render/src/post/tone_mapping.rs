//! Tone mapping and gamma correction post-processing pass.

use crate::shader::manager::create_shader_module;

/// Creates the render pipeline for the tone mapping pass.
///
/// This pipeline takes an HDR texture (Rgba16Float) and outputs to the
/// surface format (Srgb-compatible) after applying tone mapping and gamma correction.
pub fn create_tone_mapping_pipeline(
    device: &wgpu::Device,
    output_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = create_shader_module(device, "tone_mapping_shader", TONE_MAPPING_SHADER_SRC);

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("tone_mapping_pipeline_layout"),
        bind_group_layouts: &[
            &device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("tone_mapping_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            }),
        ],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("tone_mapping_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vs_main",
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    })
}

const TONE_MAPPING_SHADER_SRC: &str = r#"
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) in_vertex_index: u32,
) -> VertexOutput {
    var out: VertexOutput;
    // Fullscreen triangle trick
    let x = f32(i32(in_vertex_index) << 1u & 2) * 2.0 - 1.0;
    let y = f32(i32(in_vertex_index) & 2) * 2.0 - 1.0;
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

@group(0) @binding(0) var t_hdr: texture_2d<f32>;
@group(0) @binding(1) var s_hdr: sampler;

fn luma(c: vec3<f32>) -> f32 {
    // Rec.709 luma (linear)
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(t_hdr));
    let texel = 1.0 / max(dims, vec2<f32>(1.0));

    // 5-tap constrained sharpen in HDR space (keeps TAA from looking blurry).
    let c  = textureSample(t_hdr, s_hdr, in.uv).rgb;
    let n  = textureSample(t_hdr, s_hdr, in.uv + vec2<f32>(0.0, -texel.y)).rgb;
    let s  = textureSample(t_hdr, s_hdr, in.uv + vec2<f32>(0.0,  texel.y)).rgb;
    let e  = textureSample(t_hdr, s_hdr, in.uv + vec2<f32>( texel.x, 0.0)).rgb;
    let w  = textureSample(t_hdr, s_hdr, in.uv + vec2<f32>(-texel.x, 0.0)).rgb;

    let blur = (n + s + e + w) * 0.25;
    let high = c - blur;

    // Strength: tune as needed. Higher -> sharper but more ringing/shimmer.
    let strength = 0.25;

    // Limit sharpening in flat areas to avoid shimmering.
    let lc = luma(c);
    let ln = luma(n);
    let ls = luma(s);
    let le = luma(e);
    let lw = luma(w);
    let lmin = min(lc, min(min(ln, ls), min(le, lw)));
    let lmax = max(lc, max(max(ln, ls), max(le, lw)));
    let contrast = lmax - lmin;
    let adapt = smoothstep(0.02, 0.20, contrast);

    var hdr_color = c + high * (strength * adapt);

    // Constrain to local min/max to suppress ringing.
    let cmin = min(c, min(min(n, s), min(e, w)));
    let cmax = max(c, max(max(n, s), max(e, w)));
    hdr_color = clamp(hdr_color, cmin, cmax);

    // Reinhard tone mapping: map [0, ∞) → [0, 1) in linear space.
    // Do NOT apply manual gamma here – the sRGB render target performs the
    // linear→sRGB conversion automatically, so a second pow(x, 1/2.2) would
    // give double-gamma and break perceptual accuracy.
    let mapped = hdr_color / (hdr_color + vec3<f32>(1.0));

    return vec4<f32>(mapped, 1.0);
}
"#;
