#!/usr/bin/env python3
"""Build a trusted SDK host tool without compiling or executing application code."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import shutil


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sdk', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--tool', choices=['msign', 'kome'], required=True)
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
    cwd = devkit
    if args.tool == 'kome':
        # Host-only CLI resolution is isolated from native application/std patches.
        project = args.output.resolve() / 'kome-source'
        project.mkdir(parents=True, exist_ok=False)
        shutil.copytree(devkit / 'crates/kome/src', project / 'src')
        (project / 'Cargo.toml').write_text((devkit / 'crates/kome/Cargo.toml').read_text() + '\n[workspace]\n')
        shutil.copyfile(Path(__file__).parent / 'kome.Cargo.lock', project / 'Cargo.lock')
        command[command.index('--manifest-path') + 1] = str(project / 'Cargo.toml')
        command += ['--locked', '--features', 'keyring/vendored']
        cwd = project
    else:
        for config in sdk['cargo_config']:
            command += ['--config', config]
    if args.test:
        test = command.copy()
        test[1] = 'test'
        test.remove('--release')
        env['CARGO_PROFILE_TEST_DEBUG'] = '0'
        subprocess.run(test, env=env, cwd=cwd, check=True)
    subprocess.run(command, env=env, cwd=cwd, check=True)


if __name__ == '__main__':
    main()
