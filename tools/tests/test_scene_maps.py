import sys
from pathlib import Path
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from scene_maps import aggregate

class SceneCoverageTests(unittest.TestCase):
    def scene(self, start, length, **extra):
        return {'map':{'rom':{'sha256':'00'*32,'size':1000},'assets':[dict(id='data',kind='tiles',offset=start,length=length,**extra)]},'report':{'visible_tile_bytes':32,'explained_tile_bytes':16}}
    def test_overlapping_scene_ranges_count_once(self):
        result=aggregate([self.scene(100,100),self.scene('0x96',100)])
        self.assertEqual(result['mapped_bytes'],150)
        self.assertEqual(result['rom_pct'],15)
        self.assertEqual(result['scene_visible_tile_bytes'],64)
        self.assertEqual(result['scene_explained_tile_bytes'],32)
    def test_cross_rom_inputs_and_out_of_bounds_rejected(self):
        a=self.scene(100,100);b=self.scene(200,100);b['map']['rom']['sha256']='11'*32
        with self.assertRaises(ValueError):aggregate([a,b])
        with self.assertRaises(ValueError):aggregate([self.scene(990,20)])
    def test_text_and_reference_duplicates_across_scenes_count_once(self):
        a=self.scene(100,10,references=[{'at':20,'width':2}]);a['map']['assets'][0]['kind']='text'
        result=aggregate([a,a])
        self.assertEqual(result['text_entries'],1)
        self.assertEqual(result['references'],1)
