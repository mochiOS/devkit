#!/usr/bin/env python3
"""Publish a verified SDK without overwriting existing assets or promoting AppCore's latest release."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile

ROOT=Path(__file__).resolve().parent


def sha(path):
    """Calculate an archive digest without loading the SDK into memory."""
    with path.open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()


def main():
    """Revalidate build metadata and release bytes before publishing immutable assets."""
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--repository',required=True);p.add_argument('--commit',required=True)
    p.add_argument('--directory',type=Path,default=ROOT.parent/'out/shared-sdk/releases');args=p.parse_args()
    lock=json.loads((ROOT/'sources.lock.json').read_text());version=lock['version'];tag='sdk-'+version
    name='x86_64-mochios-sdk-'+version+'.tar.gz';archive=args.directory/name
    manifest=json.loads((args.directory/(name+'.json')).read_text())
    if manifest['sources']!=lock or manifest['archive']!=name or manifest['sha256']!=sha(archive):raise ValueError('SDK source/archive identity mismatch')
    if (args.directory/'SHA256SUMS').read_text()!=sha(archive)+'  '+name+'\n':raise ValueError('Checksum marker mismatch')
    with tarfile.open(archive) as tar:
        metadata=json.load(tar.extractfile('sdk/sdk.json'))
        if metadata!=manifest['sdk'] or metadata['version']!=version or metadata['runtime_tested'] is not False:raise ValueError('Unexpected SDK metadata')
        for member in tar:
            path=Path(member.name)
            if not path.parts or path.parts[0]!='sdk' or path.is_absolute() or '..' in path.parts:raise ValueError('Unsafe archive path')
            if member.issym() and (Path(member.linkname).is_absolute() or '..' in Path(member.linkname).parts):raise ValueError('Non-relocatable SDK symlink')
            if member.isfile():
                expected=manifest['files'][str(path.relative_to('sdk'))]
                if hashlib.file_digest(tar.extractfile(member),'sha256').hexdigest()!=expected['sha256']:raise ValueError('SDK inventory mismatch')
    existing=subprocess.run(['gh','release','view',tag,'--repo',args.repository,'--json','isDraft,assets'],capture_output=True,text=True)
    assets=[archive,args.directory/(name+'.json'),args.directory/'SHA256SUMS']
    if existing.returncode==0:
        info=json.loads(existing.stdout)
        # Verify an already-published release byte-for-byte; never clobber assets.
        with tempfile.TemporaryDirectory() as tmp:
            subprocess.run(['gh','release','download',tag,'--repo',args.repository,'--dir',tmp],check=True)
            for path in assets:
                old=Path(tmp)/path.name
                if old.exists() and sha(old)!=sha(path):raise ValueError('Existing SDK asset differs; increment SDK version')
                if not old.exists():
                    if not info['isDraft']:raise ValueError('Published SDK release is incomplete; use a new version')
                    subprocess.run(['gh','release','upload',tag,str(path),'--repo',args.repository],check=True)
    else:
        notes='Shared C/C++ cross SDK for mochiOS x86-64. C++23, thread, exception, shared/executable-link and ELF structure checks passed. Target binaries have not been executed on mochiOS. Startup-linked shared objects with initial-exec TLS; see sdk/USAGE.md for limits. Includes source pins, original license notices, patched newlib source, manifest and SHA-256 checksums.'
        with tempfile.TemporaryDirectory() as tmp:
            body=Path(tmp)/'notes.txt';body.write_text(notes+'\n')
            subprocess.run(['gh','release','create',tag,'--repo',args.repository,'--target',args.commit,'--draft','--prerelease','--latest=false','--title','mochiOS shared C/C++ SDK '+version,'--notes-file',str(body)],check=True)
        for path in assets:subprocess.run(['gh','release','upload',tag,str(path),'--repo',args.repository],check=True)
    subprocess.run(['gh','release','edit',tag,'--repo',args.repository,'--draft=false','--prerelease','--latest=false'],check=True)


if __name__=='__main__':main()
