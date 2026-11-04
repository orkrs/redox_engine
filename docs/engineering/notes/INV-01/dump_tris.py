import json
import math
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from glb_inspect import read_glb, accessor  # noqa: E402

GLB = "C:/Users/yann1n/RustroverProjects/redox_engine/.claude/worktrees/agent-aeb2e22eb4208529d/assets/forest/models/sapling_a.glb"
OUT = os.path.dirname(os.path.abspath(__file__))

js, binc = read_glb(GLB)
rows = []
for mesh in js["meshes"]:
    for p in mesh["primitives"]:
        if js["materials"][p["material"]]["name"] != "fir_sapling_medium_twigs":
            continue
        pos = accessor(js, binc, p["attributes"]["POSITION"])
        uv = accessor(js, binc, p["attributes"]["TEXCOORD_0"])
        idx = [t[0] for t in accessor(js, binc, p["indices"])]
        for k in range(0, len(idx), 3):
            t = idx[k:k + 3]
            a, b, c = [pos[i] for i in t]
            e1 = [b[j] - a[j] for j in range(3)]
            e2 = [c[j] - a[j] for j in range(3)]
            cr = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]]
            a3 = 0.5 * math.sqrt(sum(x * x for x in cr))
            ua, ub, uc = [uv[i] for i in t]
            rows.append({
                "area3d": a3,
                "cu": (ua[0] + ub[0] + uc[0]) / 3,
                "cv": (ua[1] + ub[1] + uc[1]) / 3,
                "cy": sum(pos[i][1] for i in t) / 3,
            })
with open(os.path.join(OUT, "twig_tris.json"), "w") as f:
    json.dump(rows, f)
print("TRIS", len(rows))
