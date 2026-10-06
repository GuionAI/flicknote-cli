#!/usr/bin/env python3
"""Execute an owned fake GUI artifact and verify its manifest and launch contract."""
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
            manifest = package.package(output, binaries)
            stored = json.loads((output / 'SOURCE.json').read_text())
            self.assertEqual(stored, manifest)
            app = output / 'FlickNote.app'
            metadata = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
            self.assertEqual(metadata['CFBundleIdentifier'], manifest['bundle_id'])
            self.assertEqual(manifest['mode'], 'normal')
            self.assertEqual(manifest['spec'], 3398)
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
                package.package(output, binaries)
            with self.assertRaises(ValueError):
                package.package(owned / 'outside-scratch', binaries)
            (owned / 'dirty').write_text('uncommitted')
            with self.assertRaises(ValueError):
                package.package(owned / '.scratch' / 'normal-v2', binaries)


if __name__ == '__main__':
    unittest.main()
