"""CPU identity and real LLVM ELF links for both supported native targets."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import native_target


class TargetIdentity(unittest.TestCase):
    def test_output_cannot_be_reused_for_another_cpu(self):
        with tempfile.TemporaryDirectory() as temporary:
            sdk = Path(temporary)
            arm = native_target.resolve('aarch64-unknown-mochios')
            native_target.initialize(sdk, arm)
            self.assertEqual(native_target.from_sdk(sdk), arm)
            with self.assertRaisesRegex(ValueError, 'another target'):
                native_target.initialize(sdk, native_target.resolve('x86_64-unknown-mochios'))
            (sdk / 'sdk.json').write_text(json.dumps({'target': 'x86_64-unknown-mochios'}))
            with self.assertRaisesRegex(ValueError, 'disagree'):
                native_target.from_sdk(sdk)

    def test_missing_or_unknown_cpu_never_defaults_to_host(self):
        with tempfile.TemporaryDirectory() as temporary:
            sdk = Path(temporary)
            with self.assertRaises(ValueError):
                native_target.from_sdk(sdk)
            with self.assertRaises(ValueError):
                native_target.resolve('aarch64-unknown-linux-gnu')

    @unittest.skipUnless(os.environ.get('MOCHIOS_TEST_SDK'), 'requires packaged LLVM tools')
    def test_real_compiler_and_lld_preserve_dso_kind_and_native_tls(self):
        base = Path(os.environ['MOCHIOS_TEST_SDK']).resolve()
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            source = directory / 'probe.c'
            source.write_text('__thread int value=3; int data=7; int *pointer=&data;\n'
                'int probe(void) { return value + *pointer; }\n')
            for native in native_target.TARGETS.values():
                sdk = directory / native.arch
                for name in ['bin', 'lib', 'libexec', 'share']:
                    (sdk / name).mkdir(parents=True)
                (sdk / 'host').symlink_to(base / 'host', target_is_directory=True)
                (sdk / 'sysroot').symlink_to(base / 'sysroot', target_is_directory=True)
                native_target.initialize(sdk, native)
                for source_name, destination in [('driver.py', 'bin/mochios-cc'),
                        ('link-adapter.py', 'libexec/gcc'), ('native_target.py', 'libexec/native_target.py'),
                        ('shared-unwind.ld', 'lib/shared-unwind.ld')]:
                    shutil.copy2(ROOT / source_name, sdk / destination)
                (sdk / 'bin/mochios-cc').chmod(0o755)
                (sdk / 'libexec/gcc').chmod(0o755)
                # Link-contract fixture: these are actual declarations from
                # the installed runtime, not claims of an ARM runtime build.
                shutil.copy2(base / 'share/runtime-symbols.json', sdk / 'share/runtime-symbols.json')
                env = dict(os.environ, LD_LIBRARY_PATH=str(sdk / 'host/lib'))
                subprocess.run([sdk / 'bin/mochios-cc', '-c', ROOT / 'shared-crt.c',
                    '-o', sdk / 'lib/shared-crt.o'], env=env, check=True, capture_output=True)
                # No compiler helper is needed by this source; an empty archive
                # exercises the real driver without mixing x86 runtime objects.
                subprocess.run([sdk / 'host/bin/ar', 'rcsD', sdk / 'lib/libbuiltins.a'], env=env, check=True)
                library = sdk / 'probe.so'
                subprocess.run([sdk / 'bin/mochios-cc', '-shared', '-O2', source,
                    '-Wl,-soname,probe.so', '-o', library], env=env, check=True, capture_output=True)
                native.check_elf(library)
                with library.open('rb') as stream:
                    header = stream.read(64)
                self.assertEqual(int.from_bytes(header[16:18], 'little'), 3, 'shared output must be ET_DYN')
                other = next(value for value in native_target.TARGETS.values() if value != native)
                with self.assertRaises(ValueError):
                    other.check_elf(library)
                relocations = subprocess.check_output([sdk / 'host/bin/readelf', '-r', library], env=env, text=True)
                self.assertIn('R_AARCH64_TLS_TPREL64' if native.arch == 'aarch64' else 'R_X86_64_TPOFF64', relocations)
