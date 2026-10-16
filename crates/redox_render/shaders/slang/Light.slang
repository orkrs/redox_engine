// RedOx Engine — Slang Lighting Module
// Supports Directional Light and Point Lights with physical falloff.
module Light;

import Common;
import Brdf;

public struct DirectionalLightData
{
    public float3 direction;  // normalized towards light
    public float  intensity;
    public float3 color;
    public float  source_angle;
};

public struct PointLightData
{
    public float3 position;
    public float  radius;
    public float3 color;
    public float  intensity;
};

/// Safe physical quadratic distance attenuation (no infinity at dist = 0).
public float smoothDistanceAttenuation(float dist, float radius)
{
    float num = saturate(1.0 - pow(dist / radius, 4.0));
    return (num * num) / (dist * dist + 1.0);
}

/// Evaluates a directional light against a surface using a generic BRDF.
public float3 evaluateDirectionalLight<T : IBRDF>(T brdf, Surface s, DirectionalLightData light)
{
    float3 light_color = light.color * light.intensity;
    return brdf.evaluate(s, light.direction, light_color);
}

/// Evaluates a point light with smooth range attenuation.
public float3 evaluatePointLight<T : IBRDF>(T brdf, Surface s, float3 world_pos, PointLightData light)
{
    float3 to_light = light.position - world_pos;
    float dist = length(to_light);
    if (dist >= light.radius || dist < 1e-4)
    {
        return float3(0.0);
    }
    float3 light_dir = to_light / dist;
    float atten = smoothDistanceAttenuation(dist, light.radius);
    float3 light_color = light.color * (light.intensity * atten);
    return brdf.evaluate(s, light_dir, light_color);
}
