"""Synthetic text framing checks with independent ROM/IPS and refusal oracles."""
import datetime
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


def sha(data):
    return hashlib.sha256(data).hexdigest()


def check(name, passed):
    checks.append({'name': name, 'pass': bool(passed)})
    assert passed, name


def run(root, name, args, success=True, diagnostic=None):
    argv = [cli, *map(str, args)]
    started = datetime.datetime.now(datetime.timezone.utc).isoformat()
    p = subprocess.run(argv, capture_output=True)
    (root / (name + '.stdout')).write_bytes(p.stdout)
    (root / (name + '.stderr')).write_bytes(p.stderr)
    commands.append({'name': name, 'argv': argv, 'exit': p.returncode,
                     'started': started,
                     'ended': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                     'stdout_sha256': sha(p.stdout), 'stderr_sha256': sha(p.stderr)})
    check(name + ' exit', (p.returncode == 0) == success)
    if diagnostic:
        check(name + ' diagnostic', diagnostic.encode() in p.stderr)
    return p


def fixture(root, content=b'A\r', size=65536):
    f = root / 'inputs'
    f.mkdir()
    rom = bytearray((i * 7 + 11) & 255 for i in range(size))
    rom[256:272] = content + b'\0' + bytes(15 - len(content))
    rom[-2:] = (sum(rom[:-2]) & 65535).to_bytes(2, 'little')
    (f / 'source.ws').write_bytes(rom)
    charmap = {'schema': 'swankit-charmap/1', 'entries': [
        {'bytes': '41', 'text': 'A'}, {'bytes': '42', 'text': 'B'},
        {'bytes': '0d', 'text': '\r'}, {'bytes': '0a', 'text': '\n'}],
        'controls': [{'code': 0, 'ends': True}]}
    (f / 'letters.json').write_text(json.dumps(charmap))
    m = {'schema': 'swankit-asset-map/1', 'rom': {'sha256': sha(rom), 'size': len(rom)},
         'charmaps': {'letters': {'file': 'letters.json'}},
         'assets': [{'id': 'message', 'kind': 'text', 'offset': 256, 'length': 16,
                     'relations': {'charmap': 'letters'}, 'format': {'pad': 0}}]}
    (f / 'map.json').write_text(json.dumps(m))
    return f, rom, m


def paths(root):
    return [root / 'edited.ws', root / 'edited.ips', root / 'edited.json']


def replacement(f, dest, edit, flags=()):
    return ['assets', 'replace', f / 'source.ws', f / 'map.json',
            '--id', 'message', '--from', edit, '-o', dest[0],
            '--ips', dest[1], '--map-out', dest[2], *flags]


def apply_ips(original, patch):
    check('IPS header', patch[:5] == b'PATCH')
    data = bytearray(original)
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
    check('IPS exact end', at + 3 == len(patch))
    return data


def oracle(root, f, original, dest, content):
    expected = bytearray(original)
    expected[256:272] = content + b'\0' + bytes(15 - len(content))
    expected[-2:] = (sum(expected[:-2]) & 65535).to_bytes(2, 'little')
    check('complete ROM including padding/checksum', dest[0].read_bytes() == expected)
    check('independent IPS application', apply_ips(original, dest[1].read_bytes()) == expected)
    m = json.loads(dest[2].read_text())
    check('returned identity', m['rom']['sha256'] == sha(expected))
    run(root, 'verify-result', ['assets', 'verify', dest[0], dest[2]])
    run(root, 'extract-result', ['assets', 'extract', dest[0], dest[2], root / 'returned'])
    check('exact re-export', (root / 'returned/message.txt').read_bytes() == content + b'\n')
    check('original unchanged', (f / 'source.ws').read_bytes() == original)


def roundtrip(root):
    for name, content in [('cr', b'A\r'), ('lf', b'A\n'), ('crlf', b'A\r\n'),
                          ('empty', b''), ('normal', b'AB'),
                          ('embedded', b'A\rB\nA\r\nB'), ('repeated-cr', b'A\r\r'),
                          ('cr-forced', b'A\r')]:
        r = root / name
        r.mkdir()
        f, original, _ = fixture(r, content)
        run(r, 'extract', ['assets', 'extract', f / 'source.ws', f / 'map.json', r / 'export'])
        edit = r / 'export/message.txt'
        check(name + ' exported framing', edit.read_bytes() == content + b'\n')
        dest = paths(r)
        flags = ['--text-framing', 'lf']
        if name == 'cr-forced':
            for p in dest:
                p.write_bytes(b'SENTINEL')
            flags += ['--force']
        run(r, 'replace', replacement(f, dest, edit, flags))
        oracle(r, f, original, dest, content)
        check(name + ' full-byte roundtrip', dest[0].read_bytes() == original)
    for name, text, expected in [('absent-framing-lf', b'A\r', b'A\r'),
                                 ('empty-file', b'', b''), ('no-final-newline', b'AB', b'AB'),
                                 ('single-framing-lf', b'\n', b''),
                                 ('crlf-is-content-cr-plus-framing-lf', b'A\r\n', b'A\r')]:
        r = root / name
        r.mkdir()
        f, original, _ = fixture(r)
        edit = f / 'edit.txt'
        edit.write_bytes(text)
        dest = paths(r)
        run(r, 'replace', replacement(f, dest, edit, ['--text-framing', 'lf']))
        oracle(r, f, original, dest, expected)
    r = root / 'binary-escape'
    r.mkdir()
    f, original, _ = fixture(r)
    edit = f / 'decoded.bin'
    edit.write_bytes(original[256:272])
    dest = paths(r)
    run(r, 'replace', replacement(f, dest, edit))
    oracle(r, f, original, dest, b'A\r')
    check('binary preserves full ROM', dest[0].read_bytes() == original)


