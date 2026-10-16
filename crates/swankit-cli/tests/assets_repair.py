"""Independent synthetic asset CLI checks; optional retained evidence directory."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

cli, case = sys.argv[1:3]
commands = []
checks = []


def digest(data):
    return hashlib.sha256(data).hexdigest()


def check(name, passed):
    checks.append({'name': name, 'pass': bool(passed)})
    assert passed, name


def run(root, name, args, success):
    args = [str(a) for a in args]
    p = subprocess.run([cli, *args], capture_output=True)
    (root / (name + '.stdout')).write_bytes(p.stdout)
    (root / (name + '.stderr')).write_bytes(p.stderr)
    commands.append({'name': name, 'argv': [cli, *args], 'exit': p.returncode,
                     'stdout_sha256': digest(p.stdout), 'stderr_sha256': digest(p.stderr)})
    check(name + ' exit', (p.returncode == 0) == success)
    return p


def mapping():
    return {'schema': 'swankit-charmap/1', 'entries': [
        {'bytes': '01', 'text': 'A'}, {'bytes': '02', 'text': 'B'},
        {'bytes': '03', 'text': 'C', 'aliases': ['c']}],
        'controls': [{'code': 0, 'ends': True}]}


def fixture(root):
    f = root / 'inputs'
    f.mkdir()
    rom = bytearray((i * 13 + 7) & 255 for i in range(65536))
    rom[48:56] = bytes([1, 2, 3, 0, 0, 0, 0, 0])
    rom[64:68] = bytes([1, 2, 3, 0])
    rom[80:86] = bytes([1, 0, 2, 0, 3, 0])
    rom[-2:] = (sum(rom[:-2]) & 65535).to_bytes(2, 'little')
    (f / 'source.ws').write_bytes(rom)
    (f / 'alphabet.json').write_text(json.dumps(mapping()))
    (f / 'edit.txt').write_text('CBA\n')
    (f / 'original.txt').write_text('ABC\n')
    m = {'schema': 'swankit-asset-map/1',
         'rom': {'sha256': digest(rom), 'size': len(rom)},
         'charmaps': {'alphabet': {'file': 'alphabet.json'}},
         'assets': [{'id': 'message', 'kind': 'text', 'offset': 48, 'length': 4,
                     'reserved_capacity': 8, 'relations': {'charmap': 'alphabet'},
                     'format': {'pad': 0}}]}
    (f / 'map.json').write_text(json.dumps(m))
    return f, rom, m


def replace(root, name, f, paths, success, force=False, source=None, mapfile=None, edit=None):
    return run(root, name, ['assets', 'replace', source or f / 'source.ws', mapfile or f / 'map.json',
                           '--id', 'message', '--from', edit or f / 'edit.txt', '-o', paths[0],
                           '--ips', paths[1], '--map-out', paths[2], *(['--force'] if force else [])], success)


def new_paths(root, name):
    out = root / name
    out.mkdir()
    return [out / 'edited.ws', out / 'edited.ips', out / 'edited.json']


def apply_ips(source, patch):
    check('IPS header', patch[:5] == b'PATCH')
    data = bytearray(source)
    at = 5
    while patch[at:at + 3] != b'EOF':
        offset = int.from_bytes(patch[at:at + 3], 'big')
        length = int.from_bytes(patch[at + 3:at + 5], 'big')
        at += 5
        if length:
            data[offset:offset + length] = patch[at:at + length]
            at += length
        else:
            count = int.from_bytes(patch[at:at + 2], 'big')
            data[offset:offset + count] = patch[at + 2:at + 3] * count
            at += 3
    check('IPS exact termination', at + 3 == len(patch))
    return data


def oracle(root, name, paths, original, expected_payload):
    expected = bytearray(original)
    expected[48:56] = expected_payload + bytes(8 - len(expected_payload))
    expected[-2:] = (sum(expected[:-2]) & 65535).to_bytes(2, 'little')
    actual = paths[0].read_bytes()
    check(name + ' full ROM and checksum', actual == expected)
    check(name + ' independent IPS application', apply_ips(original, paths[1].read_bytes()) == expected)
    emitted = json.loads(paths[2].read_text())
    check(name + ' exact hash binding', emitted['rom']['sha256'] == digest(actual))
    run(root, name + '-verify', ['assets', 'verify', paths[0], paths[2]], True)


def exercise(root):
    f, rom, m = fixture(root)
    originals = {p: p.read_bytes() for p in f.iterdir()}
    if case == 'destinations':
        for force in [False, True]:
            for conflict in ['rom', 'ips', 'map', 'missing-ips', 'missing-map', 'directory-ips', 'directory-map', 'fifo-map']:
                name = str(force) + '-' + conflict
                paths = new_paths(root, name)
                if force:
                    for p in paths:
                        p.write_bytes(b'SENTINEL')
                index = 0 if conflict == 'rom' else 1 if 'ips' in conflict else 2
                if conflict.startswith('missing'):
                    paths[index] = paths[index].parent / 'absent' / 'sidecar'
                elif conflict.startswith(('directory', 'fifo')):
                    if paths[index].exists():
                        paths[index].unlink()
                    if conflict.startswith('directory'):
                        paths[index].mkdir()
                    else:
                        os.mkfifo(paths[index])
                elif force:
                    continue  # all existing regular files are a permitted forced write
                else:
                    paths[index].write_bytes(b'SENTINEL')
                before = {p: p.read_bytes() if p.is_file() else None for p in paths}
                p = replace(root, name, f, paths, False, force)
                for path, contents in before.items():
                    check(name + ' preserved ' + path.name,
                          path.read_bytes() == contents if contents is not None else not path.is_file())
                if conflict.startswith('missing'):
                    check(name + ' no force hint', b'--force' not in p.stderr)
        for force in [False, True]:
            name = 'success-' + str(force)
            paths = new_paths(root, name)
            if force:
                for p in paths:
                    p.write_bytes(b'SENTINEL')
            replace(root, name, f, paths, True, force)
            oracle(root, name, paths, rom, bytes([3, 2, 1, 0]))
    elif case == 'aliases':
        for kind in ['same', 'parent', 'hardlink', 'symlink', 'dangling', 'rom', 'map', 'edit', 'charmap', 'input-hardlink']:
            paths = new_paths(root, kind)
            for p in paths:
                p.write_bytes(b'SENTINEL')
            if kind == 'same':
                paths[2] = paths[0]
            elif kind == 'parent':
                alias = root / 'parent-alias'
                alias.symlink_to(paths[0].parent, target_is_directory=True)
                paths[2] = alias / paths[0].name
            elif kind == 'hardlink':
                paths[2].unlink()
                os.link(paths[0], paths[2])
            elif kind in ['symlink', 'dangling']:
                paths[2].unlink()
                paths[2].symlink_to(paths[0] if kind == 'symlink' else paths[2].parent / 'absent')
            elif kind == 'input-hardlink':
                paths[2].unlink()
                os.link(f / 'source.ws', paths[2])
            else:
                paths[2] = f / {'rom':'source.ws', 'map':'map.json', 'edit':'edit.txt', 'charmap':'alphabet.json'}[kind]
            before = {p: p.read_bytes() for p in paths if p.is_file()}
            replace(root, kind, f, paths, False, True)
            for p, contents in before.items():
                check(kind + ' sentinel or input preserved', p.read_bytes() == contents)
    elif case == 'charmaps':
        for kind in ['json', 'definition', 'tsv', 'multiple']:
            variant = json.loads(json.dumps(m))
            if kind in ['definition', 'multiple']:
                (f / 'text-def.json').write_text(json.dumps({'schema':'swankit-text-def/1', 'id':'synthetic', 'map':mapping()}))
                variant['charmaps']['alphabet'] = {'file':'text-def.json'}
            if kind in ['tsv', 'multiple']:
                (f / 'word.tsv').write_text('bytes\ttext\n0100\t"A"\n0200\t"B"\n0300\t"C"\n')
                if kind == 'tsv':
                    variant['assets'][0].update({'offset':80, 'length':6})
                    variant['charmaps']['alphabet'] = {'file':'word.tsv', 'unit':'word-le'}
                else:
                    variant['charmaps'].update({'json':{'file':'alphabet.json'}, 'tsv':{'file':'word.tsv', 'unit':'word-le'}})
                    variant['assets'].extend([
                        {'id':'other','kind':'text','offset':64,'length':4,'relations':{'charmap':'json'}},
                        {'id':'word','kind':'text','offset':80,'length':6,'relations':{'charmap':'tsv'}}])
            mapfile = f / (kind + '.json')
            mapfile.write_text(json.dumps(variant))
            paths = new_paths(root, kind)
            replace(root, kind, f, paths, True, mapfile=mapfile)
            result = json.loads(paths[2].read_text())
            check(kind + ' all dependencies inline', all('file' not in v and 'inline' in v for v in result['charmaps'].values()))
            run(root, kind + '-verify', ['assets','verify',paths[0],paths[2]], True)
            extracted = root / (kind + '-extract')
            run(root, kind + '-extract', ['assets','extract',paths[0],paths[2],extracted], True)
            check(kind + ' exact decoded text', (extracted / 'message.txt').read_text().rstrip('\n') == 'CBA')
            if kind == 'multiple':
                check('multiple untouched second map', (extracted / 'other.txt').read_text().rstrip('\n') == 'ABC')
                check('multiple word TSV unit', (extracted / 'word.txt').read_text().rstrip('\n') == 'ABC')
            undo = new_paths(root, kind + '-undo')
            replace(root, kind + '-undo', f, undo, True, source=paths[0], mapfile=paths[2], edit=f/'original.txt')
            if kind != 'tsv':
                check(kind + ' undo full original ROM', undo[0].read_bytes() == rom)
            redo = new_paths(root, kind + '-redo')
            replace(root, kind + '-redo', f, redo, True, source=undo[0], mapfile=undo[2])
            check(kind + ' redo exact prior output', redo[0].read_bytes() == paths[0].read_bytes())
    elif case == 'safety':
        cases = {'capacity': lambda v: None,
                 'identity': lambda v: v['rom'].update({'sha256':'0'*64}),
                 'overlap': lambda v: v['assets'].append({'id':'owned','kind':'other','offset':54,'length':2}),
                 'free-overlap': lambda v: v.update({'free_space':[{'offset':54,'length':4}]}),
                 'negative': lambda v: v['assets'][0].update({'offset':-1}),
                 'negative-string': lambda v: v['assets'][0].update({'offset':'-1'}),
                 'overflow': lambda v: v['assets'][0].update({'offset':'18446744073709551616'}),
                 'nonnumeric': lambda v: v['assets'][0].update({'offset':'nowhere'}),
                 'traversal': lambda v: v['charmaps']['alphabet'].update({'file':'../alphabet.json'})}
        for name, change in cases.items():
            variant = json.loads(json.dumps(m))
            change(variant)
            mapfile = f / (name + '.json')
            mapfile.write_text(json.dumps(variant))
            edit = None
            if name == 'capacity':
                edit = f / 'large.bin'
                edit.write_bytes(bytes([1]) * 9)
            for force in [False, True]:
                label = name + '-' + str(force)
                paths = new_paths(root, label)
                if force:
                    for p in paths:
                        p.write_bytes(b'SENTINEL')
                p = replace(root, label, f, paths, False, force, mapfile=mapfile, edit=edit)
                for path in paths:
                    check(label + ' no write', path.read_bytes() == b'SENTINEL' if force else not path.exists())
                if name == 'capacity':
                    check(label + ' actual capacity', b'needs 9 stored bytes but has 8' in p.stderr)
                if name in ['negative','negative-string','overflow','nonnumeric']:
                    check(label + ' unsigned guidance', b'unsigned' in p.stderr and b'Raw' not in p.stderr)
        # Verification checks a declared fill even when no replacement would
        # allocate that free range. In-place edits need not consume free space.
        dirty = json.loads(json.dumps(m))
        dirty['free_space'] = [{'offset':512,'length':16,'fill':255}]
        (f / 'dirty-free.json').write_text(json.dumps(dirty))
        p = run(root, 'dirty-free-verify', ['assets','verify',f/'source.ws',f/'dirty-free.json'], False)
        check('declared fill remains verified', b'declared fill' in p.stdout)
    elif case == 'extract':
        for force in [False, True]:
            name = 'extract-' + str(force)
            out = root / name
            out.mkdir()
            if force:
                (out / 'message.txt').write_bytes(b'SENTINEL')
                (out / 'index.json').mkdir()
            else:
                (out / 'index.json').write_bytes(b'SENTINEL')
            run(root, name, ['assets','extract',f/'source.ws',f/'map.json',out,*(['--force'] if force else [])], False)
            check(name + ' no partial extracted file', (out/'message.txt').read_bytes() == b'SENTINEL' if force else not (out/'message.txt').exists())
    else:
        raise ValueError(case)
    for p, original in originals.items():
        check('original input preserved ' + p.name, p.read_bytes() == original)
    check('all staging cleaned', not list(root.rglob('.swankit-assets-*')))


def main(root):
    try:
        exercise(root)
    finally:
        (root / 'COMMANDS.json').write_text(json.dumps(commands, indent=2))
        (root / 'CHECKS.json').write_text(json.dumps(checks, indent=2))
    print(case, len(commands), 'CLI invocations;', len(checks), 'checks passed')


if len(sys.argv) == 4:
    destination = Path(sys.argv[3])
    destination.mkdir(parents=True, exist_ok=False)
    main(destination)
else:
    with tempfile.TemporaryDirectory(prefix='asset-repair-') as directory:
        main(Path(directory))
