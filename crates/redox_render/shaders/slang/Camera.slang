// RedOx Engine — Slang Camera Module
// ParameterBlock with 16-byte alignment and Reverse-Z projection.
module Camera;

import Common;

/// GPU uniform representation of camera matrices.
/// Matches `CameraUniform` layout in Rust with strict 16-byte alignment.
public struct CameraUniform
{
    public float4x4 view_proj;
    public float4   camera_pos; // xyz = position, w = padding
    public float4x4 view;
    public float4x4 proj;
    public float4x4 inv_view;
    public float4x4 inv_proj;
    public float4x4 prev_view_proj;
};

/// Camera parameter block bound to WGPU bind group 0.
public ParameterBlock<CameraUniform> gCamera;

/// Reconstructs view-space position from depth and screen UV (Reverse-Z compatible).
public float3 reconstructViewPos(float2 uv, float depth)
{
    float4 clip = float4(uv.x * 2.0 - 1.0, (1.0 - uv.y) * 2.0 - 1.0, depth, 1.0);
    float4 viewPos = mul(gCamera.inv_proj, clip);
    return viewPos.xyz / viewPos.w;
}

/// Reconstructs world-space position from depth and screen UV.
public float3 reconstructWorldPos(float2 uv, float depth)
{
    float4 clip = float4(uv.x * 2.0 - 1.0, (1.0 - uv.y) * 2.0 - 1.0, depth, 1.0);
    float4 worldPos = mul(mul(gCamera.inv_view, gCamera.inv_proj), clip);
    return worldPos.xyz / worldPos.w;
}
