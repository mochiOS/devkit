"""An SDK must not advertise declarations without measured loader implementations."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
spec=importlib.util.spec_from_file_location('sdk_loader_builder',ROOT/'build.py')
builder=importlib.util.module_from_spec(spec);spec.loader.exec_module(builder)

class PrivateLoaderTests(unittest.TestCase):
    def test_legacy_sysroot_does_not_claim_private_loading(self):
        with tempfile.TemporaryDirectory() as tmp:
            symbols={'puts'}
            self.assertFalse(builder.require_private_loader_api(Path(tmp),{'puts'},symbols))
            self.assertEqual(symbols,{'puts'})
    def test_header_without_all_implementations_is_rejected_before_export(self):
        with tempfile.TemporaryDirectory() as tmp:
            sdk=Path(tmp);h=sdk/'sysroot/include/dlfcn.h';h.parent.mkdir(parents=True);h.touch()
            symbols={'puts'}
            with self.assertRaisesRegex(ValueError,'dlclose'):
                builder.require_private_loader_api(sdk,{'dlopen','dlsym','puts'},symbols)
            self.assertEqual(symbols,{'puts'})
    def test_all_four_real_implementations_become_required_process_exports(self):
        with tempfile.TemporaryDirectory() as tmp:
            sdk=Path(tmp);h=sdk/'sysroot/include/dlfcn.h';h.parent.mkdir(parents=True);h.touch()
            symbols={'puts'}
            self.assertTrue(builder.require_private_loader_api(sdk,{'puts'}|builder.PRIVATE_LOADER_C_API,symbols))
            self.assertEqual(symbols,{'puts'}|builder.PRIVATE_LOADER_C_API)

if __name__=='__main__':unittest.main()
