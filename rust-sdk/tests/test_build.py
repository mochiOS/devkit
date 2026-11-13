import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import tomllib
import unittest

HERE = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('build_app', HERE / 'build.py')
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)


class PackageStage(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.source = self.root / 'independent-app'
        self.source.mkdir()
        manifest = dict(format=1, package=dict(id='net.example.editor', name='Editor', provenance='built-in',
            version='1.0.0', revision=1, vendor='Independent developer', kind='application', architecture='x86_64', abi='mochios-1'),
            application=dict(entry='entry.elf', icon='icon.svg', resources=['icon.svg'],
                document_extensions=['txt'], document_icons=[dict(extensions=['txt'], resource='icon.svg')]),
            binary=[dict(path='/system/applications/Editor.app/entry.elf', kind='application', requires=['window.create'])])
        (self.source / 'manifest.toml').write_text(build.dumps(manifest))
        (self.source / 'resources').mkdir()
        (self.source / 'resources/icon.svg').write_text('<svg/>')
        header = bytearray(64)
        header[:6] = b'\x7fELF\x02\x01'
        struct.pack_into('<HH', header, 16, 2, 62)
        self.binary = self.root / 'program.elf'
        self.binary.write_bytes(header)

    def test_nested_metadata_and_capabilities_survive_without_builtin_trust(self):
        result = build.stage(self.source, self.binary, self.root / 'stage')
        parsed = tomllib.loads((self.root / 'stage/manifest.toml').read_text())
        self.assertEqual(parsed, result)
        self.assertNotIn('provenance', parsed['package'])
        self.assertEqual(parsed['application']['document_extensions'], ['txt'])
        self.assertEqual(parsed['application']['document_icons'][0]['resource'], 'icon.svg')
        self.assertEqual(parsed['binary'][0]['requires'], ['window.create'])
        self.assertEqual(parsed['binary'][0]['path'], '/applications/Editor.app/entry.elf')
        self.assertTrue(all(f['digest'].startswith('sha256:') for f in parsed['file']))
        self.assertEqual((self.root / 'stage/payload/bundle/entry.elf').stat().st_mode & 0o777, 0o755)

    def test_resources_cannot_escape_the_checkout(self):
        for name in ('../manifest.toml', '/etc/passwd', 'a\\b', ''):
            with self.assertRaises(ValueError):
                build.resource(self.source, name)
        (self.source / 'resources/external').symlink_to(self.binary)
        with self.assertRaises(ValueError):
            build.resource(self.source, 'external')

    def test_wrong_elf_and_existing_stages_fail(self):
        self.binary.write_text('Linux shell script')
        with self.assertRaises(ValueError):
            build.stage(self.source, self.binary, self.root / 'stage')

    def test_source_and_font_pins_are_immutable(self):
        lock = json.loads((HERE / 'sources.lock.json').read_text())
        self.assertEqual(lock['sdk']['version'], '27.0-dp.1-shared.8')
        for source in lock['sources'].values():
            self.assertRegex(source['revision'], r'^[0-9a-f]{40}$')
        for digest in lock['fonts'].values():
            self.assertRegex(digest, r'^[0-9a-f]{64}$')


if __name__ == '__main__':
    unittest.main()
