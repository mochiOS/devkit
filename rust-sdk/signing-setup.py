#!/usr/bin/env python3
"""Initialize a package-scoped identity on an approval-protected GitHub runner."""
import argparse
import base64
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
from urllib.parse import quote

import release

RECOVERY = 'MSIGN_SETUP_KEYPAIR_B64'
IDENTITY = 'MSIGN_IDENTITY_B64'
SETUP_TOKEN = 'SIGNING_SETUP_TOKEN'


def child_environment():
    """Never give signing tools GitHub tokens or stored identity/recovery values."""
    return {key: value for key, value in os.environ.items()
            if key not in ('GH_TOKEN', 'GITHUB_TOKEN', SETUP_TOKEN, IDENTITY, RECOVERY)
            and not key.startswith('MSIGN_')}


def gh(args, *, token=None, data=None, capture=False):
    """Send secret values via stdin; suppress API diagnostics that could expose inputs."""
    env = child_environment()
    env['GH_TOKEN'] = token or os.environ.get('GH_TOKEN', '')
    result = subprocess.run(['gh', *args], env=env, input=data, text=True,
                            stdout=subprocess.PIPE if capture else subprocess.DEVNULL, stderr=subprocess.PIPE)
    if result.returncode:
        raise RuntimeError('GitHub operation failed; check token permissions and expiration (secret values withheld)')
    return result.stdout if capture else None


def api(path):
    """Read workflow/run metadata using the job's read-only GitHub token."""
    return json.loads(gh(['api', path], capture=True))


def encode(data):
    """Encode a file or record for write-only GitHub Secret storage."""
    return base64.b64encode(data).decode('ascii')


