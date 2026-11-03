"""Exercise real Cargo static/shared releases and fail-closed path auditing in disposable workspaces."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('release_paths', SCRIPTS / 'release_paths.py')
paths = importlib.util.module_from_spec(spec)
spec.loader.exec_module(paths)


class AuditTests(unittest.TestCase):
    def test_binary_chunk_boundary_and_exact_directory_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); stage = root/'stage'; stage.mkdir()
            policy = root/'policy.json'
            source = '/private-development-location/example'
            policy.write_text(json.dumps({'mappings':[[source,'/src/fixture']]}))
            for data in [source.encode()+b'\x00', b'x'*(1024*1024-8)+source.encode()+b'/file.rs\x00']:
                with self.subTest(size=len(data)):
                    (stage/'library.a').write_bytes(data)
                    with self.assertRaisesRegex(ValueError,'Build-environment paths remain'):
                        paths.check(stage,policy)
            (stage/'library.a').write_bytes(b'/src/fixture/file.rs\x00/rustc/fixed-source/library/core\x00')
            paths.check(stage,policy)


@unittest.skipUnless(all(shutil.which(tool) for tool in ['cargo','rustc','perl','tar','zstd']), 'release tools required')
class ReleaseTests(unittest.TestCase):
    def fixture(self, temporary, leak=False):
        root=Path(temporary)/'Developer Workspaces/devkit'
        app=root/'crates/appcore'; (app/'src').mkdir(parents=True)
        (app/'scripts').mkdir(); (app/'include').mkdir(); (app/'lib/src').mkdir(parents=True)
        dependency=Path(temporary)/'private dependency'; (dependency/'src').mkdir(parents=True)
        (dependency/'Cargo.toml').write_text('[package]\nname="release-fixture-dependency"\nversion="0.1.0"\nedition="2021"\n')
        (dependency/'src/lib.rs').write_text('include!(concat!(env!("CARGO_MANIFEST_DIR"),"/absolute.rs"));\n')
        (dependency/'absolute.rs').write_text('pub fn source_path() -> &\'static str { file!() }\n')
        (root/'Cargo.toml').write_text('[workspace]\nmembers=["crates/appcore"]\nresolver="2"\n')
        dependency_path=os.path.relpath(dependency,app)
        (app/'Cargo.toml').write_text('[package]\nname="mochios-appcore"\nversion="0.1.0"\nedition="2021"\n[lib]\nname="appcore"\ncrate-type=["staticlib","cdylib"]\n[dependencies]\nrelease-fixture-dependency={path='+json.dumps(dependency_path)+'}\n')
        (app/'Kome.toml').write_text('[package]\nname="appcore"\nversion="0.1.0"\n')
        (root/'LICENSE').write_text('release fixture only\n');(app/'README.md').write_text('release fixture only\n')
        for name in ['mochios.h','mochios_abi.h']:(app/'include'/name).write_text('/* release fixture */\n')
        (app/'lib/src/lib.kome').write_text('// release fixture\n')
        for name in ['release.pl','release_paths.py']:shutil.copy2(SCRIPTS/name,app/'scripts'/name)
        (app/'absolute.rs').write_text('''
#[cfg(not(fixture_config_kept))] compile_error!("Cargo config flags were discarded");
#[no_mangle] pub extern "C" fn fixture_dependency_path() -> *const u8 {
    release_fixture_dependency::source_path().as_ptr()
}
#[no_mangle] pub extern "C" fn fixture_source_path() -> *const u8 { file!().as_ptr() }
'''+ ('#[no_mangle] pub extern "C" fn leaked_path() -> *const u8 { env!("CARGO_MANIFEST_DIR").as_ptr() }\n' if leak else ''))
        (app/'src/lib.rs').write_text('include!(concat!(env!("CARGO_MANIFEST_DIR"),"/absolute.rs"));\n')
        (root/'.cargo').mkdir()
        (root/'.cargo/config.toml').write_text('[build]\nrustflags=["--cfg","fixture_config_kept","-C","debuginfo=2"]\n')
        subprocess.run(['cargo','generate-lockfile','--offline'],cwd=root,check=True,capture_output=True)
        return root, app

    def test_real_release_remaps_root_and_dependency_and_preserves_cargo_config(self):
        with tempfile.TemporaryDirectory() as tmp:
            root,app=self.fixture(tmp)
            inner=root/'existing compiler wrapper.py'
            log=root/'wrapper-invocations.txt'
            inner.write_text('#!/usr/bin/env python3\nimport os,sys\nfrom pathlib import Path\nwith Path(os.environ["FIXTURE_WRAPPER_LOG"]).open("a") as stream:stream.write(" \".join(sys.argv[1:])+"\\n")\nos.execvpe(sys.argv[1],sys.argv[1:],os.environ)\n')
            inner.chmod(0o755)
            env=dict(os.environ,RUSTC_WRAPPER=str(inner),FIXTURE_WRAPPER_LOG=str(log))
            env.pop('RUSTFLAGS',None);env.pop('CARGO_ENCODED_RUSTFLAGS',None)
            result=subprocess.run(['perl',str(app/'scripts/release.pl'),'--sdk-version','fixture.1','--arch','x86_64'],cwd=root,env=env,capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stdout+'\n'+result.stderr)
            self.assertIn('release_fixture_dependency',log.read_text())
            self.assertNotIn('failed to connect to jobserver',result.stderr)
            archive=root/'target/release/x86_64-appcore-fixture.1.tar.zst'
            self.assertTrue(archive.is_file())
            proc=subprocess.Popen(['zstd','-d','-c',str(archive)],stdout=subprocess.PIPE)
            libraries=[]
            with tarfile.open(fileobj=proc.stdout,mode='r|') as tar:
                for member in tar:
                    if member.isfile():
                        data=tar.extractfile(member).read()
                        self.assertNotIn(os.fsencode(str(Path(tmp).resolve())),data,member.name)
                        if member.name in ['libappcore.a','libappcore.so']:
                            libraries.append(member.name)
                            self.assertIn(b'/src/crates/mochios-appcore-0.1.0/absolute.rs',data)
                            self.assertIn(b'/src/crates/release-fixture-dependency-0.1.0/absolute.rs',data)
            proc.stdout.close();self.assertEqual(proc.wait(),0)
            self.assertEqual(len(libraries),2)

    def test_unremappable_embedded_path_aborts_before_archive_and_checksum(self):
        with tempfile.TemporaryDirectory() as tmp:
            root,app=self.fixture(tmp,leak=True)
            result=subprocess.run(['perl',str(app/'scripts/release.pl'),'--sdk-version','fixture.1','--arch','x86_64'],cwd=root,capture_output=True,text=True)
            self.assertNotEqual(result.returncode,0)
            self.assertIn('Build-environment paths remain',result.stderr)
            self.assertFalse((root/'target/release/x86_64-appcore-fixture.1.tar.zst').exists())
            self.assertFalse((root/'target/release/SHA256SUMS').exists())


if __name__=='__main__':unittest.main()
