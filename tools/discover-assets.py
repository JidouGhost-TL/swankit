#!/usr/bin/env python3
"""Normalize snapshots, runtime reads and decompile references for assets discover.

Family profiles are external JSON: assets, charmaps, observations and texts.
A read/write temporal link is a candidate; Rust checks complete decode equality.
No profile is selected from a title name. No free space is inferred.
"""
import argparse, bisect, csv, hashlib, json, pathlib, re, subprocess, tempfile


def addresses(decomp):
    result = []
    for p in sorted(decomp.glob('*.c')):
        m = re.match(r'ram_([0-9a-f]+)_([0-9a-f]+)_', p.name)
        if m:
            result.append((int(m[1], 16) * 16 + int(m[2], 16), p))
    return sorted(result)


def owner(pc, routines):
    i = bisect.bisect_right([x[0] for x in routines], pc) - 1
    if i < 0 or pc - routines[i][0] > 8192:
        return None
    return routines[i]


def references(rom, source, routine):
    """Candidate immediates in the cited decompile's local byte span.

    This is intentionally incomplete, never an instruction-boundary proof.
    Candidates must be corroborated by a literal in the decompile text.
    """
    if routine is None:
        return []
    pc, path = routine
    low = source & 65535
    text = path.read_text(errors='replace')
    if not re.search(r'\b0x0*' + format(low, 'x') + r'\b', text):
        return []
    start = (0xf00000 + pc) % len(rom)
    stop = min(start + 512, len(rom))
    found = []
    value = low.to_bytes(2, 'little')
    for at in range(start, stop - 2):
        # MOV r16, imm16 or PUSH imm16. Literal may still lie in another operand.
        if (0xb8 <= rom[at] <= 0xbf or rom[at] == 0x68) and rom[at+1:at+3] == value:
            found.append({'at': at+1, 'width': 2, 'kind': 'code-immediate',
                          'note': 'Candidate immediate corroborated by decompile literal; boundaries need listing review; incomplete.'})
    return found


def font_runs(rom, ram, ports, observations):
    """Recognize byte-per-row glyph runs expanded into a two-plane VRAM tile.

    At least six distinct contiguous glyphs and observed source reads are
    required. Geometric glyph storage is proven; typography needs visual review.
    """
    if ports[0x60] & 0x40:
        return []
    results = []
    used = set()
    for slot in range(512):
        if slot in used:
            continue
        tile = ram[0x2000 + slot * 16:0x2010 + slot * 16]
        for plane in (0, 1, 2):
            def rowbytes(s):
                t = ram[0x2000+s*16:0x2010+s*16]
                if len(t) != 16:
                    return None
                if plane == 0 and t[1::2] == bytes(8):
                    return t[::2]
                if plane == 1 and t[::2] == bytes(8):
                    return t[1::2]
                if plane == 2 and t[::2] == t[1::2]:
                    return t[::2]
                return None
            glyph = rowbytes(slot)
            if glyph is None or len(set(glyph)) < 4:
                continue
            off = rom.find(glyph)
            tries = 0
            while off >= 0 and tries < 32:
                n = 0
                while slot+n < 512 and n < 256 and rowbytes(slot+n) == rom[off+n*8:off+(n+1)*8]:
                    n += 1
                readers = [o for o in observations if o['source'] <= off+n*8 and o['source']+(o['length'] or 1) >= off]
                if n >= 6 and readers:
                    results.append({'id': f'font.rows8.{off:x}', 'kind': 'font',
                        'offset': off, 'length': n*8, 'format': {'bpp': 1, 'count': n,
                        'stride': 8, 'tiles_wide': 1, 'tiles_high': 1, 'width': 16},
                        'confidence': 'medium', 'evidence': {'loader': readers[0]['routine'],
                        'trace': f'{n} consecutive 8x8 one-plane glyph records equal expanded VRAM slots {slot} onward',
                        'notes': 'Storage expansion proven; glyph meanings and width tables remain unreviewed.'}})
                    used.update(range(slot, slot+n))
                    break
                tries += 1
                off = rom.find(glyph, off+1)
    return results


def table_texts(rom, plugin):
    result = []
    for table in plugin.get('text_tables', []):
        enc = table['encoding']
        width = enc['width']
        terminator = bytes.fromhex(table['terminator'])
        if not terminator or width not in (1, 2, 3, 4):
            raise ValueError('invalid pointer/text profile')
        if not 0 < table['count'] <= 32768 or table['stride'] < width:
            raise ValueError('pointer table bounds')
        for i in range(table['count']):
            at = table['offset'] + i*table['stride'] + table.get('pointer_offset', 0)
            raw = rom[at:at+width]
            if len(raw) != width:
                raise ValueError('truncated pointer table')
            value = int.from_bytes(raw, 'little' if enc.get('endian', 'little') == 'little' else 'big')
            if enc.get('form', 'linear') == 'far':
                offset, segment = value & 65535, value >> 16
                target = table['window'] + (segment-table['segment_base'])*16 + offset - enc.get('add', 0)
            else:
                target = (value << enc.get('shift', 0)) - enc.get('add', 0) + table.get('window', 0)
            if not 0 <= target < len(rom):
                raise ValueError('pointer target outside ROM')
            unit = table.get('unit_width', 1)
            if unit not in (1, 2):
                raise ValueError('text code-unit width')
            stop = min(len(rom), target+table.get('max_length', 4096))
            end = next((p+len(terminator) for p in range(target, stop-len(terminator)+1, unit)
                        if rom[p:p+len(terminator)] == terminator), None)
            if end is None:
                continue
            reference = dict(enc, at=at, kind='pointer-table-entry')
            if enc.get('form') == 'far':
                reference.update(window=table['window'], segment_base=table['segment_base'],
                                 normalize=table.get('normalize', 'segment'))
            elif table.get('window', 0):
                reference['add'] = enc.get('add', 0)-table['window']
            result.append({'offset': target, 'length': end-target, 'charmap': table['charmap'],
                           'routine': table['routine'], 'references': [reference],
                           'font': table.get('font')})
    return result

