#!/usr/bin/env python3
"""Stage a native application MPKG using the shared, capability-preserving SDK helper."""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tomllib
import os

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--source', type=Path, required=True)
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--mpack', type=Path, required=True)
args = parser.parse_args()
helper = Path(__file__).resolve().parents[1] / 'rust-sdk/build.py'
spec = importlib.util.spec_from_file_location('sdk_application', helper)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
output = args.source / 'out/mdp/application'
if output.exists():
    shutil.rmtree(output)
original = tomllib.loads((args.source / 'manifest.toml').read_text())
bundle_name = Path(original['binary'][0]['path']).parent.name
bindings=json.loads(Path(os.environ['MDP_VARIABLES']).read_text())
for name in json.loads(os.environ.get('MDP_VARIABLE_OVERLAYS','[]')):
    if Path(name).is_file():bindings.update(json.loads(Path(name).read_text()))
licenses=Path(bindings['FONT_ASSETS'])/'licenses'
manifest = module.stage(args.source, args.binary, output, licenses, bundle_name=bundle_name)
subprocess.run([str(args.mpack), 'create', '--manifest', str(output / 'manifest.toml'),
                '--payload', str(output / 'payload'), '--output', str(output / 'application.mpkg')], check=True)
# The MPKG is authoritative; do not retain another full copy of its ELF payload.
shutil.rmtree(output/'payload')
