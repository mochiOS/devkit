"""Native build inputs must match the declared SDK before compilation."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('native_build', Path(__file__).resolve().parents[1] / 'build-development-tools.py')
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)


class NativeSDKInputs(unittest.TestCase):
    def fixture(self, root):
        for name in ('actual', 'expected'):
            for folder in ('lib', 'sysroot/include'):
                (root / name / folder).mkdir(parents=True, exist_ok=True)
            (root / name / 'lib/runtime.a').write_bytes(b'runtime')
            (root / name / 'sysroot/include/header.h').write_bytes(b'header')

    def test_identical_inputs_are_accepted_but_changed_runtime_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root)
            build.validate_sdk_inputs(root / 'actual', root / 'expected')
            (root / 'actual/lib/runtime.a').write_bytes(b'different runtime')
            with self.assertRaisesRegex(ValueError, 'differ from the locked SDK'):
                build.validate_sdk_inputs(root / 'actual', root / 'expected')

    def test_extra_or_missing_headers_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root)
            (root / 'actual/sysroot/include/extra.h').write_bytes(b'undeclared')
            with self.assertRaises(ValueError):
                build.validate_sdk_inputs(root / 'actual', root / 'expected')
            (root / 'actual/sysroot/include/extra.h').unlink()
            (root / 'actual/sysroot/include/header.h').unlink()
            with self.assertRaises(ValueError):
                build.validate_sdk_inputs(root / 'actual', root / 'expected')


if __name__ == '__main__':
    unittest.main()
