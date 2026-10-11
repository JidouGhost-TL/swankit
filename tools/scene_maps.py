"""Aggregate verified scene maps without counting overlapping ROM bytes twice."""
def number(value):
    return int(value,0) if isinstance(value,str) else value


def aggregate(scenes):
    if not scenes:
        raise ValueError('no scene maps')
    identity=scenes[0]['map']['rom'];size=number(identity['size'])
    ranges=[];texts=set();refs=set();visible=explained=0
    for scene in scenes:
        current=scene['map']['rom']
        if current['sha256']!=identity['sha256'] or number(current['size'])!=size:
            raise ValueError('scene ROM identities differ')
        visible+=scene['report']['visible_tile_bytes'];explained+=scene['report']['explained_tile_bytes']
        for asset in scene['map']['assets']:
            start=number(asset['offset']);length=number(asset.get('length',0))
            if start<0 or length<0 or start+length>size:
                raise ValueError('scene asset outside ROM')
            if length:ranges.append((start,start+length))
            if asset['kind']=='text':texts.add((start,length))
            for ref in asset.get('references',[]):refs.add((number(ref['at']),ref.get('width',2)))
    mapped=0;end=0
    for lo,hi in sorted(ranges):
        mapped+=max(0,hi-max(lo,end));end=max(end,hi)
    return {'scenes':len(scenes),'rom_bytes':size,'mapped_bytes':mapped,'rom_pct':100*mapped/size,
            'text_entries':len(texts),'references':len(refs),'scene_visible_tile_bytes':visible,
            'scene_explained_tile_bytes':explained,'scene_vram_pct':100*explained/visible if visible else None}
