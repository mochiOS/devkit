#!/usr/bin/env python3
"""Compose a Rust application SDK from source pins and a released SDK, without an OS checkout."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import tomllib

HERE = Path(__file__).resolve().parent


def run(*args, **kwargs):
    """Stream real tool diagnostics and propagate failures."""
    subprocess.run([str(a) for a in args], check=True, **kwargs)


def checkout(root, name, spec):
    """Fetch a full reviewed commit, never a mutable branch or submodule graph."""
    if not re.fullmatch(r'[0-9a-f]{40}', spec['revision']):
        raise ValueError('Expected a full source commit ID')
    target = root / name
    target.mkdir(parents=True)
    run('git', 'init', target)
    run('git', '-C', target, 'remote', 'add', 'origin', 'https://github.com/' + spec['repository'] + '.git')
    if name == 'libraries/rust':
        run('git', '-C', target, 'sparse-checkout', 'set', 'library', 'src/mochios-backtrace',
            'src/mochios-libunwind', 'vendor/rustc-literal-escaper')
    run('git', '-C', target, 'fetch', '--depth=1', '--filter=blob:none', 'origin', spec['revision'])
    run('git', '-C', target, 'checkout', '--detach', spec['revision'])
    if subprocess.check_output(['git', '-C', str(target), 'rev-parse', 'HEAD'], text=True).strip() != spec['revision']:
        raise ValueError('Source revision mismatch')


def configs(root):
    """Use a coherent source graph for standard-library, framework and protocol crates."""
    patches = {'crates-io': {'libc': root / 'libraries/libc', 'viewkit': root / 'libraries/viewkit'},
               'https://github.com/mochiOS/libc': {'libc': root / 'libraries/libc'},
               'https://github.com/mochiOS/viewkit': {'viewkit': root / 'libraries/viewkit'},
               'https://github.com/mochiOS/mnu': {'mnu-abi': root / 'core/crates/abi'},
               'https://github.com/mochiOS/mBoot': {'mboot-protocol': root / 'services/mboot-protocol'}}
    patches['https://github.com/mochiOS/syscalls'] = {
        tomllib.loads(p.read_text())['package']['name']: p.parent for p in (root / 'user/crates').glob('*/Cargo.toml')}
    for name in ('core', 'alloc', 'std'):
        patches['crates-io']['rustc-std-workspace-' + name] = root / 'rust-overlay/lib/rustlib/src/rust/library' / ('rustc-std-workspace-' + name)
    return ['patch.' + json.dumps(source) + '.' + json.dumps(name) + '.path=' + json.dumps(str(path))
            for source, crates in patches.items() for name, path in crates.items()]


def overlay(root, toolchain):
    """Pair the known Rust compiler with mochiOS std, preserving the installed rustup tree."""
    run('rustup', 'toolchain', 'install', toolchain, '--profile', 'minimal', '--component', 'rust-src')
    base = Path(subprocess.check_output(['rustc', '+' + toolchain, '--print', 'sysroot'], text=True).strip())
    output = root / 'rust-overlay'
    (output / 'bin').mkdir(parents=True)
    library = output / 'lib/rustlib/src/rust/library'
    library.mkdir(parents=True)
    for path in (base / 'bin').iterdir():
        dest = output / 'bin' / path.name
        if path.name in ('rustc', 'rustdoc'):
            dest.write_text('#!/bin/sh\nexec ' + shlex.quote(str(path)) + ' --sysroot ' + shlex.quote(str(output)) + ' "$@"\n')
            dest.chmod(0o755)
        else:
            dest.symlink_to(path)
    for path in (base / 'lib').iterdir():
        if path.name != 'rustlib':
            (output / 'lib' / path.name).symlink_to(path)
    for path in (base / 'lib/rustlib').iterdir():
        if path.name != 'src':
            (output / 'lib/rustlib' / path.name).symlink_to(path)
    source = root / 'libraries/rust'
    for path in (source / 'library').iterdir():
        if path.name not in ('backtrace', 'vendor'):
            if path.name == 'std':
                shutil.copytree(path, library / 'std', copy_function=lambda src, dst: os.symlink(src, dst))
            else:
                (library / path.name).symlink_to(path)
    (library / 'backtrace').symlink_to(source / 'src/mochios-backtrace')
    unwind = output / 'lib/rustlib/src/rust/src/llvm-project'
    unwind.mkdir(parents=True)
    (unwind / 'libunwind').symlink_to(source / 'src/mochios-libunwind')
    for vendor in (library / 'vendor', output / 'lib/rustlib/src/rust/vendor'):
        vendor.mkdir()
        (vendor / 'rustc-literal-escaper').symlink_to(source / 'vendor/rustc-literal-escaper')
    return output, base


def prepare(output):
    """Verify/install the C SDK through komeup, then prepare independent Rust metadata."""
    root = output.resolve()
    if root.exists():
        raise ValueError('Use a fresh SDK output directory')
    root.mkdir(parents=True)
    lock = json.loads((HERE / 'sources.lock.json').read_text())
    revision = os.environ.get('SDK_ACTION_REF') or subprocess.check_output(
        ['git', '-C', str(HERE.parent), 'rev-parse', 'HEAD'], text=True).strip()
    for name, spec in lock['sources'].items():
        checkout(root, name, spec)
    devkit = root / 'tools/devkit'
    devkit.parent.mkdir()
    shutil.copytree(HERE.parent, devkit, ignore=shutil.ignore_patterns('.git', 'target', 'out', '__pycache__'))
    tool = root / 'komeup-build'
    tool.mkdir()
    shutil.copyfile(HERE.parent / 'crates/komeup/Cargo.toml', tool / 'Cargo.toml')
    shutil.copyfile(HERE / 'komeup.Cargo.lock', tool / 'Cargo.lock')
    shutil.copytree(HERE.parent / 'crates/komeup/src', tool / 'src')
    with (tool / 'Cargo.toml').open('a') as stream:
        stream.write('\n[workspace]\n')
    run('cargo', 'build', '--locked', '--release', '--manifest-path', tool / 'Cargo.toml', '--target-dir', root / 'host-tools')
    komeup = root / 'host-tools/release/komeup'
    env = dict(os.environ, KOMEUP_HOME=str(root / 'manager'))
    run(komeup, 'sdk', 'install', lock['sdk']['version'], env=env)
    sdk = Path(subprocess.check_output([str(komeup), 'sdk', 'path', '--version', lock['sdk']['version']], env=env, text=True).strip())
    receipt = tomllib.loads((sdk / '.komeup-sdk.toml').read_text())
    if receipt['sha256'] != lock['sdk']['sha256']:
        raise ValueError('SDK archive differs from the reviewed SHA-256')
    compiler, base = overlay(root, lock['rust_toolchain'])
    fonts = root / 'libraries/fonts'
    run('perl', fonts / 'scripts/install-fonts.pl', '--config', fonts / 'fonts.conf', '--output', fonts / 'out/fonts',
        '--cache', fonts / 'out/cache', '--work', fonts / 'out/build', '--curl', 'curl')
    for name, expected in lock['fonts'].items():
        if hashlib.sha256((fonts / 'out/fonts' / name).read_bytes()).hexdigest() != expected:
            raise ValueError('Font hash mismatch: ' + name)
    flags = ['--remap-path-prefix=' + str(Path.home()) + '=/build', '--remap-path-prefix=' + str(base) + '=/toolchain',
             '--remap-path-prefix=' + str(root) + '=/build/sdk', '-C', 'debuginfo=0', '-C', 'linker=' + str(sdk / 'bin/mochios-cc')]
    metadata = dict(format=1, devkit_ref=revision, cargo=str(compiler / 'bin/cargo'), rustc=str(compiler / 'bin/rustc'),
                    rustdoc=str(compiler / 'bin/rustdoc'), target=str(root / 'user/targets/x86_64-unknown-mochios.json'),
                    target_dir=str(root / 'target'), rustflags=flags, cargo_config=configs(root),
                    appcore=str(devkit / 'crates/appcore'), user=str(root / 'user'),
                    font_licenses=str(fonts / 'out/fonts/licenses'), sources=lock,
                    fonts={key: str(fonts / 'out/fonts' / name) for key, name in [
                        ('VIEWKIT_UI_FONT_PATH', 'InterVariable.ttf'), ('VIEWKIT_MONOSPACE_FONT_PATH', 'UDEVGothic-Regular.ttf'),
                        ('VIEWKIT_JAPANESE_FONT_PATH', 'IBMPlexSansJP-Regular.ttf')]})
    description = root / 'app-build.json'
    description.write_text(json.dumps(metadata, indent=2) + '\n')
    if os.environ.get('GITHUB_OUTPUT'):
        with open(os.environ['GITHUB_OUTPUT'], 'a') as stream:
            stream.write('sdk=' + str(description) + '\nhelper=' + str(HERE / 'build.py') + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    prepare(parser.parse_args().output)
