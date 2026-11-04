import json
import struct
import sys
import math
from collections import Counter

COMP = {5120: ('b', 1), 5121: ('B', 1), 5122: ('h', 2), 5123: ('H', 2), 5125: ('I', 4), 5126: ('f', 4)}
NCOMP = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4, 'MAT4': 16}


def read_glb(path):
    with open(path, 'rb') as f:
        data = f.read()
    magic, version, length = struct.unpack_from('<III', data, 0)
    assert magic == 0x46546C67, 'not glb'
    off = 12
    js = None
    binc = None
    while off < len(data):
        clen, ctype = struct.unpack_from('<II', data, off)
        chunk = data[off + 8: off + 8 + clen]
        if ctype == 0x4E4F534A:
            js = json.loads(chunk.decode('utf-8'))
        elif ctype == 0x004E4942:
            binc = chunk
        off += 8 + clen
    return js, binc


def accessor(js, binc, idx):
    acc = js['accessors'][idx]
    bv = js['bufferViews'][acc['bufferView']]
    fmt, size = COMP[acc['componentType']]
    n = NCOMP[acc['type']]
    stride = bv.get('byteStride', size * n)
    base = bv.get('byteOffset', 0) + acc.get('byteOffset', 0)
    out = []
    for i in range(acc['count']):
        o = base + i * stride
        out.append(struct.unpack_from('<' + fmt * n, binc, o))
    return out


def main(path, mat_substr):
    js, binc = read_glb(path)
    print('file', path)
    print('extensionsUsed', js.get('extensionsUsed'))
    print('images', [(i.get('name'), i.get('mimeType')) for i in js.get('images', [])])
    print('materials:')
    for mi, m in enumerate(js.get('materials', [])):
        print(' ', mi, m.get('name'), 'alphaMode=', m.get('alphaMode', 'OPAQUE'),
              'doubleSided=', m.get('doubleSided', False))
    for mesh in js['meshes']:
        print('mesh', mesh.get('name'))
        for p in mesh['primitives']:
            mname = js['materials'][p['material']]['name'] if 'material' in p else None
            attrs = p['attributes']
            print('  prim material=%s attrs=%s indices=%s' % (
                mname, sorted(attrs.keys()), p.get('indices')))
            for k in sorted(attrs):
                a = js['accessors'][attrs[k]]
                print('    %s count=%d type=%s min=%s max=%s' % (k, a['count'], a['type'], a.get('min'), a.get('max')))
            if mat_substr and mname and mat_substr in mname:
                analyse(js, binc, p)


def analyse(js, binc, p):
    pos = accessor(js, binc, p['attributes']['POSITION'])
    uv = accessor(js, binc, p['attributes']['TEXCOORD_0']) if 'TEXCOORD_0' in p['attributes'] else None
    col0 = accessor(js, binc, p['attributes']['COLOR_0']) if 'COLOR_0' in p['attributes'] else None
    col1 = accessor(js, binc, p['attributes']['COLOR_1']) if 'COLOR_1' in p['attributes'] else None
    idx = [t[0] for t in accessor(js, binc, p['indices'])] if 'indices' in p else list(range(len(pos)))
    tris = [idx[i:i + 3] for i in range(0, len(idx), 3)]
    print('  triangles', len(tris))
    if uv is None:
        print('  no TEXCOORD_0')
        return
    uvmin = [min(u[0] for u in uv), min(u[1] for u in uv)]
    uvmax = [max(u[0] for u in uv), max(u[1] for u in uv)]
    print('  UV range over all verts min', uvmin, 'max', uvmax)
    outside = 0
    areas3d, areas_uv = [], []
    for t in tris:
        a, b, c = [pos[i] for i in t]
        e1 = [b[k] - a[k] for k in range(3)]
        e2 = [c[k] - a[k] for k in range(3)]
        cr = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]]
        areas3d.append(0.5 * math.sqrt(sum(x * x for x in cr)))
        ua, ub, uc = [uv[i] for i in t]
        du1, dv1 = ub[0] - ua[0], ub[1] - ua[1]
        du2, dv2 = uc[0] - ua[0], uc[1] - ua[1]
        areas_uv.append(0.5 * abs(du1 * dv2 - du2 * dv1))
        if any(not (0.0 <= x <= 1.0 and 0.0 <= y <= 1.0) for x, y in (ua, ub, uc)):
            outside += 1
    print('  triangles with a vertex UV outside [0,1]:', outside)
    if areas3d:
        print('  3D area total %.4f, max %.6f' % (sum(areas3d), max(areas3d)))
        print('  UV area total %.6f, max %.6f' % (sum(areas_uv), max(areas_uv)))
    # Triangle centre UV histogram (u in 10 bins, v in 10 bins)
    hist = Counter()
    for t in tris:
        cu = sum(uv[i][0] for i in t) / 3.0
        cv = sum(uv[i][1] for i in t) / 3.0
        hist[(min(9, max(0, int(cu * 10))), min(9, max(0, int(cv * 10))))] += 1
    print('  centroid UV histogram (rows = v bins 0..9 bottom-up, cols = u bins 0..9):')
    for vb in range(9, -1, -1):
        print('   v%.1f ' % (vb / 10) + ' '.join('%5d' % hist.get((ub, vb), 0) for ub in range(10)))
    for name, col in (('COLOR_0', col0), ('COLOR_1', col1)):
        if col is None:
            print('  no', name)
            continue
        mins = [min(c[k] for c in col) for k in range(len(col[0]))]
        maxs = [max(c[k] for c in col) for k in range(len(col[0]))]
        print('  %s min %s max %s' % (name, mins, maxs))


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else None)