def mask(value):
    """Register dynamically generated secrets with the Actions log redactor."""
    if os.environ.get('GITHUB_ACTIONS') == 'true':
        print('::add-mask::' + value.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A'), flush=True)


def store(repository, environment, name, value):
    """Encrypt and save a Secret to the explicitly approved Environment via gh."""
    mask(value)
    gh(['secret', 'set', name, '--repo', repository, '--env', environment],
       token=os.environ[SETUP_TOKEN], data=value)


def verify_run(record, repository, branch, workflow):
    """Reject failed runs, PR/fork artifacts, and builds from another workflow/branch."""
    if (record.get('status') != 'completed' or record.get('conclusion') != 'success'
            or record.get('event') not in ('push', 'workflow_dispatch') or record.get('head_branch') != branch
            or record.get('path') != workflow or record.get('repository', {}).get('full_name') != repository
            or record.get('head_repository', {}).get('full_name') != repository
            or not re.fullmatch('[0-9a-f]{40}', record.get('head_sha', ''))):
        raise ValueError('Select a successful default-branch build from the approved package workflow')
    return record['head_sha']


def restore_recovery(encoded, package_id, developer_id, private, public):
    """Restore the same key after interruption; never generate a silent replacement."""
    if len(encoded) > 64 * 1024:
        raise ValueError('Recovery record exceeds the size limit')
    value = json.loads(base64.b64decode(encoded, validate=True))
    if (set(value) != {'format', 'package_id', 'developer_id', 'private_key', 'public_key'}
            or type(value['format']) is not int or value['format'] != 1
            or value['package_id'] != package_id or value['developer_id'] != developer_id):
        raise ValueError('Recovery record belongs to a different package/developer')
    for path, key in ((private, 'private_key'), (public, 'public_key')):
        data = base64.b64decode(value[key], validate=True)
        raw = base64.b64decode(data.strip(), validate=True)
        if len(raw) != 32:
            raise ValueError('Invalid recovery key file')
        mask(data.decode().strip())
        with path.open('xb') as stream:
            path.chmod(0o600)
            stream.write(data)


def initialize(args):
    """Generate, recover, obtain, verify, and atomically save one identity; never release."""
    if (os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('GITHUB_EVENT_NAME') != 'workflow_dispatch'
            or os.environ.get('GITHUB_REPOSITORY') != args.repository
            or os.environ.get('GITHUB_REF') != 'refs/heads/' + args.branch):
        raise ValueError('Signing initialization runs only in the default-branch Actions job')
    if os.environ.get('APP_RELEASE_ENABLED') == 'true':
        raise ValueError('Disable publication before initializing signing credentials')
    if not os.environ.get(SETUP_TOKEN):
        raise ValueError('Configure the short-lived Environment-write SIGNING_SETUP_TOKEN first')
    release.approval_policy(args.repository, args.environment, args.reviewer_team, args.branch)
    commit = verify_run(api('repos/' + args.repository + '/actions/runs/' + args.build_run),
                        args.repository, args.branch, args.build_workflow)
    package, _, _ = release.validate(args.artifacts.resolve(), args.package_id, commit, args.sdk_ref)
    endpoint = 'repos/' + args.repository + '/environments/' + quote(args.environment, safe='') + '/secrets?per_page=100'
    listed = json.loads(gh(['api', endpoint], token=os.environ[SETUP_TOKEN], capture=True))
    names = {entry['name'] for entry in listed['secrets']}
    if listed['total_count'] != len(names):
        raise ValueError('Could not completely inspect the Environment Secret inventory')
    if names & {IDENTITY, 'MSIGN_PRIVATE_KEY_B64', 'MSIGN_CERTIFICATE_B64'}:
        raise ValueError('Signing is already initialized; this workflow never rotates existing credentials')
    recovery = os.environ.get(RECOVERY, '')
    if (RECOVERY in names) != bool(recovery):
        raise ValueError('Recovery state changed; start a fresh approved setup run')
    with tempfile.TemporaryDirectory(prefix='signing-setup-') as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        private, public = root / 'application.key', root / 'application.pub'
        env = child_environment()
        env.update(PATH=str(args.tools.resolve()) + os.pathsep + env.get('PATH', ''),
                   KOME_CONFIG_HOME=str(root / 'session'), DBUS_SESSION_BUS_ADDRESS='unix:path=' + str(root / 'no-session-bus'))
        if recovery:
            restore_recovery(recovery, args.package_id, args.developer_id, private, public)
        subprocess.run([str(args.tools / 'kome'), 'keygen', '--private-key', str(private), '--public-key', str(public)],
                       env=env, cwd=root, check=True)
        mask(private.read_text().strip())
        if not recovery:
            record = dict(format=1, package_id=args.package_id, developer_id=args.developer_id,
                          private_key=encode(private.read_bytes()), public_key=encode(public.read_bytes()))
            store(args.repository, args.environment, RECOVERY, encode(json.dumps(record).encode()))
        certificate, issuer = root / 'developer.cert', root / 'issuer.pub'
        try:
            print('Authorize Kome using the public URL/code shown below. Never paste tokens into logs.', flush=True)
            subprocess.run([str(args.tools / 'kome'), 'login', '--no-browser'], env=env, cwd=root, check=True)
            subprocess.run([str(args.tools / 'kome'), 'developer', 'use', args.developer_id], env=env, cwd=root, check=True)
            subprocess.run([str(args.tools / 'kome'), 'sign', str(package), '--key', str(private), '--public-key', str(public),
                            '--certificate', str(certificate), '--issuer-public-key', str(issuer),
                            '--output', str(root / 'verification.mpkg')], env=env, cwd=root, check=True)
            identity = dict(format=1, package_id=args.package_id, developer_id=args.developer_id,
                            private_key=encode(private.read_bytes()), certificate=encode(certificate.read_bytes()),
                            issuer_public_key=encode(issuer.read_bytes()))
            encoded = encode(json.dumps(identity).encode())
            release.decode_identity(encoded)
            store(args.repository, args.environment, IDENTITY, encoded)
        finally:
            # Revoke the temporary CLI session; no refresh token is uploaded or retained.
            subprocess.run([str(args.tools / 'kome'), 'logout'], env=env, cwd=root,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
        for name in (RECOVERY, SETUP_TOKEN):
            gh(['secret', 'delete', name, '--repo', args.repository, '--env', args.environment], token=os.environ[SETUP_TOKEN])
        print('Signing identity saved and temporary setup Secrets removed. Publication remains disabled.', flush=True)


def main():
    """Parse trusted caller inputs without accepting arbitrary credential paths or publication commands."""
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('repository', 'environment', 'branch', 'package-id', 'developer-id', 'build-run', 'build-workflow', 'sdk-ref'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--reviewer-team', type=int, required=True)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--tools', type=Path, required=True)
    args = parser.parse_args()
    if (not re.fullmatch('[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', args.repository)
            or not re.fullmatch('[0-9a-f]{32}', args.developer_id) or not args.build_run.isdecimal()):
        raise ValueError('Invalid setup input')
    initialize(args)


if __name__ == '__main__':
    main()
