import base64
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('release_app', HERE / 'release.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ApprovedArtifact(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.commit = 'a' * 40
        self.identity = dict(id='net.example.editor', name='Editor', version='1.0.0', revision=1)
        data = b'[package]\nid="net.example.editor"\nname="Editor"\nversion="1.0.0"\nrevision=1\n'
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode='w') as tar:
            entry = tarfile.TarInfo('manifest.toml')
            entry.size = len(data)
            tar.addfile(entry, io.BytesIO(data))
        header = bytearray(32)
        header[:12] = b'MPKG\x01\0\0\0\x20\0\0\0'
        struct.pack_into('<Q', header, 12, len(stream.getvalue()))
        self.package = self.root / 'Editor.mpkg'
        self.package.write_bytes(header + stream.getvalue())
        (self.root / 'SHA256SUMS').write_text(release.digest(self.package) + '  Editor.mpkg\n')
        self.record = dict(format=1, source_commit=self.commit, devkit_ref=self.commit, signed=False, package=self.identity,
                           sources=json.loads((HERE / 'sources.lock.json').read_text()))
        self.save()

    def save(self):
        (self.root / 'build-info.json').write_text(json.dumps(self.record))

    def test_accepts_only_the_bound_artifact_and_source_commit(self):
        self.assertEqual(release.validate(self.root, self.identity['id'], self.commit, self.commit)[2], 'v1.0.0-r1')
        for identity, commit in [('net.attacker.app', self.commit), (self.identity['id'], 'b' * 40)]:
            with self.assertRaises(ValueError):
                release.validate(self.root, identity, commit, self.commit)

    def test_tampered_bytes_and_sdk_are_rejected(self):
        self.record['sources']['sdk']['sha256'] = 'b' * 64
        self.save()
        with self.assertRaises(ValueError):
            release.validate(self.root, self.identity['id'], self.commit, self.commit)
        self.record['sources'] = json.loads((HERE / 'sources.lock.json').read_text())
        self.save()
        with self.package.open('ab') as stream:
            stream.write(b'tampered')
        with self.assertRaises(ValueError):
            release.validate(self.root, self.identity['id'], self.commit, self.commit)

    def test_metadata_cannot_relabel_the_actual_manifest(self):
        self.record['package']['id'] = 'net.attacker.app'
        self.save()
        with self.assertRaises(ValueError):
            release.validate(self.root, 'net.attacker.app', self.commit, self.commit)

    def test_missing_keys_fail_before_invoking_a_signer(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(release.subprocess, 'run') as run:
            with self.assertRaises(ValueError):
                release.sign(self.package, self.root / 'signed.mpkg', Path('/msign'))
            run.assert_not_called()

    def test_credentials_and_github_tokens_do_not_enter_signer_environment(self):
        values = {key: base64.b64encode(b'fixture only').decode() for key in
                  ('MSIGN_CERTIFICATE_B64', 'MSIGN_PRIVATE_KEY_B64', 'MSIGN_ROOT_PUBLIC_KEY_B64')}
        values['GH_TOKEN'] = 'fixture-token'
        with patch.dict(os.environ, values, clear=True), patch.object(release.subprocess, 'run') as run:
            release.sign(self.package, self.root / 'signed.mpkg', Path('/msign'))
            self.assertEqual(run.call_count, 3)
            for call in run.call_args_list:
                self.assertTrue(all(key not in call.kwargs['env'] for key in values))
            self.assertIn('verify', run.call_args_list[-1].args[0])

    def test_atomic_identity_is_decoded_without_passing_it_to_the_signer(self):
        encoded_file = base64.b64encode(b'synthetic fixture').decode()
        identity = dict(format=1, developer_id='a'*32, package_id=self.identity['id'],
                        certificate=encoded_file, private_key=encoded_file, issuer_public_key=encoded_file)
        encoded = base64.b64encode(json.dumps(identity).encode()).decode()
        with patch.dict(os.environ, {'MSIGN_IDENTITY_B64':encoded,'SIGNING_SETUP_TOKEN':'fixture'}, clear=True), patch.object(release.subprocess,'run') as run:
            release.sign(self.package,self.root/'signed.mpkg',Path('/msign'))
            self.assertEqual(run.call_count,3)
            self.assertNotIn('MSIGN_IDENTITY_B64',run.call_args.kwargs['env'])
            self.assertNotIn('SIGNING_SETUP_TOKEN',run.call_args.kwargs['env'])
        with patch.dict(os.environ, {'MSIGN_IDENTITY_B64':encoded,'MSIGN_PRIVATE_KEY_B64':encoded_file}, clear=True), patch.object(release.subprocess,'run') as run:
            with self.assertRaises(ValueError):release.sign(self.package,self.root/'signed.mpkg',Path('/msign'))
            run.assert_not_called()


class PublicationPolicy(unittest.TestCase):
    def setUp(self):
        self.policy = dict(can_admins_bypass=False,
            protection_rules=[dict(type='required_reviewers', reviewers=[dict(type='Team', reviewer=dict(id=123))])],
            deployment_branch_policy=dict(protected_branches=False, custom_branch_policies=True))
        self.branches = dict(total_count=1, branch_policies=[dict(type='branch', name='main')])

    def check(self):
        with patch.object(release.subprocess, 'check_output', side_effect=[json.dumps(self.policy), json.dumps(self.branches)]):
            release.approval_policy('example/app', 'release', 123, 'main')

    def test_explicit_team_and_single_branch_are_required(self):
        self.check()
        self.policy['protection_rules'][0]['reviewers'] = []
        with self.assertRaises(ValueError):
            self.check()

    def test_alternative_reviewers_and_admin_bypass_are_rejected(self):
        self.policy['protection_rules'][0]['reviewers'].append(dict(type='User', reviewer=dict(id=456)))
        with self.assertRaises(ValueError):
            self.check()
        self.policy['protection_rules'][0]['reviewers'].pop()
        self.policy['can_admins_bypass'] = True
        with self.assertRaises(ValueError):
            self.check()

    def test_tags_wildcards_and_additional_branch_rules_are_rejected(self):
        for entries in ([dict(type='tag', name='main')], [dict(type='branch', name='*')],
                        [dict(type='branch', name='main'), dict(type='branch', name='staging')]):
            self.branches = dict(total_count=len(entries), branch_policies=entries)
            with self.assertRaises(ValueError):
                self.check()

    def test_api_failure_does_not_skip_policy_checks(self):
        with patch.object(release.subprocess, 'check_output', side_effect=release.subprocess.CalledProcessError(1, 'gh')):
            with self.assertRaises(release.subprocess.CalledProcessError):
                release.approval_policy('example/app', 'release', 123, 'main')


if __name__ == '__main__':
    unittest.main()
