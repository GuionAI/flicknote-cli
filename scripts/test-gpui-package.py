#!/usr/bin/env python3
"""Execute an owned fake GUI artifact and verify its manifest and launch contract."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('package', Path(__file__).with_name('package-gpui.py'))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


CERTIFICATE = b'owned fake certificate; never a real Keychain identity'
IDENTITY = hashlib.sha1(CERTIFICATE).hexdigest().upper()
REQUIREMENT = ('identifier "app.flicknote.gpui" and anchor apple generic and '
               'certificate leaf[subject.OU] = "FIXTURE123"')


class FakeSigner:
    def __init__(self, failure=None, metadata=None):
        self.failure = failure
        self.metadata = metadata
        self.calls = []

    def __call__(self, arguments):
        self.calls.append(arguments)
        target = Path(arguments[-1])
        if self.failure and self.failure in arguments:
            raise ValueError('owned signer failure')
        if '--sign' in arguments:
            self.assert_complete_bundle(target)
            executable = target / 'Contents/MacOS/flicknote-gpui'
            executable.write_bytes(executable.read_bytes() + b'\n# fake signed bytes\n')
            resources = target / 'Contents/_CodeSignature'
            resources.mkdir()
            (resources / 'CodeResources').write_bytes(b'fake resource seal')
        if '--display' in arguments:
            prefix = Path(next(value.split('=', 1)[1] for value in arguments
                               if isinstance(value, str) and value.startswith('--extract-certificates=')))
            prefix.with_name(prefix.name + '0').write_bytes(CERTIFICATE)
            return self.metadata or (f'Identifier=app.flicknote.gpui\n'
                                     f'Authority=Apple Development: Fixture\n'
                                     f'TeamIdentifier=FIXTURE123\n'
                                     f'designated => {REQUIREMENT}\n')
        return ''

    @staticmethod
    def assert_complete_bundle(app):
        assert (app / 'Contents/Info.plist').is_file()
        assert not (app.parent / 'SOURCE.json').exists()
        assert not (app.parent / 'SHA256SUMS').exists()


class NormalPackage(unittest.TestCase):
    def test_direct_gui_environment_source_and_hashes(self):
        with tempfile.TemporaryDirectory() as root:
            owned = Path(root).resolve()
            binaries = owned / 'bin'
            binaries.mkdir()
            fake = binaries / 'flicknote-gpui'
            fake.write_text(f'#!{sys.executable}\nimport json, os, sys\n'
                            "print(json.dumps({'args':sys.argv[1:],'environment':{k:v for k,v in os.environ.items() if k.startswith('FLICKNOTE_')}}))\n")
            fake.chmod(0o755)
            # A test-owned repository permits the same clean-source check during routine tests.
            package.ROOT = owned
            subprocess.run(['git', 'init', '-q', str(owned)], check=True)
            subprocess.run(['git', '-C', str(owned), '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.test',
                            'commit', '-q', '--allow-empty', '-m', 'chore(ci): fixture'], check=True)
            (owned / '.gitignore').write_text('bin/\n.scratch/\n')
            subprocess.run(['git', '-C', str(owned), 'add', '.gitignore'], check=True)
            subprocess.run(['git', '-C', str(owned), '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.test',
                            'commit', '-q', '-m', 'chore(ci): ignore artifacts'], check=True)
            output = owned / '.scratch' / 'normal-v1'
            signer = FakeSigner()
            manifest = package.package(output, binaries, IDENTITY, signer)
            stored = json.loads((output / 'SOURCE.json').read_text())
            self.assertEqual(stored, manifest)
            app = output / 'FlickNote.app'
            metadata = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
            self.assertEqual(metadata['CFBundleIdentifier'], manifest['bundle_id'])
            self.assertEqual(manifest['mode'], 'normal')
            self.assertEqual(manifest['unsigned_input_sha256'], package.digest(fake))
            signed_path = 'FlickNote.app/Contents/MacOS/flicknote-gpui'
            self.assertNotEqual(manifest['unsigned_input_sha256'], manifest['sha256'][signed_path])
            self.assertIn('FlickNote.app/Contents/_CodeSignature/CodeResources', manifest['sha256'])
            self.assertEqual(manifest['signing'], {
                'identity_sha1': IDENTITY, 'authorities': ['Apple Development: Fixture'],
                'team_identifier': 'FIXTURE123', 'designated_requirement': REQUIREMENT,
                'strict_verified': True})
            self.assertIn('--sign', signer.calls[0])
            self.assertEqual(len(signer.calls), 6)
            sums = (output / 'SHA256SUMS').read_text().splitlines()
            self.assertEqual(sums, [f'{value}  {path}' for path, value in manifest['sha256'].items()])
            for expression, field in [('HEAD', 'source_commit'), ('HEAD^{tree}', 'source_tree')]:
                self.assertEqual(manifest[field], subprocess.check_output(['git', 'rev-parse', expression], cwd=owned, text=True).strip())
            for path, hash_value in manifest['sha256'].items():
                self.assertEqual(package.digest(output / path), hash_value)
            for overrides in [{}, {'FLICKNOTE_ENV':'prod', 'FLICKNOTE_SUPABASE_URL':'http://127.0.0.1:1',
                                  'FLICKNOTE_MCP_PORT':'0', 'FLICKNOTE_WEB_URL':'https://example.test'}]:
                launched = json.loads(subprocess.check_output([app / 'Contents/MacOS' / metadata['CFBundleExecutable']],
                                    env={'PATH':os.defpath, **overrides}, text=True))
                self.assertEqual(launched['args'], [])
                self.assertEqual(launched['environment'], overrides)
            self.assertEqual({p.name for p in (app / 'Contents/MacOS').iterdir()}, {'flicknote-gpui'})
            with self.assertRaises(ValueError):
                package.package(output, binaries, IDENTITY, signer)
            with self.assertRaises(ValueError):
                package.package(owned / 'outside-scratch', binaries, IDENTITY, signer)
            (owned / 'dirty').write_text('uncommitted')
            with self.assertRaises(ValueError):
                package.package(owned / '.scratch' / 'normal-v2', binaries, IDENTITY, signer)

    def test_missing_or_adhoc_identity_never_invokes_signer(self):
        for identity in (None, '', '-', 'Apple Development', 'ambiguous name'):
            signer = FakeSigner()
            with self.assertRaisesRegex(ValueError, 'explicit Apple Development'):
                package.package(Path('unused'), Path('unused'), identity, signer)
            self.assertEqual(signer.calls, [])
        result = subprocess.run([sys.executable, str(Path(__file__).with_name('package-gpui.py')),
                                 '--output', 'unused'], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('--signing-identity', result.stderr)

    def test_failed_signing_or_verification_has_no_success_manifest(self):
        with tempfile.TemporaryDirectory() as root:
            owned = Path(root).resolve()
            package.ROOT = owned
            binaries = owned / 'bin'
            binaries.mkdir()
            (binaries / 'flicknote-gpui').write_bytes(b'owned unsigned fixture')
            subprocess.run(['git', 'init', '-q', str(owned)], check=True)
            (owned / '.gitignore').write_text('bin/\n.scratch/\n')
            subprocess.run(['git', '-C', str(owned), 'add', '.gitignore'], check=True)
            subprocess.run(['git', '-C', str(owned), '-c', 'user.name=Fixture',
                            '-c', 'user.email=fixture@example.test', 'commit', '-q',
                            '-m', 'chore(ci): fixture'], check=True)
            cases = [FakeSigner(failure='--sign'), FakeSigner(failure='--verify'),
                     FakeSigner(metadata='Identifier=wrong\n'),
                     FakeSigner(metadata=f'Identifier=app.flicknote.gpui\n'
                                f'Authority=Apple Development: Fixture\n'
                                f'TeamIdentifier=FIXTURE123\ndesignated => cdhash abc\n')]
            for index, signer in enumerate(cases):
                output = owned / '.scratch' / f'failed-{index}'
                with self.assertRaises(ValueError):
                    package.package(output, binaries, IDENTITY, signer)
                self.assertFalse((output / 'SOURCE.json').exists())
                self.assertFalse((output / 'SHA256SUMS').exists())
            with self.assertRaisesRegex(ValueError, 'unexpected certificate'):
                package.package(owned / '.scratch' / 'wrong-certificate', binaries,
                                '0' * 40, FakeSigner())



if __name__ == '__main__':
    unittest.main()
