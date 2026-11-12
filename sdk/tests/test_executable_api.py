"""New headers must not claim executable authority on an older SDK runtime."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
import build
spec=importlib.util.spec_from_file_location('sdk_executable_verifier',ROOT/'verify.py')
verify=importlib.util.module_from_spec(spec);spec.loader.exec_module(verify)
class ExecutableAPI(unittest.TestCase):
    def test_old_sdk_does_not_claim_new_api(self):
        with tempfile.TemporaryDirectory() as td:
            sdk=Path(td);roots=set()
            self.assertFalse(build.require_executable_api(sdk,set(),roots))
            self.assertFalse(verify.verify_executable_api(sdk,set(),set()))
            with self.assertRaises(ValueError):verify.verify_executable_api(sdk,set(),verify.EXECUTABLE_API)
    def test_new_header_requires_all_real_exports_and_both_abi_links(self):
        with tempfile.TemporaryDirectory() as td:
            sdk=Path(td);p=sdk/'sysroot/include/mochi/executable.h';p.parent.mkdir(parents=True);p.touch()
            for missing in verify.EXECUTABLE_API:
                with self.subTest(missing=missing),self.assertRaises(ValueError):build.require_executable_api(sdk,verify.EXECUTABLE_API-{missing},set())
                with self.subTest(missing=missing),self.assertRaises(ValueError):verify.verify_executable_api(sdk,{'sdk_executable_probe','libsdk_executable_probe_shared.so'},verify.EXECUTABLE_API-{missing})
            with self.assertRaises(ValueError):verify.verify_executable_api(sdk,{'sdk_executable_probe'},verify.EXECUTABLE_API)
            roots=set();self.assertTrue(build.require_executable_api(sdk,verify.EXECUTABLE_API,roots));self.assertEqual(roots,verify.EXECUTABLE_API)
            self.assertTrue(verify.verify_executable_api(sdk,{'sdk_executable_probe','libsdk_executable_probe_shared.so'},roots))
