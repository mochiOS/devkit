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
from urllib.parse import quote

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
    values = {key: os.environ.get(key) for key in required}
    if os.environ.get('MSIGN_IDENTITY_B64'):
        if any(values.values()):
            raise ValueError('Configure one atomic identity or legacy signing values, not both')
        identity = decode_identity(os.environ['MSIGN_IDENTITY_B64'])
        values = dict(zip(required, (identity['certificate'], identity['private_key'], identity['issuer_public_key'])))
    if any(not values[key] for key in required):
        raise ValueError('Signing certificate, private key, and trusted root public key must be configured before enabling releases')
    with tempfile.TemporaryDirectory(prefix='mpkg-signing-') as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        paths = []
        for key in required:
            data = base64.b64decode(values[key], validate=True)
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
        env.pop('MSIGN_IDENTITY_B64', None)
        env.pop('SIGNING_SETUP_TOKEN', None)
        env.pop('MSIGN_SETUP_KEYPAIR_B64', None)
        for key in ('GH_TOKEN', 'GITHUB_TOKEN'):
            env.pop(key, None)
        subprocess.run([str(msign), 'identity', 'import', 'ci-release', '--certificate', str(certificate), '--key', str(private)], env=env, check=True)
        subprocess.run([str(msign), 'package', 'sign-auto', str(package), '--identity', 'ci-release', '--output', str(output)], env=env, check=True)
        subprocess.run([str(msign), 'package', 'verify', str(output), '--root-public-key', str(issuer), '--unix-time', str(int(time.time()))], env=env, check=True)


def decode_identity(encoded):
    """Decode an atomically stored signing identity without printing its contents."""
    if len(encoded) > 64 * 1024:
        raise ValueError('Signing identity exceeds the size limit')
    value = json.loads(base64.b64decode(encoded, validate=True))
    expected = {'format', 'developer_id', 'package_id', 'certificate', 'private_key', 'issuer_public_key'}
    if (not isinstance(value, dict) or set(value) != expected or type(value['format']) is not int
            or value['format'] != 1 or not re.fullmatch('[0-9a-f]{32}', value['developer_id'])
            or not re.fullmatch('[a-z0-9]+(?:[.-][a-z0-9]+)+', value['package_id'])):
        raise ValueError('Invalid signing identity metadata')
    for key in ('certificate', 'private_key', 'issuer_public_key'):
        if not isinstance(value[key], str) or not base64.b64decode(value[key], validate=True):
            raise ValueError('Invalid signing identity file')
    return value


def absent(repository, path):
    """Require a definite 404; authentication/network failures are never interpreted as absence."""
    result = subprocess.run(['gh', 'api', '-i', 'repos/' + repository + '/' + path], capture_output=True, text=True)
    if result.returncode == 0 or 'HTTP 404' not in result.stderr:
        raise ValueError('Release/tag already exists or could not be checked; refusing to overwrite')


def approval_policy(repository, environment, team, branch):
    """Fail closed if the caller's required Team or branch restriction was removed."""
    endpoint = 'repos/' + repository + '/environments/' + quote(environment, safe='')
    def fetch(path):
        return json.loads(subprocess.check_output(['gh', 'api', path], text=True))
    policy = fetch(endpoint)
    reviewers = [reviewer for rule in policy.get('protection_rules', [])
                 if rule.get('type') == 'required_reviewers' for reviewer in rule.get('reviewers', [])]
    # GitHub accepts approval from any listed reviewer. Do not allow a different
    # user/team to substitute for the Team explicitly selected by the publisher.
    if (policy.get('can_admins_bypass') is not False or len(reviewers) != 1
            or reviewers[0].get('type') != 'Team' or reviewers[0].get('reviewer', {}).get('id') != team
            or policy.get('deployment_branch_policy') != dict(protected_branches=False, custom_branch_policies=True)):
        raise ValueError('Required Team approval or environment restrictions are missing')
    branches = fetch(endpoint + '/deployment-branch-policies?per_page=100')
    entries = branches.get('branch_policies', [])
    if (branches.get('total_count') != 1 or len(entries) != 1
            or entries[0].get('name') != branch or entries[0].get('type') != 'branch'):
        raise ValueError('Publication environment must allow only the selected branch')


def main():
    """Only an approval-protected caller should execute this publication command."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--msign', type=Path, required=True)
    parser.add_argument('--package-id', required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--sdk-ref', required=True)
    parser.add_argument('--repository', required=True)
    parser.add_argument('--environment', required=True)
    parser.add_argument('--reviewer-team', type=int, required=True)
    parser.add_argument('--branch', required=True)
    args = parser.parse_args()
    if not re.fullmatch('[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', args.repository):
        raise ValueError('Invalid release repository')
    package, identity, tag = validate(args.artifacts.resolve(), args.package_id, args.commit, args.sdk_ref)
    approval_policy(args.repository, args.environment, args.reviewer_team, args.branch)
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