def prepare(rom_path, ram_path, ports_path, flow, decomp, profile, out, binary):
    rom = rom_path.read_bytes()
    ram = ram_path.read_bytes()
    routines = addresses(decomp) if decomp else []
    with tempfile.TemporaryDirectory() as tmp:
        raw = pathlib.Path(tmp) / 'map.json'
        subprocess.run(['python3', str(pathlib.Path(__file__).with_name('discover-raw.py')),
                        str(rom_path), str(ram_path), str(ports_path), str(raw)],
                       check=True, stdout=subprocess.DEVNULL)
        seed = json.loads(raw.read_text())
    snapshot_maps = [a['id'] for a in seed['assets'] if a['kind'] == 'tilemap']
    observations = []
    if flow:
        for row in csv.DictReader(flow.open(), delimiter='\t'):
            pc = int(row['pc'], 16)
            fn = owner(pc, routines)
            routine = f'{fn[0]:05x}' if fn else f'pc:{pc:05x}'
            source = int(row['rom_start'], 16)
            n = int(row['rom_end'], 16) - source + 1
            dest = int(row['ram_start'], 16)
            observations.append({'source': source, 'routine': routine,
                                 'destination': dest if dest < 65536 else None,
                                 'length': n if n >= 32 else None,
                                 'references': references(rom, source, fn)})
    # Add references for raw snapshot assets only when the cited reader has a
    # matching source range. Observed source ranges may be split at 256 bytes.
    for asset in seed['assets']:
        offset = int(asset['offset'], 16)
        for o in observations:
            if o['source'] <= offset <= o['source'] + (o['length'] or 0):
                fn = owner(int(o['routine'], 16), routines) if not o['routine'].startswith('pc:') else None
                asset['references'] = references(rom, offset, fn)
                asset.setdefault('evidence', {})['loader'] = o['routine']
                break
    seed['assets'].extend(font_runs(rom, ram, ports_path.read_bytes(), observations))
    texts = []
    seen = set()
    # Read-backed ASCII candidates only; no arbitrary Shift-JIS ROM scan.
    # They remain low confidence and are not called dialogue or script engines.
    ascii_map = {'schema': 'swankit-charmap/1', 'encoding': 'table', 'entries': [
        {'bytes': f'{b:02x}', 'text': chr(b)} for b in range(32, 127)],
        'controls': [{'code': 0, 'ends': True}]}
    for o in observations:
        source = o['source']
        span = rom[source: min(source + (o['length'] or 1), len(rom))]
        for match in re.finditer(rb'[A-Za-z][\x20-\x7e]{5,80}\x00', span):
            offset = source + match.start()
            if offset in seen or len(set(match[0][:-1])) < 5:
                continue
            seen.add(offset)
            texts.append({'offset': offset, 'length': len(match[0]),
                          'charmap': ascii_map, 'routine': o['routine'],
                          'references': []})
    family = None
    if profile:
        plugin = json.loads(profile.read_text())
        if plugin.get('schema') != 'swankit-discovery-profile/1':
            raise ValueError('unsupported family profile schema')
        if plugin.get('rom_sha256') != hashlib.sha256(rom).hexdigest():
            raise ValueError('profile ROM identity mismatch')
        family = plugin.get('family')
        seed['assets'].extend(plugin.get('assets', []))
        seed.setdefault('charmaps', {}).update(plugin.get('charmaps', {}))
        seed.setdefault('palettes', {}).update(plugin.get('palettes', {}))
        seed.setdefault('screens', []).extend(plugin.get('screens', []))
        observations.extend(plugin.get('observations', []))
        for observation in observations:
            policy = plugin.get('codec_policies', {}).get(observation['routine'])
            if policy is not None:
                observation['prehistory'] = policy
        texts.extend(plugin.get('texts', []))
        texts.extend(table_texts(rom, plugin))
    evidence = {'schema': 'swankit-discovery-evidence/1', 'seed': seed,
                'ram': list(ram), 'ports': list(ports_path.read_bytes()),
                'observations': observations[:32768], 'texts': texts[:32768],
                'family': family, 'snapshot_maps': snapshot_maps}
    evidence_path = out.with_suffix('.evidence.json')
    evidence_path.write_text(json.dumps(evidence) + '\n')
    subprocess.run([str(binary), 'assets', 'discover', str(rom_path), str(evidence_path),
                    '-o', str(out), '--report', str(out.with_suffix('.report.json')),
                    '--force', '--json'], check=True)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('rom', type=pathlib.Path)
    p.add_argument('--ram', type=pathlib.Path, required=True)
    p.add_argument('--ports', type=pathlib.Path, required=True)
    p.add_argument('--flow', type=pathlib.Path)
    p.add_argument('--decomp', type=pathlib.Path)
    p.add_argument('--profile', type=pathlib.Path)
    p.add_argument('--binary', type=pathlib.Path, required=True)
    p.add_argument('-o', '--output', type=pathlib.Path, required=True)
    a = p.parse_args()
    prepare(a.rom, a.ram, a.ports, a.flow, a.decomp, a.profile, a.output, a.binary)


if __name__ == '__main__':
    main()
