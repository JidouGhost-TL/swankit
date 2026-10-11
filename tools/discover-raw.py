#!/usr/bin/env python3
"""Title-independent asset discovery from one emulator frame (pilot tooling).

Inputs: ROM, a 64 KiB work-RAM dump (tile data, maps, colour palettes) and the
256 I/O ports of the same frame (display mode, map bases, scroll, mono shades).
For every visible background layer it finds the ROM bytes that reached VRAM
byte-identically (raw, uncompressed storage): runs of tiles, the map rows and
the colour palettes. Output: a swankit-asset-map/1 document with a composed
screen. Evidence = trace (frame dump); confidence medium (no loader cited).
Compressed or code-generated data is reported as not found, never guessed.
Hardware: WSdev Display/IO_Ports, Display/Screen, Display/Tile_Data, Palette."""
import argparse, hashlib, json, sys

ap = argparse.ArgumentParser()
ap.add_argument('rom'); ap.add_argument('ram'); ap.add_argument('ports'); ap.add_argument('out')
ap.add_argument('--label', default=''); ap.add_argument('--frame', default='?'); ap.add_argument('--prefix', default='title')
a = ap.parse_args()
rom, ram, ports = (open(p, 'rb').read() for p in (a.rom, a.ram, a.ports))
hx = lambda n: f'0x{n:x}'
mode = ports[0x60]
color, bpp4, packed = bool(mode & 0x80), bool(mode & 0x40), bool(mode & 0x20)
tb = 32 if bpp4 else 16
tile_base = 0x4000 if bpp4 else 0x2000
report = {'mode': hex(mode), 'layers': {}}

def map_words(base):
    return [ram[base + 2 * i] | ram[base + 2 * i + 1] << 8 for i in range(32 * 32)]

def slot_of(w):
    return (w & 0x1ff) | ((w & 0x2000) >> 4 if color else 0)

def tile_bytes(slot):
    o = tile_base + slot * tb
    return ram[o:o + tb]

def find_all(needle, limit=64):
    out, i = [], rom.find(needle)
    while i >= 0 and len(out) < limit:
        out.append(i); i = rom.find(needle, i + 1)
    return out

assets, ids = [], set()
def add(asset):
    if asset['id'] in ids:
        return False
    ids.add(asset['id']); assets.append(asset); return True

evidence = {'trace': f'emulator frame {a.frame}: work RAM + I/O port dump; bytes matched byte-identically in ROM',
            'notes': 'discovered by VRAM-to-ROM matching (no loader routine cited)'}
