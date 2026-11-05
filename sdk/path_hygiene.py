#!/usr/bin/env python3
"""Remap SDK build paths and audit complete distributions before creating upload candidates."""
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import tarfile

POLICY_ENV = 'MOCHIOS_SDK_PATH_POLICY'


def make_policy(project, out, sources, rust_sysroot, environment=None):
    """Record real build/cache roots outside the SDK and map them to stable virtual source names."""
    environment = os.environ if environment is None else environment
    mappings = set()
    def add(source, destination):
        source = Path(source).absolute()
        for path in {source, source.resolve()}:
            if path.parent == path:
                raise ValueError('Refusing a filesystem-root prefix mapping')
            mappings.add((str(path), destination))
    if environment.get('HOME'):
        add(environment['HOME'], '/build-home')
    if environment.get('CARGO_HOME'):
        add(environment['CARGO_HOME'], '/cargo')
    if environment.get('RUSTUP_HOME'):
        add(environment['RUSTUP_HOME'], '/toolchains')
    add(project, '/src/devkit')
    add(out, '/build')
    add(out / 'cargo-home', '/cargo')
    add(rust_sysroot, '/toolchain')
    for name in ['user', 'tls', 'mnu', 'libc', 'mboot', 'newlib', 'llvm']:
        if name in sources:
            add(sources[name], '/src/' + name)
        if name in ['user', 'tls', 'newlib', 'llvm']:
            add(out / 'prepared' / name, '/src/' + name)
    inner = environment.get('RUSTC_WRAPPER') or None
    if inner and Path(inner).resolve() == Path(__file__).resolve():
        raise ValueError('Recursive SDK rustc wrapper')
    return dict(format=1, mappings=sorted(mappings, key=lambda pair: (len(pair[0]), pair)),
                inner_wrapper=inner,
                implementation_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest())


def identity(policy):
    """Separate Cargo caches when the source mappings or wrapper implementation change."""
    return hashlib.sha256(json.dumps(policy, sort_keys=True).encode()).hexdigest()[:20]


def compiler_flags(policy):
    """Return C/C++/preprocessed-assembly mappings for __FILE__, macros and debug source records."""
    return [f'-ffile-prefix-map={source}={destination}' for source, destination in policy['mappings']]


def rustc(arguments):
    """Preserve Cargo flags, existing wrapper chains, jobserver descriptors and compiler signals."""
    policy = json.loads(Path(os.environ[POLICY_ENV]).read_text())
    command = ([policy['inner_wrapper']] if policy['inner_wrapper'] else []) + arguments
    command += [f'--remap-path-prefix={source}={destination}' for source, destination in policy['mappings']]
    os.execvpe(command[0], command, os.environ)


def matchers(policy):
    """Reject the actual machine roots and known CI checkout roots, including partially remapped ones."""
    roots = [os.fsencode(source.rstrip('/')) for source, _ in policy['mappings']]
    patterns = [re.compile(re.escape(root) + rb'(?:/|(?=[\x00\r\n\t "<>]|$))') for root in roots]
    patterns += [re.compile(rb'/home/runner/work/|/Users/runner/work/|/opt/hostedtoolcache/'),
                 re.compile(rb'/build/work/(?:devkit|libs)/')]
    return patterns, max([512, *[len(root) + 1 for root in roots]])


def scan_stream(stream, label, policy):
    """Inspect raw strings in text, ELF, static archives and debug sections using bounded memory."""
    patterns, overlap = matchers(policy)
    tail = b''
    while chunk := stream.read(1024 * 1024):
        data = tail + chunk
        if any(pattern.search(data) for pattern in patterns):
            raise ValueError('Build-environment path remains in ' + label)
        tail = data[-overlap:]


