import bpy

S = "C:/Users/yann1n/AppData/Local/Temp/claude/3f7c0d1712cd1c53/ea830a79-3409-4e19-a140-ddd9ff1bba96/scratchpad/"
img = bpy.data.images.load(S + "glb_twigs_diff_alpha.png", check_existing=False)
w, h = img.size
px = img.pixels[:]
ch = img.channels


def texel(u, v):
    x = min(w - 1, max(0, int(u * w)))
    y = min(h - 1, max(0, int((1.0 - v) * h)))
    i = (y * w + x) * ch
    return px[i], px[i + 1], px[i + 2], px[i + 3]


for (u, v) in [(0.714, 0.722), (0.556, 0.609), (0.45, 0.45), (0.25, 0.25)]:
    r, g, b, a = texel(u, v)
    print("PROBE uv (%.3f, %.3f) rgb (%.3f %.3f %.3f) alpha %.3f" % (u, v, r, g, b, a))

# Window around the big-triangle patch: alpha and mean RGB over an 80x80 px box (in UV ~0.08)
def window(u0, v0, span):
    n = 0
    sa = sr = sg = sb = 0.0
    op = 0
    steps = 20
    for iy in range(steps + 1):
        for ix in range(steps + 1):
            u = u0 + span * ix / steps
            v = v0 + span * iy / steps
            r, g, b, a = texel(u, v)
            sr += r; sg += g; sb += b; sa += a
            op += 1 if a >= 0.5 else 0
            n += 1
    print("WINDOW uv0 (%.3f,%.3f) span %.3f: alpha mean %.3f opaque frac %.3f rgb mean (%.3f %.3f %.3f)" % (
        u0, v0, span, sa / n, op / n, sr / n, sg / n, sb / n))


window(0.674, 0.682, 0.08)
window(0.516, 0.569, 0.08)
