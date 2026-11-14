#!/usr/bin/env python3
"""Install verified SDK artifacts into a content-addressed local build context."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys

root=Path(os.environ['MDP_CONTEXT']).resolve()
sys.path.insert(0,str(root/'tools/mdp/src'))
from mdp.cli import assemble,atomic_json,digest,encoded,verify

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('kind',choices=['native','shared'])
args=parser.parse_args()
receipts=json.loads((root/'.mdp/builds.json').read_text())
names=['runtime','fonts'] if args.kind=='native' else ['runtime','fonts','viewkit','devkit-host','shared-sdk','system-components']
artifacts=[{'path':receipts[name]['artifact'],'sha256':receipts[name]['sha256'],'package':receipts[name]['package']} for name in names]
identity=hashlib.sha256(encoded(artifacts)).hexdigest()
configuration={'format':1,'target':'x86_64-mochios','abi':'mochios-1','artifacts':artifacts}
directory=root/'.mdp'/('sdk-'+args.kind)
directory.mkdir(parents=True,exist_ok=True)
mirrors=[]
for artifact in artifacts:
    source=root/artifact['path'];target=directory/(artifact['package']['name']+'.tar.mdp')
    if not target.is_file() or digest(target)!=artifact['sha256']:
        target.unlink(missing_ok=True)
        try:os.link(source,target)
        except OSError:__import__('shutil').copyfile(source,target)
    mirrors.append(dict(artifact,path=target.name))
configuration['artifacts']=mirrors
lock=directory/'assembly.lock';atomic_json(lock,configuration)
output=root/'out/sdk/contexts'/identity
areas={'sdk','system'} if args.kind=='native' else {'sdk-shared','system','host-tools'}
if not output.exists():assemble(lock,output,areas)
# Verify cached installed files against the actual archives before declaring them SDK inputs.
for artifact in artifacts:
    manifest=verify(root/artifact['path'])
    for file in manifest['files']:
        if file['area'] not in areas:continue
        path=output/'tree'/file['area']/file['destination']
        if file.get('type')=='symlink':
            if not path.is_symlink() or os.readlink(path)!=file['target']:raise ValueError('SDK alias changed')
        elif path.is_symlink() or not path.is_file() or digest(path)!=file['sha256']:
            raise ValueError('Installed SDK context changed; remove its named context directory and rebuild it')
        if not path.is_symlink():path.chmod(0o555 if path.stat().st_mode&0o111 else 0o444)
if args.kind=='native':
    values={'RUST_SDK':str(output/'tree/sdk/native'),'FONT_ASSETS':str(output/'tree/system/libraries/fonts')}
else:
    prefix=output/'link-prefix/usr/lib';prefix.mkdir(parents=True,exist_ok=True)
    for file in (output/'tree/system/libraries').rglob('*'):
        if file.is_file() and '.so' in file.name:
            target=prefix/file.name
            if not target.exists():target.symlink_to(file.resolve())
    for file in list(prefix.glob('*.so.*')):
        short=prefix/(file.name.split('.so.',1)[0]+'.so')
        if not short.exists():short.symlink_to(file.name)
    values={'SHARED_SDK':str(output/'tree/sdk-shared/sdk'),'WEBKIT_PREFIX':str(output/'link-prefix'),'MSIGN':str(output/'tree/host-tools/msign'),'MPACK':str(output/'tree/host-tools/mpack')}
atomic_json(root/('out/mdp/'+args.kind+'-bindings.json'),values)
