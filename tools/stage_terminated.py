"""Freeze source-bound terminated codec pairs from independent full outputs."""
import fcntl
import hashlib
import json
from pathlib import Path
import struct


def stage_terminated(manifest_path, output):
    output = output.resolve()
    repo = Path(__file__).resolve().parents[1]
    if output == repo or repo in output.parents:
        raise ValueError('staging must remain external')
    manifest_bytes = manifest_path.read_bytes()
    manifest = json.loads(manifest_bytes)
    codec = manifest['codec']
    if codec not in ('lz-u8-zero', 'lz-word16-msb'):
        raise ValueError('unknown terminated codec')
    namespace = 'ws-compress-' + codec.removeprefix('lz-')
    records = manifest['samples']
    if not records or len(records) > 4096:
        raise ValueError('bounded nonempty samples required')
    prepared = []
    for record in records:
        rom = Path(record['rom']['path']).read_bytes()
        raw = Path(record['raw']['path']).read_bytes()
        if (hashlib.sha256(rom).hexdigest() != record['rom']['sha256']
                or hashlib.sha256(raw).hexdigest() != record['raw']['sha256']):
            raise ValueError('independent source/output digest changed')
        offset, length = record['offset'], record['length']
        if (type(offset) is not int or type(length) is not int or offset < 0
                or not 0 < length <= 1048576 or offset + length > len(rom)
                or len(raw) > 65536):
            raise ValueError('invalid original source/output extent')
        stored = rom[offset:offset + length]
        if hashlib.sha256(stored).hexdigest() != record['stored_sha256']:
            raise ValueError('stored source digest changed')
        label, provenance = codec.encode(), record['provenance'].encode()
        pair = (b'SCEPAIR1' + struct.pack('<IIQQ', len(label), len(provenance), len(raw), len(stored))
                + label + provenance + raw + stored)
        prepared.append((pair, {'args': {'codec': codec}, 'expect': {
            'codec': codec, 'raw_sha256': record['raw']['sha256'],
            'stored_sha256': record['stored_sha256'], 'consumed': length, 'round_trip': True}}))
    output.mkdir(parents=True, exist_ok=True)
    with (output / '.staging.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        destination = output / namespace
        if destination.exists():
            raise ValueError('existing frozen selection cannot be overwritten')
        destination.mkdir()
        samples = {}
        for i, (pair, sample) in enumerate(prepared):
            name = f'sample-{i:06d}.pair'
            (destination / name).write_bytes(pair)
            samples[name] = sample
        (destination / 'spec.json').write_text(json.dumps({
            'format': namespace, 'samples': samples,
            'excluded': {'enrollment.json': 'independent source provenance'}}, indent=2) + '\n')
        (destination / 'enrollment.json').write_text(json.dumps({
            'manifest_sha256': hashlib.sha256(manifest_bytes).hexdigest(),
            'records': records}, indent=2) + '\n')
    return len(prepared)