def legacy(root):
    # Expected values are explicit controls for the historical import contract.
    for name, text, expected in [('cr-no-lf', b'A\r', b'A'),
                                 ('crlf', b'A\r\n', b'A'), ('lf', b'A\n', b'A'),
                                 ('normal', b'AB', b'AB'), ('empty', b'', b''),
                                 ('content-lf', b'A\n\n', b'A\n'),
                                 ('content-crlf', b'A\r\n\n', b'A\r\n'),
                                 ('only-one-cr', b'A\r\r\n', b'A\r')]:
        r = root / name
        r.mkdir()
        f, original, _ = fixture(r)
        edit = f / 'edit.txt'
        edit.write_bytes(text)
        dest = paths(r)
        run(r, 'replace', replacement(f, dest, edit))
        oracle(r, f, original, dest, expected)


def refusals(root):
    for forced in [False, True]:
        for problem in ['invalid-value', 'missing-value', 'duplicate-option', 'binary', 'png',
                        'uppercase-extension', 'wrong-kind', 'verify', 'extract',
                        'utf8', 'unmapped-text', 'stale-identity', 'too-small', 'capacity',
                        'line-limit', 'input-alias']:
            if forced and problem == 'line-limit':
                continue  # force explicitly permits text layout warnings
            r = root / (problem + ('-force' if forced else '-fresh'))
            r.mkdir()
            f, original, m = fixture(r, size=65535 if problem == 'too-small' else 65536)
            edit = f / ('edit.' + {'binary': 'bin', 'png': 'png',
                                 'uppercase-extension': 'TXT'}.get(problem, 'txt'))
            edit.write_bytes({'utf8': b'\xff\n', 'unmapped-text': b'Z\n',
                              'capacity': b'A' * 32 + b'\n',
                              'line-limit': b'A\nB\n'}.get(problem, b'B\n'))
            if problem == 'wrong-kind':
                m['assets'][0]['kind'] = 'other'
            elif problem == 'stale-identity':
                m['rom']['sha256'] = '0' * 64
            elif problem == 'line-limit':
                m['assets'][0]['format']['line_width'] = 4
                m['assets'][0]['relations']['font'] = 'font'
                m['assets'].append({'id': 'font', 'kind': 'font', 'offset': 512,
                                    'length': 8, 'format': {'bpp': 1, 'count': 1}})
            (f / 'map.json').write_text(json.dumps(m))
            before = {p: p.read_bytes() for p in f.iterdir()}
            dest = paths(r)
            if forced:
                for p in dest:
                    p.write_bytes(b'SENTINEL:' + p.name.encode())
            flags = ['--text-framing', 'lf', *(['--force'] if forced else [])]
            diagnostic = 'this input type cannot replace' if problem == 'wrong-kind' else None
            if problem == 'line-limit':
                diagnostic = 'pixels wide'
            if problem == 'invalid-value':
                flags[1] = 'crlf'
                diagnostic = '--text-framing must be lf'
            elif problem == 'duplicate-option':
                flags += ['--text-framing', 'lf']
                diagnostic = 'duplicate option'
            elif problem == 'missing-value':
                flags = [*(['--force'] if forced else []), '--text-framing']
                diagnostic = 'missing value'
            elif problem in ['binary', 'png', 'uppercase-extension']:
                diagnostic = 'requires a .txt replacement file'
            args = replacement(f, dest, edit, flags)
            if problem == 'verify':
                args = ['assets', 'verify', f / 'source.ws', f / 'map.json', *flags]
                diagnostic = 'unknown option'
            elif problem == 'extract':
                args = ['assets', 'extract', f / 'source.ws', f / 'map.json', r / 'export', *flags]
                diagnostic = 'unknown option'
            elif problem == 'input-alias':
                dest[0] = f / 'source.ws'
                args = replacement(f, dest, edit, flags)
            sentinel = {p: p.read_bytes() if p.exists() else None for p in dest}
            children = sorted(p.name for p in r.iterdir())
            run(r, 'refused', args, False, diagnostic)
            for p, data in sentinel.items():
                check(problem + ' preserved destination ' + p.name,
                      p.read_bytes() == data if data is not None else not p.exists())
            check(problem + ' no staging files',
                  sorted(p.name for p in r.iterdir()) ==
                  sorted(children + ['refused.stdout', 'refused.stderr']))
            for p, data in before.items():
                check(problem + ' input unchanged ' + p.name, p.read_bytes() == data)


def main(root):
    root.mkdir(parents=True, exist_ok=True)
    try:
        {'roundtrip': roundtrip, 'legacy': legacy, 'refusals': refusals}[case](root)
    finally:
        (root / 'COMMANDS.json').write_text(json.dumps(commands, indent=2))
        (root / 'CHECKS.json').write_text(json.dumps(checks, indent=2))
        (root / 'BINARY.sha256').write_text(sha(Path(cli).read_bytes()) + '\n')
    print(f'{case}: {len(checks)} checks; {len(commands)} CLI calls')


if os.environ.get('SWANKIT_TEXT_EVIDENCE'):
    main(Path(os.environ['SWANKIT_TEXT_EVIDENCE']) / case)
else:
    with tempfile.TemporaryDirectory(prefix='asset-text-') as temporary:
        main(Path(temporary))
