#!/usr/bin/env python3
"""Invoke Terra's native engine build; never silently fall back to a shell-only app."""
import argparse
import json
import os
from pathlib import Path
import subprocess

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--source',type=Path,required=True)
args=parser.parse_args()
root=Path(os.environ['MDP_CONTEXT']).resolve()
bindings=json.loads(Path(os.environ['MDP_VARIABLES']).read_text())
for name in json.loads(os.environ.get('MDP_VARIABLE_OVERLAYS','[]')):
    if Path(name).is_file():bindings.update(json.loads(Path(name).read_text()))
sdk=args.source/'out/mdp/app-build.json'
sdk.parent.mkdir(parents=True,exist_ok=True)
subprocess.run(['python3',str(Path(__file__).resolve().parent/'export-app-sdk.py'),
                '--root',str(root),'--sdk',bindings['RUST_SDK'],'--output',str(sdk)],check=True)
subprocess.run(['python3',str(args.source/'tools/build.py'),'package','--sdk',str(sdk),
                '--output',str(args.source/'out/mdp/engine'),
                '--webkit-prefix',str(Path(bindings['WEBKIT_PREFIX'])),
                '--shared-sdk',str(Path(bindings['SHARED_SDK'])),
                '--font-licenses',str(Path(bindings['FONT_ASSETS'])/'licenses'),
                '--mpack',bindings['MPACK']],check=True)
output=args.source/'out/mdp/application'
output.mkdir(parents=True,exist_ok=True)
__import__('shutil').copyfile(args.source/'out/mdp/engine/Terra.mpkg',output/'application.mpkg')
