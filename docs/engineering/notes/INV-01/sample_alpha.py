import bpy
import json
import os

S = "C:/Users/yann1n/AppData/Local/Temp/claude/3f7c0d1712cd1c53/ea830a79-3409-4e19-a140-ddd9ff1bba96/scratchpad/"
rows = json.load(open(S + "twig_tris.json"))
img = bpy.data.images.load(S + "glb_twigs_diff_alpha.png", check_existing=False)
w, h = img.size
px = img.pixels[:]
ch = img.channels


def alpha_at(u, v):
    x = min(w - 1, max(0, int(u * w)))
    y = min(h - 1, max(0, int((1.0 - v) * h)))  # glTF v=0 is the top row
    return px[(y * w + x) * ch + 3]


total = sum(r["area3d"] for r in rows)
opaque = []
for r in rows:
    a = alpha_at(r["cu"], r["cv"])
    r["alpha"] = a
    if a >= 0.5:
        opaque.append(r)
print("TOTAL_TRIS", len(rows), "total_area3d %.3f" % total)
print("OPAQUE_CENTROID_TRIS", len(opaque), "area share %.3f" % (sum(r["area3d"] for r in opaque) / total))
areas = sorted((r["area3d"] for r in rows), reverse=True)
p99 = areas[len(areas) // 100]
big = [r for r in rows if r["area3d"] >= p99]
print("BIG_TRIS (top 1%% by 3D area, >= %.4f): %d, on opaque texels: %d" % (
    p99, len(big), sum(1 for r in big if r["alpha"] >= 0.5)))
top = sorted(opaque, key=lambda r: -r["area3d"])[:10]
for r in top:
    print("TOP_OPAQUE area3d %.4f cu %.3f cv %.3f cy %.3f alpha %.2f" % (
        r["area3d"], r["cu"], r["cv"], r["cy"], r["alpha"]))
# Opaque texels by U band: fraction of triangles in each 0.1 band that are opaque
for band in range(10):
    lo, hi = band / 10, (band + 1) / 10
    sel = [r for r in rows if lo <= r["cu"] < hi]
    if sel:
        o = sum(1 for r in sel if r["alpha"] >= 0.5)
        print("BAND u %.1f-%.1f tris %d opaque %d" % (lo, hi, len(sel), o))
