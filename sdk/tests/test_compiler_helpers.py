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

    def test_rust_hidden_math_does_not_hide_public_newlib_exports(self):
        import hashlib
        import importlib.util
        base = Path(os.environ['MOCHIOS_TEST_SDK']).resolve()
        spec = importlib.util.spec_from_file_location('math_driver', ROOT / 'driver.py')
        driver = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(driver)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'intrinsics.c'
            source.write_text('__attribute__((weak,visibility("hidden"))) double ceil(double x) { return x; }\n')
            obj = root / 'intrinsics.o'
            subprocess.run([str(base / 'bin/mochios-cc'), '-fno-builtin', '-c', str(source), '-o', str(obj)], check=True, capture_output=True)
            archive = root / 'libcompiler_builtins-ab12.rlib'
            subprocess.run([str(base / 'host/bin/ar'), 'rcs', str(archive), str(obj)], check=True)
            digest = hashlib.sha256(archive.read_bytes()).digest()
            copies = root / 'copies'
            copies.mkdir()
            args = driver.isolate_rust_math(base, [str(archive), str(obj)], copies, os.environ.copy())
            self.assertEqual(args[1], str(obj))
            self.assertEqual(hashlib.sha256(archive.read_bytes()).digest(), digest)
            symbols = subprocess.check_output([str(base / 'host/bin/nm'), args[0]], text=True)
            self.assertIn('__mochios_builtin_ceil', symbols)
            main = root / 'main.c'
            main.write_text('int main(void) { return 0; }\n')
            executable = root / 'consumer'
            subprocess.run([str(base / 'bin/mochios-cc'), str(main), *args[:1], '-o', str(executable)], check=True, capture_output=True)
            dynamic = subprocess.check_output([str(base / 'host/bin/nm'), '-D', '--defined-only', str(executable)], text=True)
            self.assertIn(' T ceil\n', dynamic)
            self.assertNotIn('__mochios_builtin_ceil', dynamic)
