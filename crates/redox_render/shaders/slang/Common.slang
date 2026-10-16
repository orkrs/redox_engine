// RedOx Engine — Slang Common Module
// Foundation for math, coordinate spaces, and packing utilities.
module Common;

public static const float PI = 3.14159265358979323846;
public static const float INV_PI = 0.31830988618379067154;
public static const float EPSILON = 1e-5;

/// Clamps value to [0.0, 1.0] without divergence.
public float saturate(float x)
{
    return clamp(x, 0.0, 1.0);
}

public float3 saturate(float3 v)
{
    return clamp(v, float3(0.0), float3(1.0));
}

/// Octahedral normal encoding into 2 floats in [-1, 1].
public float2 octEncode(float3 n)
{
    float l1norm = dot(abs(n), float3(1.0));
    float2 p = n.xy * (1.0 / l1norm);
    if (n.z < 0.0)
    {
        p = (1.0 - abs(p.yx)) * sign(p);
    }
    return p;
}

/// Octahedral normal decoding from 2 floats in [-1, 1].
public float3 octDecode(float2 f)
{
    float3 n = float3(f.x, f.y, 1.0 - abs(f.x) - abs(f.y));
    float t = saturate(-n.z);
    n.xy += select(n.xy >= 0.0, -t, t);
    return normalize(n);
}

/// Linear RGB to perceptual sRGB.
public float3 linearToSrgb(float3 c)
{
    return select(c <= 0.0031308, c * 12.92, 1.055 * pow(max(c, 0.0), float3(1.0 / 2.4)) - 0.055);
}

/// sRGB to linear RGB.
public float3 srgbToLinear(float3 c)
{
    return select(c <= 0.04045, c / 12.92, pow((c + 0.055) / 1.055, float3(2.4)));
}
