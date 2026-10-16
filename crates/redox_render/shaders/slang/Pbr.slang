// RedOx Engine — Slang Modular PBR Shader
// Vertex and Fragment pipelines composed from Common, Camera, Brdf, and Light modules.
module Pbr;

import Common;
import Camera;
import Brdf;
import Light;

struct ModelUniform
{
    float4x4 model;
    float4x4 normal_mat;
};

ParameterBlock<ModelUniform> gModel;

struct VSInput
{
    float3 position : POSITION;
    float3 normal   : NORMAL;
    float2 uv       : TEXCOORD0;
};

struct VSOutput
{
    float4 position_clip : SV_Position;
    float3 world_pos     : POSITION;
    float3 normal        : NORMAL;
    float2 uv            : TEXCOORD0;
};

[shader("vertex")]
VSOutput vsMain(VSInput input)
{
    VSOutput output;
    float4 world_pos = mul(gModel.model, float4(input.position, 1.0));
    output.world_pos = world_pos.xyz;
    output.normal = normalize(mul(float3x3(gModel.normal_mat), input.normal));
    output.uv = input.uv;
    output.position_clip = mul(gCamera.view_proj, world_pos);
    return output;
}

struct MaterialUniform
{
    float4 base_color_factor;
    float  metallic_factor;
    float  roughness_factor;
    float2 _padding;
};

ParameterBlock<MaterialUniform> gMaterial;
ParameterBlock<DirectionalLightData> gSun;

[shader("fragment")]
float4 fsMain(VSOutput input) : SV_Target
{
    Surface surface;
    surface.albedo = gMaterial.base_color_factor.rgb;
    surface.metallic = saturate(gMaterial.metallic_factor);
    surface.roughness = clamp(gMaterial.roughness_factor, 0.04, 1.0);
    surface.normal = normalize(input.normal);
    surface.view_dir = normalize(gCamera.camera_pos.xyz - input.world_pos);
    surface.occlusion = 1.0;

    CookTorranceBRDF brdf;
    float3 direct_lighting = evaluateDirectionalLight(brdf, surface, gSun);

    // Simple ambient fallback
    float3 ambient = float3(0.03) * surface.albedo;

    float3 final_color = direct_lighting + ambient;
    return float4(final_color, gMaterial.base_color_factor.a);
}
