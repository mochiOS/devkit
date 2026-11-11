import base64
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location('setup_signing', HERE / 'signing-setup.py')
setup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(setup)


class SetupFlow(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.package = Path(self.tmp.name) / 'App.mpkg'
        self.package.write_bytes(b'fixture')
        self.args = SimpleNamespace(repository='example/app', environment='release', branch='main',
            package_id='org.example.app', developer_id='a' * 32, build_run='123', build_workflow='.github/workflows/package.yml',
            sdk_ref='b' * 40, reviewer_team=123, artifacts=Path(self.tmp.name), tools=Path('/trusted/tools'))
        self.env = dict(GITHUB_ACTIONS='true', GITHUB_EVENT_NAME='workflow_dispatch', GITHUB_REPOSITORY='example/app',
            GITHUB_REF='refs/heads/main', GH_TOKEN='read-only-fixture', SIGNING_SETUP_TOKEN='write-fixture')
        self.run = dict(status='completed', conclusion='success', event='push', head_branch='main',
            path=self.args.build_workflow, repository=dict(full_name='example/app'),
            head_repository=dict(full_name='example/app'), head_sha='c' * 40)
        self.commands, self.saved = [], []
        self.names = {'SIGNING_SETUP_TOKEN'}
        self.fail_sign = False

    def command(self, args, **kwargs):
        self.commands.append((args, kwargs))
        self.assertTrue(all(key not in kwargs['env'] for key in ('GH_TOKEN', 'GITHUB_TOKEN', setup.SETUP_TOKEN, setup.RECOVERY, setup.IDENTITY)))
        if args[1] == 'keygen':
            for flag, byte in (('--private-key', 1), ('--public-key', 2)):
                path = Path(args[args.index(flag) + 1])
                if not path.exists():
                    path.write_bytes(base64.b64encode(bytes([byte]) * 32))
        if args[1] == 'sign':
            if self.fail_sign:
                raise subprocess.CalledProcessError(1, 'kome sign')
            Path(args[args.index('--certificate') + 1]).write_bytes(b'synthetic certificate')
            Path(args[args.index('--issuer-public-key') + 1]).write_bytes(base64.b64encode(b'3' * 32))
        return SimpleNamespace(returncode=0)

    def execute(self):
        with contextlib.ExitStack() as stack:
            stack.enter_context(patch.dict(os.environ, self.env, clear=True))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            stack.enter_context(patch.object(setup.release, 'approval_policy'))
            stack.enter_context(patch.object(setup.release, 'validate', return_value=(self.package, {}, 'v1')))
            stack.enter_context(patch.object(setup, 'api', return_value=self.run))
            stack.enter_context(patch.object(setup, 'gh', return_value=json.dumps(dict(total_count=len(self.names),secrets=[dict(name=n) for n in self.names]))))
            stack.enter_context(patch.object(setup, 'store', side_effect=lambda r,e,n,v:self.saved.append((n,v))))
            stack.enter_context(patch.object(setup.subprocess, 'run', side_effect=self.command))
            setup.initialize(self.args)

    def test_success_is_atomic_and_never_uploads_or_releases(self):
        self.execute()
        self.assertEqual([name for name,_ in self.saved], [setup.RECOVERY, setup.IDENTITY])
        identity = setup.release.decode_identity(self.saved[1][1])
        self.assertEqual(identity['package_id'],self.args.package_id)
        self.assertEqual(identity['developer_id'],self.args.developer_id)
        self.assertEqual([args[1] for args,_ in self.commands], ['keygen','login','developer','sign','logout'])
        for args,_ in self.commands:
            for flag in ('--private-key','--key','--certificate'):
                if flag in args:
                    self.assertFalse(Path(args[args.index(flag)+1]).exists())

    def test_issuance_failure_preserves_recovery_and_revokes_session(self):
        self.fail_sign=True
        with self.assertRaises(subprocess.CalledProcessError): self.execute()
        self.assertEqual([name for name,_ in self.saved],[setup.RECOVERY])
        self.assertEqual(self.commands[-1][0][1],'logout')

    def test_retry_restores_same_key_without_new_recovery_record(self):
        self.fail_sign=True
        with self.assertRaises(subprocess.CalledProcessError): self.execute()
        recovery=self.saved[0][1]
        self.env[setup.RECOVERY]=recovery;self.names.add(setup.RECOVERY)
        self.fail_sign=False;self.saved=[];self.commands=[]
        self.execute()
        identity=setup.release.decode_identity(self.saved[0][1])
        previous=json.loads(base64.b64decode(recovery))
        self.assertEqual(identity['private_key'],previous['private_key'])
        self.assertEqual([name for name,_ in self.saved],[setup.IDENTITY])

    def test_existing_identity_and_publication_enabled_are_rejected(self):
        self.names.add(setup.IDENTITY)
        with self.assertRaises(ValueError):self.execute()
        self.assertEqual(self.commands,[])
        self.names.remove(setup.IDENTITY);self.env['APP_RELEASE_ENABLED']='true'
        with self.assertRaises(ValueError):self.execute()
        self.assertEqual(self.commands,[])

    def test_local_pr_fork_and_wrong_branch_cannot_generate_keys(self):
        for event in ('pull_request','push'):
            self.env['GITHUB_EVENT_NAME']=event
            with self.assertRaises(ValueError):self.execute()
        self.env['GITHUB_EVENT_NAME']='workflow_dispatch'
        for field,value in [('event','pull_request'),('head_branch','staging'),('conclusion','failure'),('path','other.yml'),('head_repository',{'full_name':'fork/app'})]:
            previous=self.run[field];self.run[field]=value
            with self.assertRaises(ValueError):self.execute()
            self.run[field]=previous
        self.assertEqual(self.commands,[])

    def test_secret_transport_uses_stdin_and_environment_token_only(self):
        result=SimpleNamespace(returncode=0,stdout='')
        with patch.dict(os.environ,self.env,clear=True),patch.object(setup.subprocess,'run',return_value=result) as run:
            setup.gh(['secret','set','EXAMPLE'],token='write-fixture',data='private-fixture')
            self.assertNotIn('private-fixture',str(run.call_args.args))
            self.assertEqual(run.call_args.kwargs['input'],'private-fixture')
            self.assertEqual(run.call_args.kwargs['env']['GH_TOKEN'],'write-fixture')
            self.assertNotIn(setup.SETUP_TOKEN,run.call_args.kwargs['env'])


if __name__ == '__main__':unittest.main()
