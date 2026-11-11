import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ACTION = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('mpkg_action', ACTION / 'package.py')
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class Packaging(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.root = Path(self.folder.name)
        self.addCleanup(self.folder.cleanup)
        self.payload = self.root / 'payload'
        self.payload.mkdir()
        self.manifest = self.root / 'manifest.toml'
        self.manifest.write_text('format = 1\n[package]\nid = "net.example.independent"\nkind = "resource"\n')
        (self.payload / 'bundle').mkdir()
        (self.payload / 'bundle/sample').write_bytes(b'Independent publisher payload\n')
        self.output = self.root / 'dist/Example.mpkg'

    def test_no_publisher_or_package_kind_restriction(self):
        for kind in ('application', 'library', 'resource', 'command-line-tool'):
            self.manifest.write_text(f'format = 1\n[package]\nid = "net.thirdparty.example"\nkind = "{kind}"\n')
            package.validate(self.manifest, self.payload, self.output)

    def test_cannot_overwrite_inputs_or_existing_outputs(self):
        for output in (self.manifest, self.payload / 'package.mpkg'):
            with self.assertRaises(ValueError):
                package.validate(self.manifest, self.payload, output)
        self.output.parent.mkdir()
        self.output.write_text('existing')
        with self.assertRaises(ValueError):
            package.validate(self.manifest, self.payload, self.output)
        self.assertEqual(self.output.read_text(), 'existing')

    def test_cannot_overwrite_checksum_or_follow_dangling_output_link(self):
        self.output.parent.mkdir()
        self.output.symlink_to(self.root / 'not-created')
        with self.assertRaises(ValueError):
            package.validate(self.manifest, self.payload, self.output)
        self.output.unlink()
        (self.output.parent / 'SHA256SUMS').write_text('existing')
        with self.assertRaises(ValueError):
            package.validate(self.manifest, self.payload, self.output)

    def test_invalid_and_multiline_inputs(self):
        for value in ('', 'a\nb', 'a\rb', 'a\0b'):
            with self.assertRaises(ValueError):
                package.checked_path(value, 'test')
        for text in ('format = true\n[package]\n', 'format = 2\n[package]\n', 'format = 1\n'):
            self.manifest.write_text(text)
            with self.assertRaises(ValueError):
                package.validate(self.manifest, self.payload, self.output)

    def test_packager_failure_never_creates_final_outputs(self):
        with patch.object(package.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, 'mpack')):
            with self.assertRaises(subprocess.CalledProcessError):
                package.create(self.manifest, self.payload, self.output, Path('/unavailable'))
        self.assertFalse(self.output.exists())
        self.assertFalse((self.output.parent / 'SHA256SUMS').exists())

    def test_bad_container_cannot_be_reported_as_success(self):
        def fake(args, **kwargs):
            Path(args[-1]).write_bytes(b'not an mpkg')
        with patch.object(package.subprocess, 'run', side_effect=fake):
            with self.assertRaises(ValueError):
                package.create(self.manifest, self.payload, self.output, Path('/fake'))
        self.assertFalse(self.output.exists())

    def test_bundled_tool_uses_standalone_locked_sources(self):
        temporary = self.root / 'tool'
        temporary.mkdir()
        with patch.object(package.subprocess, 'run') as call:
            package.bundled_tool(ACTION, temporary)
        self.assertIn('--locked', call.call_args.args[0])
        self.assertIn('[workspace]', (temporary / 'mpack/Cargo.toml').read_text())
        self.assertEqual((temporary / 'mpack/Cargo.lock').read_bytes(), (ACTION / 'tool.Cargo.lock').read_bytes())
        self.assertTrue((temporary / 'mpack/src/main.rs').is_file())

    @unittest.skipUnless(os.environ.get('MPKG_TEST_MPACK'), 'Set MPKG_TEST_MPACK to test the actual packager')
    def test_real_mpack_and_literal_action_inputs(self):
        # Shell syntax is a literal filename, never evaluated by this Action.
        self.output = self.root / 'out; literal/Example $(touch INJECTION).mpkg'
        original = self.manifest.read_bytes()
        result = package.create(self.manifest, self.payload, self.output, Path(os.environ['MPKG_TEST_MPACK']))
        self.assertEqual(result['sha256'], package.digest(self.output))
        self.assertEqual(self.manifest.read_bytes(), original)
        self.assertEqual(self.output.stat().st_nlink, 1)
        self.assertFalse((self.root / 'INJECTION').exists())
        self.assertEqual((self.output.parent / 'SHA256SUMS').read_text(), result['sha256'] + '  ' + self.output.name + '\n')
        environment = dict(os.environ, MPKG_MANIFEST=str(self.manifest), MPKG_PAYLOAD=str(self.payload),
                           MPKG_OUTPUT=str(self.root / 'second/Example.mpkg'), MPKG_TOOL=os.environ['MPKG_TEST_MPACK'],
                           GITHUB_OUTPUT=str(self.root / 'outputs'))
        subprocess.run(['python3', str(ACTION / 'package.py')], env=environment, check=True)
        outputs = (self.root / 'outputs').read_text()
        self.assertIn('package=' + str(self.root / 'second/Example.mpkg'), outputs)
        self.assertIn('sha256=', outputs)

    @unittest.skipUnless(os.environ.get('MPKG_TEST_MPACK'), 'Set MPKG_TEST_MPACK to test the actual packager')
    def test_real_mpack_rejects_symlink_payload(self):
        (self.payload / 'bundle/external-link').symlink_to(self.manifest)
        with self.assertRaises(subprocess.CalledProcessError):
            package.create(self.manifest, self.payload, self.output, Path(os.environ['MPKG_TEST_MPACK']))
        self.assertFalse(self.output.exists())


if __name__ == '__main__':
    unittest.main()
