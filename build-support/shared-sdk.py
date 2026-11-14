#!/usr/bin/env python3
"""Prepare a local shared SDK from an explicitly locked, externally supplied SDK tree."""
import json
import hashlib
import os
from pathlib import Path
import subprocess

root=Path(os.environ['MDP_CONTEXT']).resolve()
base=Path(os.environ.get('MDP_SHARED_SDK_BASE',str(root/'out/sdk/shared-base'))).resolve()
if not (base/'bin/mochios-c++').is_file() or not (base/'sdk.json').is_file():
    raise SystemExit('A locked shared SDK is required at out/sdk/shared-base (or MDP_SHARED_SDK_BASE). Install the reviewed SDK with komeup and import it before mdp workspace lock.')
info=json.loads((base/'sdk.json').read_text())
if info.get('target') != 'x86_64-unknown-mochios' or info.get('abi') != 'mochios-1':
    raise SystemExit('Shared SDK target/ABI mismatch')
output=root/'out/mdp/shared-sdk'
output.mkdir(parents=True,exist_ok=True)
subprocess.run(['cp','--reflink=auto','-a',str(base)+ '/.',str(output)],check=True)
bindings=json.loads((root/'out/mdp/native-bindings.json').read_text())
runtime=Path(bindings['RUST_SDK'])/'lib/libmochi_user_newlib_runtime.a'
if not runtime.is_file():
    raise SystemExit('Build the current native runtime before preparing its shared SDK overlay')
__import__('shutil').copyfile(runtime,output/'lib/libmochi_user_newlib_runtime.a')
subprocess.run([str(output/'host/bin/objcopy'),
                '--redefine-syms='+str(output/'share/runtime-intrinsics.rename'),
                str(output/'lib/libmochi_user_newlib_runtime.a')],check=True)
with runtime.open('rb') as source:
    runtime_hash=hashlib.file_digest(source,'sha256').hexdigest()
info.update(base_release=info['version'],version=info['version']+'+workspace.'+runtime_hash[:12],
            local_runtime_overlay=True,runtime_archive_sha256=runtime_hash)
(output/'sdk.json').write_text(json.dumps(info,indent=2)+'\n')
# This local SDK is a declared build input; this command publishes no SDK release.