def safe_member(member, seen):
    """Require relative archive names, safe links and deterministic owner/timestamp metadata."""
    path = PurePosixPath(member.name)
    if path.is_absolute() or '..' in path.parts or '\\' in member.name or member.name in seen:
        raise ValueError('Unsafe or duplicate archive path: ' + member.name)
    seen.add(member.name)
    if not (member.isfile() or member.isdir() or member.issym() or member.islnk()):
        raise ValueError('Unsupported archive entry: ' + member.name)
    if member.uid != 0 or member.gid != 0 or member.uname or member.gname or member.mtime != 0:
        raise ValueError('Build-machine archive metadata remains in ' + member.name)
    if member.issym() or member.islnk():
        link = PurePosixPath(member.linkname)
        if link.is_absolute() or '\\' in member.linkname:
            raise ValueError('Absolute archive link: ' + member.name)
        parts = list(path.parent.parts) if member.issym() else []
        for part in link.parts:
            if part == '..':
                if not parts:
                    raise ValueError('Archive link escapes its root: ' + member.name)
                parts.pop()
            elif part != '.':
                parts.append(part)


def audit_tar(tar, policy, label, depth=0):
    """Inspect all members, including the bundled newlib source archive, without extracting or executing."""
    if depth > 2:
        raise ValueError('Unexpected nested SDK archive depth')
    seen = set()
    files = archives = 0
    for member in tar:
        safe_member(member, seen)
        scan_stream(io.BytesIO(member.name.encode()), label + ':member name', policy)
        if member.issym() or member.islnk():
            scan_stream(io.BytesIO(member.linkname.encode()), label + ':link name', policy)
        if not member.isfile():
            continue
        files += 1
        if member.name.endswith(('.tar.gz', '.tar')):
            archives += 1
            with tar.extractfile(member) as stream, tarfile.open(fileobj=stream, mode='r|*') as nested:
                inner = audit_tar(nested, policy, label + '!' + member.name, depth + 1)
                files += inner['files_scanned']; archives += inner['archives_scanned']
        else:
            with tar.extractfile(member) as stream:
                scan_stream(stream, label + '!' + member.name, policy)
    return dict(files_scanned=files, archives_scanned=archives)


def audit_tree(directory, policy):
    """Fail before packaging if any distributed file, link or nested source archive retains host paths."""
    files = archives = 0
    for path in sorted(directory.rglob('*')):
        label = path.relative_to(directory).as_posix()
        if path.is_symlink():
            if not path.resolve().is_relative_to(directory.resolve()):
                raise ValueError('SDK symlink escapes its root: ' + label)
            scan_stream(io.BytesIO(os.fsencode(os.readlink(path))), label, policy)
        elif path.is_file():
            files += 1
            if path.name.endswith(('.tar.gz', '.tar')):
                archives += 1
                with tarfile.open(path) as tar:
                    result = audit_tar(tar, policy, label)
                    files += result['files_scanned']; archives += result['archives_scanned']
            else:
                with path.open('rb') as stream:
                    scan_stream(stream, label, policy)
    return dict(format=1, passed=True, files_scanned=files, archives_scanned=archives,
                verification='source-prefix-remapping-and-distribution-scan')


def archive_member(info):
    """Remove machine-specific tar owner, time and extended metadata while preserving file contents."""
    info.uid = info.gid = info.mtime = 0
    info.uname = info.gname = ''
    info.pax_headers = {}
    return info


def write_archive(source, destination, name, exclude_git=False):
    """Create deterministic gzip/tar metadata; original legal notices and source bytes remain intact."""
    def filter_member(info):
        if exclude_git and '.git' in PurePosixPath(info.name).parts:
            return None
        return archive_member(info)
    with destination.open('wb') as raw, gzip.GzipFile(fileobj=raw, mode='wb', filename='', mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode='w', format=tarfile.PAX_FORMAT) as tar:
            tar.add(source, arcname=name, filter=filter_member)


if __name__ == '__main__':
    try:
        if POLICY_ENV not in os.environ:
            raise ValueError('SDK path policy is missing')
        rustc(sys.argv[1:])
    except (ValueError, OSError) as error:
        print('SDK rustc wrapper: ' + str(error), file=sys.stderr)
        raise SystemExit(1)
