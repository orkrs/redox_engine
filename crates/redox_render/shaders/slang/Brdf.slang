// RedOx Engine — Slang BRDF Module
// Generic BRDF interface and Cook-Torrance Microfacet implementation.
module Brdf;

import Common;

/// Material surface properties for lighting evaluation.
public struct Surface
{
    public float3 albedo;
    public float  metallic;
    public float  roughness;
    public float3 normal;      // normalized world-space normal
    public float3 view_dir;    // normalized surface -> camera vector
    public float  occlusion;
};

/// Interface defining a physically-based bidirectional reflectance distribution function.
public interface IBRDF
{
    float3 evaluate(Surface s, float3 light_dir, float3 light_color);
};

/// Standard Cook-Torrance microfacet specular BRDF with Lambertian diffuse.
public struct CookTorranceBRDF : IBRDF
{
    /// GGX / Trowbridge-Reitz Normal Distribution Function (NDF).
    public static float distributionGGX(float NdotH, float roughness)
    {
        float a = roughness * roughness;
        float a2 = a * a;
        float NdotH2 = NdotH * NdotH;
        float denom = (NdotH2 * (a2 - 1.0) + 1.0);
        return a2 / (PI * denom * denom + 1e-7);
    }

    /// Schlick-GGX Geometric Shadowing function.
    public static float geometrySchlickGGX(float NdotV, float roughness)
    {
        float r = (roughness + 1.0);
        float k = (r * r) / 8.0;
        return NdotV / (NdotV * (1.0 - k) + k + 1e-7);
    }

    /// Smith model combining light and view geometric shadowing.
    public static float geometrySmith(float NdotV, float NdotL, float roughness)
    {
        float ggx2 = geometrySchlickGGX(NdotV, roughness);
        float ggx1 = geometrySchlickGGX(NdotL, roughness);
        return ggx1 * ggx2;
    }

    /// Fresnel-Schlick approximation with F0 base reflectivity.
    public static float3 fresnelSchlick(float cosTheta, float3 F0)
    {
        return F0 + (1.0 - F0) * pow(saturate(1.0 - cosTheta), 5.0);
    }

    /// Evaluates the complete Cook-Torrance BRDF for a directional or point light source.
    public float3 evaluate(Surface s, float3 light_dir, float3 light_color)
    {
        float3 H = normalize(s.view_dir + light_dir);
        float NdotL = saturate(dot(s.normal, light_dir));
        float NdotV = saturate(dot(s.normal, s.view_dir));
        float NdotH = saturate(dot(s.normal, H));
        float VdotH = saturate(dot(s.view_dir, H));

        if (NdotL <= 0.0 || NdotV <= 0.0)
        {
            return float3(0.0);
        }

        float3 F0 = lerp(float3(0.04), s.albedo, s.metallic);
        float D = distributionGGX(NdotH, s.roughness);
        float G = geometrySmith(NdotV, NdotL, s.roughness);
        float3 F = fresnelSchlick(VdotH, F0);

        float3 numerator = D * G * F;
        float denominator = 4.0 * NdotV * NdotL + 0.0001;
        float3 specular = numerator / denominator;

        float3 kS = F;
        float3 kD = (float3(1.0) - kS) * (1.0 - s.metallic);

        float3 diffuse = (kD * s.albedo) * INV_PI;
        return (diffuse + specular) * light_color * NdotL;
    }
};
