#!/usr/bin/env python3
"""Export local SDK bindings for independently owned mochiOS producer pipelines."""
import json
import os
from pathlib import Path
import re
import shlex
import tomllib

root = Path(os.environ['MDP_CONTEXT']).resolve()
kit = Path(__file__).resolve().parent
(root / "out/mdp/tmp").mkdir(parents=True, exist_ok=True, mode=0o700)
values = {'ROOT': str(root), 'OUT': str(root / 'out'), 'MMAKE_OUT': str(root / 'out/mmake'),
          'SCRIPTS': str(root / 'scripts'), 'HOST_TOOLS_DIR': str(root / 'out/mmake/host-tools')}
for line in (root / '.config').read_text().splitlines():
    if line.strip() and not line.startswith('#') and '=' in line:
        key, value = line.split('=', 1)
        values[key] = shlex.split(value)[0]
for line in (kit / 'toolchain.mk').read_text().splitlines():
    match = re.match(r'([A-Z_][A-Z0-9_]*)\s*(:=|\+=)\s*(.*)', line)
    if match:
        key, op, value = match.groups()
        values[key] = (values.get(key, '') + ' ' if op == '+=' else '') + value
values['SIGNING_CARGO_PATCHES'] = '--config ' + shlex.quote('patch.\"https://github.com/mochiOS/syscalls\".mochios-certificate.path=' + json.dumps(str(root / 'user/crates/certificate')))
values.update(MSIGN='$(HOST_TOOLS_DIR)/release/msign', MPACK='$(HOST_TOOLS_DIR)/release/mpack')
# The producer kit owns compiler definitions, not the product's root Makefile.
output = Path(os.environ['MDP_VARIABLES'])
output.parent.mkdir(parents=True, exist_ok=True)
text = json.dumps(values, indent=2) + '\n'
if not output.exists() or output.read_text() != text:
    output.write_text(text)