layers, used_palettes, tile_rel = [], set(), {}
for name, enable_bit, base, sx, sy in (('scr1', 1, (ports[0x07] & 0x0f) << 11, ports[0x10], ports[0x11]),
                                       ('scr2', 2, (ports[0x07] >> 4 & 0x0f) << 11, ports[0x12], ports[0x13])):
    if not ports[0x00] & enable_bit:
        continue
    words = map_words(base)
    c0, r0 = sx // 8, sy // 8
    vis = [(c % 32, r % 32) for r in range(r0, r0 + 18 + (sy % 8 > 0)) for c in range(c0, c0 + 28 + (sx % 8 > 0))]
    used = sorted({slot_of(words[r * 32 + c]) for c, r in vis})
    for c, r in vis:
        used_palettes.add(words[r * 32 + c] >> 9 & 15)
    info = {'map_base': hex(base), 'scroll': [sx, sy], 'tiles_used': len(used)}
    # ---- tiles: longest byte-identical ROM runs covering used slots
    covered, runs = set(), []
    for slot in used:
        if slot in covered:
            continue
        t = tile_bytes(slot)
        if len(set(t)) <= 1:
            continue  # uniform tiles match anywhere; they ride along with runs
        best = None
        for o in find_all(t):
            k0 = 0
            while slot - k0 - 1 >= 0 and o - (k0 + 1) * tb >= 0 and rom[o - (k0 + 1) * tb:o - k0 * tb] == tile_bytes(slot - k0 - 1):
                k0 += 1
            k1 = 1
            while slot + k1 < 1024 and rom[o + k1 * tb:o + (k1 + 1) * tb] == tile_bytes(slot + k1) and o + (k1 + 1) * tb <= len(rom):
                k1 += 1
            span = set(range(slot - k0, slot + k1))
            score = len(span & set(used))
            if best is None or score > best[0]:
                best = (score, o - k0 * tb, slot - k0, k0 + k1)
        if best and (best[3] > 1 or len(set(t)) >= 4):
            _, off, first, count = best
            covered |= set(range(first, first + count))
            runs.append((off, first, count))
    unresolved = [s for s in used if s not in covered and len(set(tile_bytes(s))) > 1]
    info['tile_runs'] = len(runs); info['tiles_unlocated'] = len(unresolved)
    for off, first, count in runs:
        aid = f'{a.prefix}.tiles.{off:06x}'
        add({'id': aid, 'kind': 'tiles', 'offset': hx(off), 'length': hx(count * tb),
             'format': {'bpp': 4 if bpp4 else 2, 'layout': 'packed' if packed else 'planar', 'width': '0x10'},
             'confidence': 'medium', 'evidence': dict(evidence, notes=f'{count} tiles = VRAM slots {first}-{first + count - 1}')})
        tile_rel[aid] = first
    # ---- map: visible rectangle rows, as a full 32-wide map or a packed rectangle
    rowbytes = lambda r, c, n: b''.join(words[(r % 32) * 32 + (cc % 32)].to_bytes(2, 'little') for cc in range(c, c + n))
    found = None
    for r in range(r0, r0 + 18):
        for width, col in ((32, 0), (28, c0), (30, c0), (32, c0)):
            needle = rowbytes(r, col, width)
            if len(set(needle)) <= 2:
                continue
            for o in find_all(needle, 16):
                n = 1
                while n < 32 and rom[o + n * width * 2:o + (n + 1) * width * 2] == rowbytes(r + n, col, width):
                    n += 1
                k = 0  # rows above the probe row
                while r - k - 1 >= 0 and o - (k + 1) * width * 2 >= 0 and rom[o - (k + 1) * width * 2:o - k * width * 2] == rowbytes(r - k - 1, col, width):
                    k += 1
                cand = (n + k, o - k * width * 2, width, col, r - k)
                if found is None or cand[0] > found[0]:
                    found = cand
        if found and found[0] >= 4:
            break
    if found:
        rows, off, width, col, row = found
        mid = f'{a.prefix}.{name}.{off:06x}'
        add({'id': mid, 'kind': 'tilemap', 'offset': hx(off), 'length': hx(rows * width * 2),
             'format': {'width': hx(width), 'height': hx(rows), 'entry': 'ws-word', 'color': color},
             'relations': {}, 'confidence': 'medium',
             'evidence': dict(evidence, notes=f'{name} map cells ({col},{row}) {width}x{rows}, map base {base:#x}')})
        layers.append({'tilemap': mid, 'x': col * 8 - sx, 'y': row * 8 - sy})
        info['map'] = f'{width}x{rows} at {off:#x}'
    else:
        info['map'] = 'not found raw in ROM'
    report['layers'][name] = info

