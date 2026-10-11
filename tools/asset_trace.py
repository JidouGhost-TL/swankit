"""Normalize instruction-propagated loader witnesses, without temporal fallback.

The extension collector is tools/ghidra/WSAssetTrace.java. Unsupported or
truncated captures remain evidence, but cannot establish a loader witness.
Reference origins are candidates until stored-field arithmetic is reviewed.
"""
import hashlib
import json

IMMEDIATE = 1 << 40


def observations(capture):
    result = []
    seen = set()
    capture_hash = hashlib.sha256(capture).hexdigest()
    for line in capture.decode('utf-8').splitlines():
        row = json.loads(line)
        if row.get('kind') != 'instruction-flow' or not row.get('complete_return'):
            continue
        output = bytes.fromhex(row['output_hex'])
        size, dest, source = row['length'], row['destination'], row['source']
        if size != len(output) or not 8 <= size <= 32768:
            raise ValueError('loader output extent')
        if not 0 <= dest <= 65536-size or not 0 <= source < row['source_end']:
            raise ValueError('loader address bounds')
        if not 0 < row['explained_bytes'] <= size or row['causal_source_bytes'] <= 0:
            continue
        key = source, dest, size, output
        if key in seen:
            continue
        seen.add(key)
        registers = row.get('entry_registers', {})
        di = registers.get('DI')
        codec_dest = None
        if isinstance(di, int) and 0 <= di <= 65535:
            if any(isinstance(registers.get(seg), int) and ((registers[seg]*16+di) & 0xfffff) == row.get('cpu_destination', dest)
                   for seg in ['DS', 'ES']):
                codec_dest = di
        result.append({'source': source, 'routine': f"instruction-flow entry={row['entry']:05x} call={row['id']} capture={capture_hash}",
                       'destination': dest, 'length': size if row['source_end']-source == size else None, 'decoded_length': size,
                       'output': list(output), 'codec_destination': codec_dest, 'prehistory': None, 'references': []})
    return result


def reference_candidates(capture):
    """Return distinct pointer-origin byte locations, with their origin kind.

    These are deliberately not asset references and do not authorize relocation.
    Register address provenance can include banks, table indices and displacements.
    """
    found = set()
    for line in capture.decode('utf-8').splitlines():
        row = json.loads(line)
        if row.get('kind') != 'instruction-flow' or not row.get('complete_return'):
            continue
        for origin in row.get('pointer_origins', []):
            if origin < 0:
                raise ValueError('negative origin')
            found.add((origin % IMMEDIATE, 'code-immediate' if origin >= IMMEDIATE else 'rom-read'))
    return [{'at': at, 'proof_kind': kind} for at, kind in sorted(found)]


def reference_fields(capture, rom):
    """Candidate fields whose origins and encoded address agree with a source.

    A two-byte offset has an implied bank, so this does not establish pointer
    closure or authorize relocation. Far fields must also reproduce the source
    through an actually recorded window setting. This is address evidence;
    independent reader/profile review still establishes ownership and bounds.
    """
    found=set()
    for line in capture.decode('utf-8').splitlines():
        row=json.loads(line)
        if row.get('kind')!='instruction-flow' or not row.get('complete_return'):
            continue
        kinds={}
        for origin in row.get('pointer_origins',[]):
            at=origin%IMMEDIATE
            kinds.setdefault(at,set()).add('code-immediate' if origin>=IMMEDIATE else 'rom-read')
        source=row['source']
        for at in kinds:
            if at<0 or at+2>len(rom) or at+1 not in kinds:continue
            offset=int.from_bytes(rom[at:at+2],'little')
            kind='code-immediate' if all(kinds[at+i]=={'code-immediate'} for i in range(2)) else 'rom-read'
            if offset==source&65535:found.add((at,2,'linear',source,kind))
            if at+4>len(rom) or any(at+i not in kinds for i in range(4)):continue
            segment=int.from_bytes(rom[at+2:at+4],'little');linear=((segment<<4)+offset)&0xfffff
            for banks in row.get('banks',[]):
                c0,c2,c3=map(int,banks.split(':'))
                mapped=(c2*65536+(linear&65535)) if 0x20000<=linear<0x30000 else ((c3*65536+(linear&65535)) if 0x30000<=linear<0x40000 else (((c0&15)*0x100000+linear) if linear>=0x40000 else -1))
                if mapped>=0 and mapped%len(rom)==source:
                    kind='code-immediate' if all(kinds[at+i]=={'code-immediate'} for i in range(4)) else 'rom-read'
                    found.add((at,4,'far',source,kind))
    return [dict(at=at,width=width,form=form,source=source,proof_kind=kind)
            for at,width,form,source,kind in sorted(found)]
