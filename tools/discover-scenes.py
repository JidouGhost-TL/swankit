#!/usr/bin/env python3
"""Discover separate ROM maps for a manifest of synchronized RAM/port captures.

The manifest has schema swankit-scene-captures/1 and a scenes array. Each
scene supplies id, ram and ports paths, optionally instruction_flow and profile.
Relative paths resolve beside the manifest. All inputs describe one ROM.
The resulting scene maps remain separate: runtime tile slots and palettes can
change between scenes. The summary reports the union of their ROM ranges.
"""
import argparse,importlib.util,json,pathlib,subprocess
from scene_maps import aggregate


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('rom',type=pathlib.Path);p.add_argument('manifest',type=pathlib.Path)
    p.add_argument('--binary',required=True,type=pathlib.Path);p.add_argument('-o','--output',required=True,type=pathlib.Path)
    a=p.parse_args();manifest=json.loads(a.manifest.read_text())
    if manifest.get('schema')!='swankit-scene-captures/1':raise ValueError('scene manifest schema')
    spec=importlib.util.spec_from_file_location('discovery',pathlib.Path(__file__).with_name('discover-assets.py'))
    adapter=importlib.util.module_from_spec(spec);spec.loader.exec_module(adapter)
    a.output.mkdir(parents=True,exist_ok=True);records=[];seen=set()
    def path(value):
        if value is None:return None
        value=pathlib.Path(value);return value if value.is_absolute() else a.manifest.parent/value
    for scene in manifest['scenes']:
        ident=scene['id']
        if not ident or any(c not in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-' for c in ident) or ident in seen:
            raise ValueError('unsafe or duplicate scene id')
        seen.add(ident);d=a.output/ident;d.mkdir(parents=True,exist_ok=True);out=d/'map.json'
        adapter.prepare(a.rom,path(scene['ram']),path(scene['ports']),None,None,path(scene.get('profile')),out,a.binary,path(scene.get('instruction_flow')))
        record={'id':ident,'map':json.loads(out.read_text()),'report':json.loads(out.with_suffix('.report.json').read_text())}
        records.append(record)
        print(ident,record['report']['mapped_bytes'],record['report']['explained_tile_bytes'],flush=True)
    summary=aggregate(records);summary['records']=[{'id':r['id'],'report':r['report']} for r in records]
    (a.output/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')


if __name__=='__main__':main()