# ---- palettes
pal_assets = []
if color:
    done_p = set()
    for p in sorted(used_palettes):
        if p in done_p:
            continue
        data = ram[0xfe00 + 32 * p:0xfe20 + 32 * p]
        hits = find_all(data, 4)
        if hits:
            # extend to the longest run of consecutive palettes stored together
            o, first, n = hits[0], p, 1
            while first + n < 16 and rom[o + 32 * n:o + 32 * (n + 1)] == ram[0xfe00 + 32 * (first + n):0xfe20 + 32 * (first + n)]:
                n += 1
            done_p |= set(range(first, first + n))
            pid = f'{a.prefix}.palette.{o:06x}'
            if add({'id': pid, 'kind': 'palette', 'offset': hx(o), 'length': hx(32 * n),
                    'format': {'palette_format': 'rgb444', 'palette_index': hx(first)}, 'confidence': 'medium',
                    'evidence': dict(evidence, notes=f'palettes {first}-{first + n - 1} at colour RAM FE00')}):
                pal_assets.append(pid)
    missing = [p for p in sorted(used_palettes) if not any(
        int(x['format']['palette_index'], 16) <= p < int(x['format']['palette_index'], 16) + int(x['length'], 16) // 32
        for x in assets if x['kind'] == 'palette')]
    report['palettes_unlocated'] = missing
    inline = {}
    if missing:
        cols = []
        for p in range(16):
            for c in range(16):
                w = ram[0xfe00 + 32 * p + 2 * c] | ram[0xfe00 + 32 * p + 2 * c + 1] << 8
                cols.append('#%02x%02x%02x' % ((w >> 8 & 15) * 17, (w >> 4 & 15) * 17, (w & 15) * 17))
        inline = {f'{a.prefix}.frame-palettes': {'colors': cols, 'evidence': f'colour RAM FE00 at frame {a.frame} (not located in ROM)'}}
else:
    levels = [(ports[0x1c + i // 2] >> (4 * (i % 2))) & 15 for i in range(8)]
    cols = []
    for p in range(16):
        for c in range(16):
            s = (ports[0x20 + 2 * p + c // 2] >> (4 * (c % 2))) & 7 if c < 4 else 0
            g = 255 - levels[s] * 17
            cols.append('#%02x%02x%02x' % (g, g, g))
    inline = {f'{a.prefix}.mono-palettes': {'colors': cols, 'evidence': f'ports 1C-3F at frame {a.frame} (shades set by code immediates)'}}
    report['palettes_unlocated'] = 'mono: shade ports written by code (inline palette)'

pal_names = list(inline) + pal_assets
for x in assets:
    if x['kind'] == 'tilemap':
        x['relations'] = {'tiles': [{'id': t, 'at': hx(s)} for t, s in sorted(tile_rel.items(), key=lambda kv: kv[1])],
                          'palettes': pal_names}
    if x['kind'] == 'tiles':
        x['relations'] = {'palette': pal_names[0]} if pal_names and not pal_assets else {}
spans = sorted((int(x['offset'], 16), int(x['offset'], 16) + int(x['length'], 16), x) for x in assets)
for i, (s0, e0, x0) in enumerate(spans):
    for s1, e1, x1 in spans[i + 1:]:
        if s1 < e0:
            x0.setdefault('overlaps', []).append(x1['id'])
if color:
    back = ram[0xfe00 + 2 * ports[0x01]] | ram[0xfe00 + 2 * ports[0x01] + 1] << 8
else:
    g = (15 - levels[ports[0x01] & 7]) * 17 // 17
    back = g << 8 | g << 4 | g
amap = {'schema': 'swankit-asset-map/1',
        'rom': {'sha256': hashlib.sha256(rom).hexdigest(), 'size': hx(len(rom)), 'label': a.label},
        **({'palettes': inline} if inline else {}),
        'coverage': {'notes': f'automatic VRAM-to-ROM discovery from frame {a.frame}; only the visible background layers'},
        'assets': assets,
        'screens': [{'id': f'screen.{a.prefix}', 'layers': layers,
                     'back_color': '#%02x%02x%02x' % ((back >> 8 & 15) * 17, (back >> 4 & 15) * 17, (back & 15) * 17),
                     'note': f'frame {a.frame}; sprites not composed'}]}
open(a.out, 'w').write(json.dumps(amap, indent=1) + '\n')
print(json.dumps(report, indent=1))
