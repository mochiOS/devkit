"""Fast tests for immutable input/driver contracts; target links run separately."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('sdk_driver',ROOT/'driver.py')
driver=importlib.util.module_from_spec(spec);spec.loader.exec_module(driver)

class RecipeTests(unittest.TestCase):
    def test_patches_are_locked(self):
        lock=json.loads((ROOT/'sources.lock.json').read_text())
        self.assertEqual(set(lock['patches']),{p.name for p in (ROOT/'patches').iterdir()})
        for name,expected in lock['patches'].items():
            self.assertEqual(hashlib.sha256((ROOT/'patches'/name).read_bytes()).hexdigest(),expected)
    def test_runtime_dependencies_are_locked(self):
        lock=json.loads((ROOT/'sources.lock.json').read_text())
        self.assertEqual(hashlib.sha256((ROOT/'runtime.Cargo.lock').read_bytes()).hexdigest(),lock['runtime_cargo_lock_sha256'])
        self.assertNotIn('source = \"git+', (ROOT/'runtime.Cargo.lock').read_text())
    def test_shared_runtime_not_embedded(self):
        with tempfile.TemporaryDirectory() as tmp:
            sdk=Path(tmp)
            inputs=['-lc','-lm','-l','pthread','-lz',str(sdk/'lib/libc++.a'),'object.o']
            self.assertEqual(driver.shared_runtime_arguments(sdk,inputs),['-lz','object.o'])
    def test_public_sources_are_exact_commits(self):
        for source in json.loads((ROOT/'sources.lock.json').read_text())['sources'].values():
            self.assertTrue(source['url'].startswith('https://github.com/'))
            self.assertRegex(source['commit'],r'^[0-9a-f]{40}$')
    def test_response_quoting_and_nested_arguments(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)/'response.rsp';p.write_text('"a path/file.cpp" -DVALUE=42')
            self.assertEqual(driver.response_arguments(['@'+str(p)]),['a path/file.cpp','-DVALUE=42'])
    def test_recursive_response_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)/'recursive.rsp';p.write_text('@'+str(p))
            with self.assertRaisesRegex(ValueError,'nesting'):driver.response_arguments(['@'+str(p)])
    def test_malformed_response_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)/'malformed.rsp';p.write_text('"unfinished')
            with self.assertRaises(ValueError):driver.response_arguments(['@'+str(p)])
if __name__=='__main__':unittest.main()
