#!/usr/bin/env python3
"""Sign and publish one approved build artifact; never compile application source."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
import time
import tomllib

HERE = Path(__file__).resolve().parent


def digest(path):
    """Hash the complete artifact without trusting build metadata."""
    result = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            result.update(chunk)
    return result.hexdigest()


def validate(directory, expected_id, expected_commit, expected_sdk_ref):
    """Bind the approved source revision, SDK, package identity, and actual bytes."""
    if not re.fullmatch('[0-9a-f]{40}', expected_commit) or not re.fullmatch('[0-9a-f]{40}', expected_sdk_ref):
        raise ValueError('Release requires an exact source commit')
    metadata = directory / 'build-info.json'
    checksums = directory / 'SHA256SUMS'
    packages = list(directory.glob('*.mpkg'))
    if len(packages) != 1:
        raise ValueError('Expected exactly one unsigned MPKG')
    package = packages[0]
    if any(p.is_symlink() or not p.is_file() for p in (metadata, checksums, package)):
        raise ValueError('Release artifacts must be ordinary files')
    if metadata.stat().st_size > 1024 * 1024 or checksums.stat().st_size > 4096:
        raise ValueError('Artifact metadata exceeds the size limit')
    record = json.loads(metadata.read_text())
    lock = json.loads((HERE / 'sources.lock.json').read_text())
    if (record.get('format') != 1 or record.get('source_commit') != expected_commit
            or record.get('signed') is not False or record.get('sources') != lock or record.get('devkit_ref') != expected_sdk_ref):
        raise ValueError('Artifact source/SDK does not match the approved release')
    if checksums.read_text() != digest(package) + '  ' + package.name + '\n':
        raise ValueError('Unsigned artifact checksum mismatch')
    with package.open('rb') as stream:
        header = stream.read(32)
        if len(header) != 32 or header[:12] != b'MPKG\x01\0\0\0\x20\0\0\0':
            raise ValueError('Expected unsigned MPKG v1')
        with tarfile.open(fileobj=stream, mode='r|') as archive:
            first = next(iter(archive))
            if first.name != 'manifest.toml' or not first.isfile() or first.size > 1024 * 1024:
                raise ValueError('Invalid MPKG manifest')
            manifest = tomllib.loads(archive.extractfile(first).read().decode('utf-8'))
    identity = manifest.get('package', {})
    if identity != record.get('package') or identity.get('id') != expected_id or 'provenance' in identity:
        raise ValueError('MPKG identity differs from the approved application')
    version, revision = identity.get('version'), identity.get('revision')
    if (not isinstance(version, str) or not re.fullmatch('[0-9][0-9A-Za-z.+-]*', version)
            or type(revision) is not int or revision < 1):
        raise ValueError('Invalid release version/revision')
    return package, identity, 'v' + version + '-r' + str(revision)


def sign(package, output, msign):
    """Use explicit environment credentials, verify the result, and remove temporary keys."""
    required = ('MSIGN_CERTIFICATE_B64', 'MSIGN_PRIVATE_KEY_B64', 'MSIGN_ROOT_PUBLIC_KEY_B64')
    if any(not os.environ.get(key) for key in required):
        raise ValueError('Signing certificate, private key, and trusted root public key must be configured before enabling releases')
    with tempfile.TemporaryDirectory(prefix='mpkg-signing-') as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        paths = []
        for key in required:
            data = base64.b64decode(os.environ[key], validate=True)
            if not data or len(data) > 1024 * 1024:
                raise ValueError('Invalid signing credential file')
            path = root / key
            with path.open('xb') as stream:
                path.chmod(0o600)
                stream.write(data)
            paths.append(path)
        certificate, private, issuer = paths
        env = dict(os.environ, MOCHIOS_SIGNING_HOME=str(root / 'identity'))
        # Do not propagate the credential values to subprocess environments.
        for key in required:
            env.pop(key, None)
        for key in ('GH_TOKEN', 'GITHUB_TOKEN'):
            env.pop(key, None)
        subprocess.run([str(msign), 'identity', 'import', 'ci-release', '--certificate', str(certificate), '--key', str(private)], env=env, check=True)
        subprocess.run([str(msign), 'package', 'sign-auto', str(package), '--identity', 'ci-release', '--output', str(output)], env=env, check=True)
        subprocess.run([str(msign), 'package', 'verify', str(output), '--root-public-key', str(issuer), '--unix-time', str(int(time.time()))], env=env, check=True)


def absent(repository, path):
    """Require a definite 404; authentication/network failures are never interpreted as absence."""
    result = subprocess.run(['gh', 'api', '-i', 'repos/' + repository + '/' + path], capture_output=True, text=True)
    if result.returncode == 0 or 'HTTP 404' not in result.stderr:
        raise ValueError('Release/tag already exists or could not be checked; refusing to overwrite')


def main():
    """Only an approval-protected caller should execute this publication command."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--msign', type=Path, required=True)
    parser.add_argument('--package-id', required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--sdk-ref', required=True)
    parser.add_argument('--repository', required=True)
    args = parser.parse_args()
    if not re.fullmatch('[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', args.repository):
        raise ValueError('Invalid release repository')
    package, identity, tag = validate(args.artifacts.resolve(), args.package_id, args.commit, args.sdk_ref)
    absent(args.repository, 'releases/tags/' + tag)
    absent(args.repository, 'git/ref/tags/' + tag)
    with tempfile.TemporaryDirectory(prefix='mpkg-release-') as temporary:
        root = Path(temporary)
        output = root / package.name
        sign(package, output, args.msign.resolve())
        checksums = root / 'SHA256SUMS'
        checksums.write_text(digest(output) + '  ' + output.name + '\n')
        notes = root / 'notes.md'
        notes.write_text('Native mochiOS application package.\n\nSource commit: `' + args.commit + '`\n')
        subprocess.run(['gh', 'release', 'create', tag, str(output), str(checksums), '--repo', args.repository,
                        '--target', args.commit, '--title', identity['name'] + ' ' + identity['version'] + ' (r' + str(identity['revision']) + ')',
                        '--prerelease', '--notes-file', str(notes)], check=True)


if __name__ == '__main__':
    main()
