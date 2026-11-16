"""Synthetic adversarial inputs only; never runs wallpaper contents."""
import importlib.util
import hashlib
import sys
sys.dont_write_bytecode = True
from pathlib import Path
import struct
import tempfile
import unittest
spec=importlib.util.spec_from_file_location('triage', Path(__file__).with_name('scan.py'))
m=importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
class Tests(unittest.TestCase):
    def test_hash_and_hidden_payload(self):
        with tempfile.TemporaryDirectory() as t:
            p=Path(t); (p/'project.json').write_bytes(b'{"type":"video"}')
            (p/'.hidden').mkdir(); (p/'.hidden'/'x.bin').write_bytes(b'MZpayload')
            r=m.scan(p)
            expected=b'wpe-gallery-content-v1\0'
            expected+=b'D'+struct.pack('>Q',len(b'.hidden'))+b'.hidden'
            for name,data in [('.hidden/x.bin',b'MZpayload'),('project.json',b'{"type":"video"}')]:
                b=name.encode(); expected+=b'F'+struct.pack('>Q',len(b))+b+struct.pack('>Q',len(data))+data
            self.assertEqual(r['content_hash'],hashlib.sha256(expected).hexdigest())
            self.assertTrue(r['blocked']); self.assertFalse(r['auto_safe'])
    def test_package_and_embedded_script(self):
        with tempfile.TemporaryDirectory() as t:
            p=Path(t); (p/'project.json').write_text('{"type":"scene"}')
            data=b'{"origin":{"script":"fetch(1)"}}'
            def string(s): return struct.pack('<I',len(s))+s
            header=string(b'PKGV0023')+struct.pack('<I',1)+string(b'scene.json')+struct.pack('<II',0,len(data))
            (p/'scene.pkg').write_bytes(header+data)
            r=m.scan(p); self.assertFalse(r['errors']); self.assertEqual(r['scripts'][0]['signals'],['network'])
            (p/'scene.pkg').write_bytes(header+data[:-1]); self.assertTrue(m.scan(p)['errors'])
    def test_symlink(self):
        with tempfile.TemporaryDirectory() as t:
            p=Path(t); (p/'project.json').symlink_to('/etc/passwd')
            self.assertTrue(m.scan(p)['errors'])
if __name__=='__main__': unittest.main()
