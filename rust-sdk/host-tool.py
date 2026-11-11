#!/usr/bin/env python3
"""Build a trusted SDK host tool without compiling or executing application code."""
import argparse
import json
import os
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sdk', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--tool', choices=['msign'], required=True)
    parser.add_argument('--test', action='store_true')
    args = parser.parse_args()
    sdk = json.loads(args.sdk.read_text())
    devkit = Path(sdk['appcore']).parents[1]
    env = os.environ.copy()
    for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_TARGET'):
        env.pop(key, None)
    env.update(RUSTC=sdk['rustc'], RUSTDOC=sdk['rustdoc'])
    command = [sdk['cargo'], 'build', '--release', '--manifest-path', str(devkit / 'Cargo.toml'),
               '--package', args.tool, '--no-default-features', '--target-dir', str(args.output.resolve())]
    for config in sdk['cargo_config']:
        command += ['--config', config]
    if args.test:
        test = command.copy()
        test[1] = 'test'
        test.remove('--release')
        env['CARGO_PROFILE_TEST_DEBUG'] = '0'
        subprocess.run(test, env=env, cwd=devkit, check=True)
    subprocess.run(command, env=env, cwd=devkit, check=True)


if __name__ == '__main__':
    main()
