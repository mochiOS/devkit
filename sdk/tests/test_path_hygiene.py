"""Regressions for C/C++/Rust source mapping and fail-closed SDK archive production."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
import path_hygiene as hygiene
spec=importlib.util.spec_from_file_location('sdk_build',ROOT/'build.py')
builder=importlib.util.module_from_spec(spec);spec.loader.exec_module(builder)


class PathTests(unittest.TestCase):
    def policy(self,root):
        project=root/'private project'
        out=root/'private output'
        return hygiene.make_policy(project,out,{},root/'rust toolchain',environment={})

    def test_known_root_at_chunk_boundary_and_partly_remapped_ci_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);policy=self.policy(root);prefix=os.fsencode(root/'private project')
            for data in [prefix+b'\x00', b'x'*(1024*1024-9)+prefix+b'/source.c\x00',
                         b'/home/runner/work/devkit/devkit/source.cpp\x00',
                         b'/build/work/devkit/devkit/out/source.rs\x00']:
                with self.subTest(size=len(data)), self.assertRaisesRegex(ValueError,'Build-environment path'):
                    hygiene.scan_stream(io.BytesIO(data),'fixture.a',policy)
            hygiene.scan_stream(io.BytesIO(b'/src/llvm/source.cpp\x00/src/user/runtime.rs\x00'), 'fixture.a',policy)

    def test_normal_source_examples_are_not_mistaken_for_the_build_machine(self):
        with tempfile.TemporaryDirectory() as tmp:
            hygiene.scan_stream(io.BytesIO(b'/home/user/example.txt\nhttps://example.invalid/source/file'),
                                'upstream-source.c',self.policy(Path(tmp)))

    def test_archive_metadata_is_deterministic_and_has_no_owner_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);source=root/'source';source.mkdir();(source/'file').write_text('fixture\n')
            outputs=[root/'one.tar.gz',root/'two.tar.gz']
            for out in outputs:hygiene.write_archive(source,out,'sdk')
            self.assertEqual(outputs[0].read_bytes(),outputs[1].read_bytes())
            header=outputs[0].read_bytes()[:10]
            self.assertEqual(header[3]&8,0);self.assertEqual(header[4:8],b'\x00'*4)
            with tarfile.open(outputs[0]) as tar:
                for member in tar:
                    self.assertEqual((member.uid,member.gid,member.uname,member.gname,member.mtime),(0,0,'','',0))

    def test_nested_corresponding_source_archive_is_also_scanned(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);policy=self.policy(root);payload=root/'payload';payload.mkdir()
            sdk=root/'sdk';sdk.mkdir()
            (payload/'source.c').write_text(str(root/'private project'/'source.c'))
            hygiene.write_archive(payload,sdk/'source.tar.gz','source')
            with self.assertRaisesRegex(ValueError,'source.tar.gz!source/source.c'):
                hygiene.audit_tree(sdk,policy)
            (payload/'source.c').write_text('/src/project/source.c')
            hygiene.write_archive(payload,sdk/'source.tar.gz','source')
            report=hygiene.audit_tree(sdk,policy)
            self.assertTrue(report['passed']);self.assertEqual(report['archives_scanned'],1)

    def test_legacy_tar_owner_metadata_and_escaping_links_are_rejected(self):
        for kind in ['owner','link']:
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);archive=root/'bad.tar'
                with tarfile.open(archive,'w') as tar:
                    member=tarfile.TarInfo('root/member')
                    if kind=='owner':member.uid=1001;member.uname='private-builder'
                    else:member.type=tarfile.SYMTYPE;member.linkname='../../outside'
                    tar.addfile(member)
                with tarfile.open(archive) as tar, self.assertRaises(ValueError):
                    hygiene.audit_tar(tar,self.policy(root),'bad.tar')

    def test_bad_payload_cannot_replace_an_archive_or_its_checksum(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);sdk=root/'sdk';sdk.mkdir();out=root/'out';release=out/'releases';release.mkdir(parents=True)
            archive=release/'x86_64-mochios-sdk-fixture.tar.gz';archive.write_bytes(b'previous archive')
            marker=release/'SHA256SUMS';marker.write_bytes(b'previous checksum')
            (sdk/'library.a').write_text(str(root/'private project'/'source.cpp'))
            with self.assertRaisesRegex(ValueError,'Build-environment path'):
                builder.package({'version':'fixture'},sdk,out,self.policy(root))
            self.assertEqual(archive.read_bytes(),b'previous archive')
            self.assertEqual(marker.read_bytes(),b'previous checksum')

    def test_new_policy_changes_the_rust_cache_namespace(self):
        with tempfile.TemporaryDirectory() as tmp:
            policy=self.policy(Path(tmp));first=hygiene.identity(policy)
            policy['mappings'].append(('/another/private/path','/src/another'))
            self.assertNotEqual(first,hygiene.identity(policy))

    @unittest.skipUnless(shutil.which('cc') and shutil.which('c++'), 'host C/C++ compilers required')
    def test_real_c_and_cxx_macros_and_debug_paths_are_remapped(self):
        with tempfile.TemporaryDirectory(prefix='SDK path test ') as tmp:
            root=Path(tmp);project=root/'private project';project.mkdir();policy=self.policy(root)
            for compiler,suffix in [('cc','c'),('c++','cpp')]:
                source=project/('source.'+suffix);source.write_text('const char *source_path = __FILE__;\n')
                output=root/(suffix+'.o')
                subprocess.run([compiler,'-g',*hygiene.compiler_flags(policy),'-c',str(source),'-o',str(output)],cwd=project,check=True)
                with output.open('rb') as stream:hygiene.scan_stream(stream,output.name,policy)
                self.assertIn(b'/src/devkit/source.',output.read_bytes())

    @unittest.skipUnless(shutil.which('rustc'), 'Rust compiler required')
    def test_real_rust_wrapper_remaps_absolute_file_macro_and_debug_data(self):
        with tempfile.TemporaryDirectory(prefix='SDK Rust path ') as tmp:
            root=Path(tmp);project=root/'private project';project.mkdir();policy=self.policy(root)
            policy_file=root/'policy.json';policy_file.write_text(json.dumps(policy))
            source=project/'source.rs';source.write_text('pub fn source_path() -> &\'static str { file!() }\n')
            output=root/'fixture.rlib'
            subprocess.run([sys.executable,str(ROOT/'path_hygiene.py'),shutil.which('rustc'),
                            '--crate-type=rlib','--crate-name=fixture','-C','debuginfo=2',str(source),'-o',str(output)],
                           cwd=project,env=dict(os.environ,MOCHIOS_SDK_PATH_POLICY=str(policy_file)),check=True)
            with output.open('rb') as stream:hygiene.scan_stream(stream,'fixture.rlib',policy)


if __name__=='__main__':unittest.main()
