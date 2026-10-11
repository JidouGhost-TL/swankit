"""Immutable enrollment of discovery drafts and emulator compression witnesses."""
import fcntl
import hashlib
import json
import struct
from pathlib import Path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def stage_discovery(manifest_path, output):
    output = output.resolve()
    repo = Path(__file__).resolve().parents[1]
    if output == repo or repo in output.parents:
        raise ValueError('staging must remain external')
    manifest = json.loads(manifest_path.read_bytes())
    prepared = []
    for r in manifest['drafts']:
        rom, amap, evidence = (Path(r[k]).resolve(strict=True) for k in ('rom', 'map', 'evidence'))
        doc = json.loads(amap.read_bytes())
        if doc['rom']['sha256'] != digest(rom):
            raise ValueError('map/ROM identity mismatch')
        independent = {}
        asset_ids = {a['id'] for a in doc['assets']}
        for asset_id, oracle in r.get('independent', {}).items():
            if asset_id not in asset_ids:
                raise ValueError('independent asset missing from map')
            p = Path(oracle['path']).resolve(strict=True)
            if digest(p) != oracle['sha256']:
                raise ValueError('independent output changed')
            independent[asset_id] = oracle['sha256']
        prepared.append((rom, amap, evidence, independent))
    pairs = []
    for r in manifest['codecs']:
        codec = r['codec']
        if codec not in ('lzss-u16-zero', 'lzss-12'):
            raise ValueError('unknown additional codec')
        raw, stored = (Path(r[k]).read_bytes() for k in ('raw', 'stored'))
        if hashlib.sha256(raw).hexdigest() != r['raw_sha256'] or hashlib.sha256(stored).hexdigest() != r['stored_sha256']:
            raise ValueError('capture digest changed')
        label, provenance = codec.encode(), r['provenance'].encode()
        pair = b'SCEPAIR1' + struct.pack('<IIQQ', len(label), len(provenance), len(raw), len(stored)) + label + provenance + raw + stored
        args = {'codec': codec}
        if codec == 'lzss-u16-zero':
            destination = r.get('destination', 0x2000)
            policy = r.get('prehistory', 'low5-prefix')
            if not isinstance(destination, int) or not 0 <= destination <= 65535:
                raise ValueError('codec destination outside RAM')
            if policy not in ('low5-prefix', 'low8-prefix', 'zero-until-start'):
                raise ValueError('unknown prehistory policy')
            args.update(destination=destination, prehistory=policy)
        pairs.append((args, pair, {'codec': codec, 'raw_sha256': r['raw_sha256'],
                                   'stored_sha256': r['stored_sha256'], 'consumed': len(stored),
                                   'round_trip': True}))
    if not prepared or not pairs:
        raise ValueError('enrollment needs drafts and independent codec witnesses')
    output.mkdir(parents=True, exist_ok=True)
    with (output / '.staging.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        for name in ('ws-discover', 'ws-compress-extra'):
            if (output / name).exists():
                raise ValueError('existing frozen namespace cannot be overwritten')
        d = output / 'ws-discover'
        d.mkdir()
        (d / 'maps').mkdir()
        (d / 'evidence').mkdir()
        samples = {}
        for i, (rom, amap, evidence, independent) in enumerate(prepared):
            stem = f'sample-{i:06d}'
            name = stem + '.rom'
            (d / name).symlink_to(rom)
            mn, en = 'maps/' + stem + '.json', 'evidence/' + stem + '.json'
            (d / mn).write_bytes(amap.read_bytes())
            (d / en).write_bytes(evidence.read_bytes())
            samples[name] = {'args': {'map': mn, 'evidence': en,
                                     'independent_ids': {k: True for k in independent}},
                             'expect': {'rom_sha256': digest(rom), 'verify_errors': 0,
                                        'independent': independent}}
        (d / 'spec.json').write_text(json.dumps({'format': 'ws-discover', 'samples': samples,
                                               'excluded': {'maps': 'frozen draft maps',
                                                            'evidence': 'normalized runtime sources and snapshots'}}, indent=2)+'\n')
        d = output / 'ws-compress-extra'
        d.mkdir()
        samples = {}
        for i, (args, pair, expected) in enumerate(pairs):
            name = f'sample-{i:06d}.pair'
            (d / name).write_bytes(pair)
            samples[name] = {'args': args, 'expect': expected}
        (d / 'spec.json').write_text(json.dumps({'format': 'ws-compress', 'samples': samples}, indent=2)+'\n')
    return len(prepared), len(pairs)
