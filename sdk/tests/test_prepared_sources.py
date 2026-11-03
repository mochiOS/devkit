"""Source/patch pin transitions must invalidate disposable SDK build inputs."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
spec=importlib.util.spec_from_file_location('sdk_source_builder',ROOT/'build.py')
builder=importlib.util.module_from_spec(spec);spec.loader.exec_module(builder)

class PreparedSourceTests(unittest.TestCase):
    def fixture(self,root):
        recipe=root/'recipe';recipe.mkdir();(recipe/'patches').mkdir()
        (recipe/'runtime.Cargo.lock').write_text('version = 4\n')
        sources={}
        for name in ['user','newlib','llvm']:
            source=root/('source-'+name);source.mkdir();(source/'value').write_text('original\n');sources[name]=source
        (sources['user']/'scripts').mkdir();(sources['user']/'scripts/build-newlib.sh').write_text('#!/bin/sh\n')
        lock={'sources':{name:{'url':'https://example.invalid/'+name,'commit':'a'*40} for name in ['user','newlib']},
              'llvm':{'url':'https://example.invalid/llvm','sha256':'b'*64},'patches':{},
              'runtime_cargo_lock_sha256':hashlib.sha256((recipe/'runtime.Cargo.lock').read_bytes()).hexdigest()}
        return recipe,sources,lock,root/'out'
    def test_source_pin_change_replaces_old_working_copy_but_same_pin_reuses_it(self):
        with tempfile.TemporaryDirectory() as tmp:
            recipe,sources,lock,out=self.fixture(Path(tmp))
            with mock.patch.object(builder,'ROOT',recipe):
                first=builder.prepared_sources(lock,sources,out)['user']/'value'
                timestamp=first.stat().st_mtime_ns
                builder.prepared_sources(lock,sources,out)
                self.assertEqual(first.stat().st_mtime_ns,timestamp)
                (sources['user']/'value').write_text('updated native API\n')
                lock['sources']['user']['commit']='c'*40
                builder.prepared_sources(lock,sources,out)
                self.assertEqual(first.read_text(),'updated native API\n')
    def test_legacy_copy_without_input_stamp_is_recreated(self):
        with tempfile.TemporaryDirectory() as tmp:
            recipe,sources,lock,out=self.fixture(Path(tmp))
            old=out/'prepared/user';old.mkdir(parents=True);(old/'value').write_text('stale\n')
            with mock.patch.object(builder,'ROOT',recipe):
                builder.prepared_sources(lock,sources,out)
            self.assertEqual((old/'value').read_text(),'original\n')
    def test_patch_update_starts_from_pristine_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            recipe,sources,lock,out=self.fixture(Path(tmp))
            patch=recipe/'patches/newlib-fixture.patch'
            with mock.patch.object(builder,'ROOT',recipe):
                for value in ['first','second']:
                    patch.write_text('--- a/value\n+++ b/value\n@@ -1 +1 @@\n-original\n+'+value+'\n')
                    lock['patches'][patch.name]=hashlib.sha256(patch.read_bytes()).hexdigest()
                    actual=builder.prepared_sources(lock,sources,out)
                    self.assertEqual((actual['newlib']/'value').read_text(),value+'\n')
                    self.assertEqual((sources['newlib']/'value').read_text(),'original\n')
    def test_prepared_symlink_cannot_redirect_builder_writes(self):
        with tempfile.TemporaryDirectory() as tmp:
            recipe,sources,lock,out=self.fixture(Path(tmp));(out/'prepared').mkdir(parents=True)
            (out/'prepared/user').symlink_to(sources['user'],target_is_directory=True)
            with mock.patch.object(builder,'ROOT',recipe),self.assertRaisesRegex(ValueError,'Invalid prepared source'):
                builder.prepared_sources(lock,sources,out)
