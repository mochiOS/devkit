"""An advertised resource API must have its real process exports and DSO links."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('sdk_bundle_verifier',ROOT/'verify.py')
verify=importlib.util.module_from_spec(spec);spec.loader.exec_module(verify)

class BundleAPITests(unittest.TestCase):
    def test_old_sdk_does_not_claim_bundle_support(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertFalse(verify.verify_bundle_api(Path(tmp),set(),set()))
    def test_header_cannot_mask_missing_exports_or_link_probes(self):
        with tempfile.TemporaryDirectory() as tmp:
            sdk=Path(tmp);header=sdk/'sysroot/include/mochi/bundle.h'
            header.parent.mkdir(parents=True);header.touch()
            with self.assertRaisesRegex(ValueError,'runtime exports'):
                verify.verify_bundle_api(sdk,set(),set())
            for missing in verify.BUNDLE_API:
                with self.assertRaisesRegex(ValueError,'runtime exports'):
                    verify.verify_bundle_api(sdk,{'sdk_bundle_probe','libsdk_bundle_probe_shared.so'},verify.BUNDLE_API-{missing})
            with self.assertRaisesRegex(ValueError,'ABI probes'):
                verify.verify_bundle_api(sdk,{'sdk_bundle_probe'},verify.BUNDLE_API)
            self.assertTrue(verify.verify_bundle_api(sdk,{'sdk_bundle_probe','libsdk_bundle_probe_shared.so'},verify.BUNDLE_API))
    def test_exports_cannot_be_distributed_without_the_header(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(ValueError,'public header'):
                verify.verify_bundle_api(Path(tmp),set(),verify.BUNDLE_API)

if __name__=='__main__': unittest.main()
