#!/usr/bin/env python3
"""Public MPKG Action: no publisher policy, credentials, signing, or publication."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib


def digest(path):
    """Hash a package without loading it into memory."""
    result = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            result.update(chunk)
    return result.hexdigest()


def checked_path(value, label):
    """Reject multiline Action outputs and ambiguous filesystem names."""
    if not value or any(c in value for c in ('\r', '\n', '\0')):
        raise ValueError('Invalid ' + label + ' path')
    return Path(value).absolute()


def validate(manifest, payload, output):
    """Check input boundaries without inventing, rewriting, or trusting package metadata."""
    if not manifest.is_file() or not payload.is_dir():
        raise ValueError('A regular manifest file and payload directory are required')
    data = tomllib.loads(manifest.read_text(encoding='utf-8'))
    if type(data.get('format')) is not int or data['format'] != 1 or not isinstance(data.get('package'), dict):
        raise ValueError('Expected a completed format-1 MPKG manifest')
    if output.exists() or output.is_symlink():
        raise ValueError('Refusing to overwrite an existing output')
    if output.resolve().is_relative_to(payload.resolve()) or output.resolve() == manifest.resolve():
        raise ValueError('Output must be outside the payload and must not alias the manifest')
    checksum = output.parent / 'SHA256SUMS'
    if checksum.exists() or checksum.is_symlink():
        raise ValueError('Refusing to overwrite an existing SHA256SUMS file; use a fresh output directory')
    return checksum


def bundled_tool(action, temporary):
    """Build only mpack from this Action revision, independently of SDK/OS workspaces."""
    devkit = action.parents[1]
    project = temporary / 'mpack'
    project.mkdir()
    shutil.copyfile(devkit / 'crates/mpack/Cargo.toml', project / 'Cargo.toml')
    shutil.copyfile(action / 'tool.Cargo.lock', project / 'Cargo.lock')
    shutil.copytree(devkit / 'crates/mpack/src', project / 'src')
    # Cargo resolves a standalone tool, never the AppCore/compiler workspace.
    with (project / 'Cargo.toml').open('a') as stream:
        stream.write('\n[workspace]\n')
    target = temporary / 'target'
    env = os.environ.copy()
    for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTC', 'RUSTDOC', 'CARGO_BUILD_TARGET'):
        env.pop(key, None)
    subprocess.run(['cargo', 'build', '--locked', '--release', '--manifest-path', str(project / 'Cargo.toml'),
                    '--target-dir', str(target)], check=True, cwd=temporary, env=env)
    return target / 'release' / ('mpack.exe' if os.name == 'nt' else 'mpack')


def create(manifest, payload, output, tool):
    """Run the ordinary packager and attest file integrity, never signature validity."""
    checksum = validate(manifest, payload, output)
    output.parent.mkdir(parents=True, exist_ok=True)
    # Publish completed outputs only. Failed mpack cannot leave a final artifact.
    with tempfile.TemporaryDirectory(prefix='.mpkg-', dir=output.parent) as folder:
        staged = Path(folder) / 'package.mpkg'
        subprocess.run([str(tool), 'create', '--manifest', str(manifest), '--payload', str(payload),
                        '--output', str(staged)], check=True)
        with staged.open('rb') as stream:
            header = stream.read(32)
        if (len(header) != 32 or header[:12] != b'MPKG\x01\0\0\0\x20\0\0\0'
                or header[20:] != bytes(12) or int.from_bytes(header[12:20], 'little') != staged.stat().st_size - 32):
            raise ValueError('mpack did not generate an unsigned MPKG v1')
        sha256 = digest(staged)
        staged_checksum = Path(folder) / 'SHA256SUMS'
        staged_checksum.write_text(sha256 + '  ' + output.name + '\n', encoding='utf-8')
        # Same-filesystem links publish complete files, exclusively and atomically.
        # The temporary names disappear on exit, leaving ordinary single-link files.
        published = False
        try:
            os.link(staged, output)
            published = True
            os.link(staged_checksum, checksum)
        except FileExistsError:
            if published and output.exists() and os.path.samefile(staged, output):
                output.unlink()
            raise ValueError('Output appeared while packaging; refusing to replace it')
        except OSError:
            if published and output.exists() and os.path.samefile(staged, output):
                output.unlink()
            raise
    return {'package': str(output), 'sha256': sha256, 'checksum-file': str(checksum)}


def main():
    """Read untrusted Action inputs as data, without shell interpolation."""
    manifest = checked_path(os.environ.get('MPKG_MANIFEST', ''), 'manifest')
    payload = checked_path(os.environ.get('MPKG_PAYLOAD', ''), 'payload')
    output = checked_path(os.environ.get('MPKG_OUTPUT', 'dist/package.mpkg'), 'output')
    validate(manifest, payload, output)
    action = Path(__file__).resolve().parent
    scratch = os.environ.get('RUNNER_TEMP')
    with tempfile.TemporaryDirectory(prefix='mpkg-action-', dir=scratch) as folder:
        configured = os.environ.get('MPKG_TOOL', '')
        tool = checked_path(configured, 'mpack') if configured else bundled_tool(action, Path(folder))
        results = create(manifest, payload, output, tool)
    if os.environ.get('GITHUB_OUTPUT'):
        with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as stream:
            for key, value in results.items():
                stream.write(key + '=' + value + '\n')
    print('Created unsigned MPKG: ' + results['package'])
    print('SHA-256: ' + results['sha256'])


if __name__ == '__main__':
    main()
