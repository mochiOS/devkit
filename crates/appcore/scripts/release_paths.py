#!/usr/bin/env python3
"""Build AppCore with source-path remapping, and reject host paths before packaging."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys

POLICY_ENV = 'MOCHIOS_APPCORE_RELEASE_PATHS'


def capture(args, env=None, cwd=None):
    """Read compiler/Cargo metadata and propagate failures instead of using stale artifacts."""
    return subprocess.check_output([str(arg) for arg in args], env=env, cwd=cwd, text=True).strip()


def aliases(path):
    """Cover both logical and canonical paths when a checkout or cache uses symlinks."""
    absolute = Path(path).absolute()
    return sorted({str(absolute), str(absolute.resolve())})


def build(workspace, target, output_info):
    """Preserve Cargo flags/wrappers while remapping every dependency in a separate release cache."""
    workspace = workspace.resolve()
    env = os.environ.copy()
    metadata = json.loads(capture(['cargo', 'metadata', '--locked', '--format-version', '1',
                                  '--manifest-path', workspace / 'Cargo.toml'], env, workspace))
    compiler = env.get('RUSTC', 'rustc')
    sysroot = capture([compiler, '--print', 'sysroot'], env, workspace)
    base = workspace / 'target/appcore-release'
    mappings = []
    def add(source, destination):
        for path in aliases(source):
            if Path(path).parent == Path(path):
                raise ValueError('Refusing a filesystem-root path mapping')
            mappings.append([path, destination])
    home = env.get('HOME')
    if home:
        add(home, '/build-home')
    add(env.get('CARGO_HOME', str(Path.home() / '.cargo')), '/cargo')
    if env.get('RUSTUP_HOME'):
        add(env['RUSTUP_HOME'], '/toolchains')
    add(workspace, '/src/devkit')
    add(base, '/build')
    add(sysroot, '/toolchain')
    broad = list(mappings)
    for package in metadata['packages']:
        add(Path(package['manifest_path']).parent,
            '/src/crates/' + package['name'] + '-' + package['version'])
    # Rust applies the last matching rule: broad prefixes first, package paths last.
    mappings = sorted({tuple(pair) for pair in mappings}, key=lambda pair: (len(pair[0]), pair))
    wrapper = Path(__file__).resolve()
    inner = env.get('RUSTC_WRAPPER') or None
    if inner and Path(inner).resolve() == wrapper:
        raise ValueError('Recursive AppCore release compiler wrapper')
    policy = dict(format=1, mappings=mappings, inner_wrapper=inner,
                  wrapper_sha256=hashlib.sha256(wrapper.read_bytes()).hexdigest())
    identity = hashlib.sha256(json.dumps(policy, sort_keys=True).encode()).hexdigest()[:20]
    directory = base / identity
    directory.mkdir(parents=True, exist_ok=True)
    policy_file = directory / 'paths.json'
    policy_file.write_text(json.dumps(policy, indent=2) + '\n')
    env[POLICY_ENV] = str(policy_file)
    env['RUSTC_WRAPPER'] = str(wrapper)
    # C build scripts inherit equivalent remapping. Keep pre-existing compiler flags.
    c_maps = list(broad)
    for source, destination in mappings:
        if not any(source == root or source.startswith(root + os.sep) for root, _ in broad):
            c_maps.append((source, destination))
    c_maps.sort(key=lambda pair: (len(pair[0]), pair))
    c_flags = shlex.join([f'-ffile-prefix-map={source}={destination}' for source, destination in c_maps])
    for key in ['CFLAGS', 'CXXFLAGS']:
        env[key] = (env.get(key, '') + ' ' + c_flags).strip()
    args = ['cargo', 'build', '--manifest-path', str(workspace / 'Cargo.toml'),
            '--release', '--locked', '-p', 'mochios-appcore', '--target-dir', str(directory)]
    if target:
        args += ['--target', target]
    subprocess.run(args, env=env, cwd=workspace, check=True)
    binary_directory = directory
    if target:
        binary_directory /= Path(target).stem if target.endswith('.json') else target
    binary_directory /= 'release'
    output_info.write_text(json.dumps(dict(binary_directory=str(binary_directory), policy=str(policy_file))) + '\n')


def rustc(arguments):
    """Append remaps after Cargo's effective flags, preserving configured compiler-wrapper chains."""
    policy = json.loads(Path(os.environ[POLICY_ENV]).read_text())
    command = ([policy['inner_wrapper']] if policy['inner_wrapper'] else []) + arguments
    command += [f'--remap-path-prefix={source}={destination}' for source, destination in policy['mappings']]
    # Replacing this process preserves Cargo's jobserver file descriptors and signals.
    os.execvpe(command[0], command, os.environ)


def check(stage, policy_file):
    """Scan staged text, ELF, static archives and debug strings; never edit finished binaries."""
    policy = json.loads(policy_file.read_text())
    prefixes = {os.fsencode(source.rstrip('/')) for source, _ in policy['mappings']}
    patterns = [re.compile(re.escape(prefix) + rb'(?:/|(?=[\x00\r\n\t \"<>]|$))') for prefix in prefixes]
    patterns.append(re.compile(rb'/(?:home|Users)/[^/\x00\r\n\t "<>]+/'))
    patterns.append(re.compile(rb'/opt/hostedtoolcache/'))
    overlap = max([512, *[len(prefix) + 1 for prefix in prefixes]])
    offending = []
    for path in sorted(stage.rglob('*')):
        if path.is_symlink():
            raise ValueError('Unexpected staged release symlink: ' + str(path.relative_to(stage)))
        if not path.is_file():
            continue
        with path.open('rb') as stream:
            tail = b''
            while chunk := stream.read(1024 * 1024):
                data = tail + chunk
                if any(pattern.search(data) for pattern in patterns):
                    offending.append(str(path.relative_to(stage)))
                    break
                tail = data[-overlap:]
    if offending:
        raise ValueError('Build-environment paths remain; no archive will be produced: ' + ', '.join(offending))
    print('AppCore release path audit passed.')


def main():
    """Run a release build/audit, or serve as Cargo's transparent rustc wrapper."""
    if len(sys.argv) > 1 and sys.argv[1] not in ['build', 'check', '--help', '-h']:
        if POLICY_ENV not in os.environ:
            raise ValueError('Release path policy is missing')
        raise SystemExit(rustc(sys.argv[1:]))
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    builder = commands.add_parser('build')
    builder.add_argument('--workspace', type=Path, required=True)
    builder.add_argument('--target')
    builder.add_argument('--output-info', type=Path, required=True)
    auditor = commands.add_parser('check')
    auditor.add_argument('--stage', type=Path, required=True)
    auditor.add_argument('--policy', type=Path, required=True)
    args = parser.parse_args()
    if args.command == 'build':
        build(args.workspace, args.target, args.output_info)
    else:
        check(args.stage, args.policy)


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print('AppCore release: ' + str(error), file=sys.stderr)
        raise SystemExit(1)
