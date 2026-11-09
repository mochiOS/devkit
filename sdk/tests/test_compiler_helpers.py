"""Real target links for compiler-generated calls; no target execution on Linux."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(os.environ.get('MOCHIOS_TEST_SDK'), 'set MOCHIOS_TEST_SDK to a built SDK')
class CompilerHelperLinks(unittest.TestCase):
    def test_private_integer_helpers_and_unknown_import_rejection(self):
        base = Path(os.environ['MOCHIOS_TEST_SDK']).resolve()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sdk = root / 'sdk'
            (sdk / 'bin').mkdir(parents=True)
            for path in base.iterdir():
                if path.name != 'bin':
                    (sdk / path.name).symlink_to(path, target_is_directory=path.is_dir())
            compiler = sdk / 'bin/mochios-c++'
            shutil.copyfile(ROOT / 'driver.py', compiler)
            compiler.chmod(0o755)
            library = root / 'helpers.so'
            subprocess.run([str(compiler), '-std=c++23', '-O2', '-shared', str(ROOT / 'probe/shared.cpp'),
                            '-Wl,-soname,helpers.so', '-o', str(library)], check=True,
                           capture_output=True, text=True)
            nm = str(sdk / 'host/bin/nm')
            dynamic = subprocess.check_output([nm, '-D', str(library)], text=True)
            complete = subprocess.check_output([nm, '--defined-only', str(library)], text=True)
            for name in ['__divti3', '__modti3', '__udivti3', '__umodti3']:
                self.assertNotIn(name, dynamic)
                self.assertIn(' t ' + name, complete)
            for name in ['memcpy', 'pthread_create', '__cxa_throw']:
                self.assertIn(' U ' + name, dynamic)
                self.assertNotIn(' T ' + name, dynamic)
            unknown = root / 'unknown.c'
            unknown.write_text('extern int unavailable_sdk_function(void); int probe(void) { return unavailable_sdk_function(); }\n')
            result = subprocess.run([str(compiler), '-x', 'c', '-shared', str(unknown),
                                     '-o', str(root / 'unknown.so')], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('Unresolved non-runtime imports: unavailable_sdk_function', result.stderr)
            self.assertFalse((root / 'unknown.so').exists())
