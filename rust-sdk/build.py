#!/usr/bin/env python3
"""Build/test a Rust app and stage an unsigned package for the public MPKG Action."""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import struct
import subprocess
import tomllib


def run(args, **kwargs):
    """Preserve real compiler and test failures with streaming build output."""
    subprocess.run([str(a) for a in args], check=True, **kwargs)


def dumps(data):
    """Write ordinary Cargo/MPKG tables, preserving nested metadata and array tables."""
    lines = []
    def table(mapping, path=(), header=None):
        if header:
            lines.append(header)
        children = []
        for key, value in mapping.items():
            if isinstance(value, dict) or (isinstance(value, list) and value and isinstance(value[0], dict)):
                children.append((key, value))
            else:
                lines.append(json.dumps(key) + ' = ' + json.dumps(value, ensure_ascii=True, allow_nan=False))
        for key, value in children:
            child = (*path, key)
            name = '.'.join(json.dumps(p) for p in child)
            for item in value if isinstance(value, list) else [value]:
                lines.append('')
                table(item, child, '[[' + name + ']]' if isinstance(value, list) else '[' + name + ']')
    table(data)
    return '\n'.join(lines) + '\n'


def resource(source, name):
    """Find a declared bundle-local resource without following links outside source."""
    path = PurePosixPath(name)
    if not name or path.is_absolute() or '..' in path.parts or '\\' in name:
        raise ValueError('Invalid resource path: ' + name)
    for candidate in (source / name, source / 'resources' / name):
        if candidate.is_file() and candidate.resolve().is_relative_to(source.resolve()):
            return candidate
    raise ValueError('Missing or external resource: ' + name)


def stage(source, binary, output, licenses=None):
    """Preserve capabilities/associations and bind actual files without claiming built-in trust."""
    with binary.open('rb') as stream:
        header = stream.read(64)
    if len(header) != 64 or header[:6] != b'\x7fELF\x02\x01' or struct.unpack_from('<HH', header, 16) != (2, 62):
        raise ValueError('Expected a native x86-64 ELF executable')
    manifest = copy.deepcopy(tomllib.loads((source / 'manifest.toml').read_text()))
    package, app = manifest['package'], manifest['application']
    package.pop('provenance', None)
    name = package['name']
    if not re.fullmatch('[A-Za-z][A-Za-z0-9_-]*', name) or app['entry'] != 'entry.elf' or len(manifest.get('binary', [])) != 1:
        raise ValueError('Expected a single-entry native application')
    if output.exists():
        raise ValueError('Use a fresh package stage')
    bundle = output / 'payload/bundle'
    bundle.mkdir(parents=True)
    shutil.copyfile(binary, bundle / 'entry.elf')
    (bundle / 'entry.elf').chmod(0o755)
    for name in dict.fromkeys([app['icon'], *app.get('resources', [])]):
        target = bundle / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(resource(source, name), target)
        target.chmod(0o644)
    if licenses and licenses.is_dir():
        shutil.copytree(licenses, bundle / 'licenses/fonts')
    manifest['file'] = []
    for index, path in enumerate(sorted(bundle.rglob('*'))):
        if path.is_file():
            relative = path.relative_to(bundle).as_posix()
            path.chmod(0o755 if relative == 'entry.elf' else 0o644)
            manifest['file'].append(dict(id='f' + str(index), path='$/' + relative,
                digest='sha256:' + hashlib.sha256(path.read_bytes()).hexdigest(), size=path.stat().st_size,
                mode='0755' if relative == 'entry.elf' else '0644'))
    entry = next(f for f in manifest['file'] if f['path'] == '$/entry.elf')
    manifest['binary'][0].update(path='/applications/' + package['name'] + '.app/entry.elf', file=entry['id'])
    (output / 'manifest.toml').write_text(dumps(manifest))
    return manifest


def build(source, output, sdk, tests):
    """Build independently of the caller's checkout layout; never change its lockfile."""
    project = output / 'project'
    project.mkdir(parents=True)
    manifest = tomllib.loads((source / 'Cargo.toml').read_text())
    for group in [manifest.get('dependencies', {})] + [v.get('dependencies', {}) for v in manifest.get('target', {}).values()]:
        for dependency in group.values():
            if not isinstance(dependency, dict) or 'path' not in dependency:
                continue
            path = dependency['path']
            if path == '../../tools/devkit/crates/appcore':
                dependency['path'] = sdk['appcore']
            elif path.startswith('../../user/crates/'):
                dependency['path'] = str(Path(sdk['user']) / 'crates' / path.removeprefix('../../user/crates/'))
            elif (source / path).resolve().is_relative_to(source):
                dependency['path'] = str((source / path).resolve())
            else:
                raise ValueError('External path dependency needs an explicit SDK mapping: ' + path)
    for child in source.iterdir():
        if child.name not in ('.git', '.github', 'out', 'target', 'Cargo.toml', 'Cargo.lock'):
            (project / child.name).symlink_to(child.resolve(), target_is_directory=child.is_dir())
    (project / 'Cargo.toml').write_text(dumps(manifest))
    shutil.copyfile(source / 'Cargo.lock', project / 'Cargo.lock')
    options = []
    for config in sdk['cargo_config']:
        options += ['--config', config]
    host_env = dict(os.environ, **sdk['fonts'], CARGO_PROFILE_TEST_DEBUG='0')
    for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTC', 'RUSTDOC'):
        host_env.pop(key, None)
    host_env.update(RUSTC=sdk['rustc'], RUSTDOC=sdk['rustdoc'])
    if tests:
        run([sdk['cargo'], 'test', '--manifest-path', project / 'Cargo.toml', '--target-dir', output / 'host-target', *options],
            env=host_env, cwd=project)
    env = dict(host_env, RUSTC=sdk['rustc'], RUSTDOC=sdk['rustdoc'],
               CARGO_ENCODED_RUSTFLAGS='\x1f'.join(sdk['rustflags'] + ['--remap-path-prefix=' + str(source) + '=/src/app',
                 '--remap-path-prefix=' + str(output) + '=/build/app']))
    run([sdk['cargo'], 'build', '--release', '-Z', 'build-std=std,panic_abort,compiler_builtins', '-Z', 'json-target-spec',
         '--manifest-path', project / 'Cargo.toml', '--target', sdk['target'], '--target-dir', sdk['target_dir'],
         '--bin', manifest['package']['name'], *options], env=env, cwd=project)
    shutil.copyfile(project / 'Cargo.lock', output / 'resolved.Cargo.lock')
    binary = Path(sdk['target_dir']) / Path(sdk['target']).stem / 'release' / manifest['package']['name']
    for option in ('-l', '-d', '--version-info'):
        text = subprocess.check_output(['readelf', option, str(binary)], text=True)
        if any(value in text for value in ('GLIBC_', 'libc.so.6', '/ld-linux', '(RUNPATH)', '(RPATH)')):
            raise ValueError('Host linkage or runtime search path in native executable')
    data = binary.read_bytes()
    for prefix in (str(source), str(output), str(Path.home())):
        if prefix.encode() in data:
            raise ValueError('Build-host path in executable')
    package = stage(source, binary, output / 'stage', Path(sdk['font_licenses']))
    revision = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip()
    (output / 'build-info.json').write_text(json.dumps(dict(format=1, source_commit=revision,
        package=package['package'], sources=sdk['sources'], devkit_ref=sdk['devkit_ref'], runtime_tested=False, signed=False), indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--sdk', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--test', action='store_true')
    args = parser.parse_args()
    build(args.source.resolve(), args.output.resolve(), json.loads(args.sdk.read_text()), args.test)
