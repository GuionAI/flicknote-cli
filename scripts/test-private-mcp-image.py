#!/usr/bin/env python3
"""Offline release and registry failure-path fixtures; no registry writes."""
import hashlib
import io
import json
from pathlib import Path
import runpy
import tarfile
import tempfile
import unittest
from unittest.mock import patch

prepare = runpy.run_path('scripts/private-mcp-image/prepare.py')
registry = runpy.run_path('scripts/private-mcp-image/registry.py')
ASSET, ROOT = prepare['ASSET'], prepare['ROOT']


def fixture(version='1.16.0', extra=None):
    data = io.BytesIO()
    with tarfile.open(fileobj=data, mode='w:xz') as tar:
        for name, content in [('flicknote', f'#!/bin/sh\necho flicknote {version}\n'.encode()), ('LICENSE', b'MIT')]+(extra or []):
            info = tarfile.TarInfo(ROOT + name)
            info.size = len(content)
            tar.addfile(info, io.BytesIO(content))
    archive = data.getvalue()
    digest = hashlib.sha256(archive).hexdigest()
    checksum = f'{digest} *{ASSET}\n'.encode()
    base = 'https://github.com/GuionAI/flicknote-cli/releases/download/v1.16.0/'
    assets = [{'name': name, 'state': 'uploaded', 'browser_download_url': base + name,
               'digest': 'sha256:' + hashlib.sha256(contents).hexdigest()}
              for name, contents in [(ASSET, archive), (ASSET + '.sha256', checksum)]]
    release = {'tag_name': 'v1.16.0', 'draft': False, 'prerelease': False,
               'published_at': '2026-01-01', 'assets': assets}
    return release, archive, checksum


class ReleaseFixture(unittest.TestCase):
    def run_fixture(self, tag='v1.16.0', release=None, archive=None, checksum=None, version='1.16.0', extra=None):
        good_release, good_archive, good_checksum = fixture(version, extra)
        release = good_release if release is None else release
        archive = good_archive if archive is None else archive
        checksum = good_checksum if checksum is None else checksum
        def fetch(url):
            return json.dumps(release).encode() if '/api.github.com/' in url else checksum if url.endswith('.sha256') else archive
        with tempfile.TemporaryDirectory() as tmp:
            return prepare['prepare'](tag, Path(tmp) / 'context', fetch)

    def test_valid_and_invalid_inputs(self):
        digest, _ = self.run_fixture()
        self.assertEqual(len(digest), 64)
        release, archive, checksum = fixture()
        for change in ({'draft': True}, {'prerelease': True}, {'tag_name': 'v1.16.1'}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.run_fixture(release={**release, **change})
        for kwargs in ({'tag': 'v1.16.0;false'}, {'tag': 'v' + '1' * 128 + '.0.0'}, {'archive': archive + b'corrupt'},
                       {'checksum': checksum.replace(ASSET.encode(), b'other')},
                       {'version': '1.16.1'}, {'extra': [('../escape', b'x')]}):
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                self.run_fixture(**kwargs)


class RegistryFixture(unittest.TestCase):
    def run_registry(self, package, manifest, mode='before'):
        def fake_get(url, headers):
            if 'api.github.com' in url:
                return package
            if '/token?' in url:
                return 200, {}, b'{"token":"fixture"}'
            return manifest
        with patch.dict('os.environ', {'GITHUB_TOKEN': 'fixture', 'GITHUB_ACTOR': 'fixture'}), patch.dict(registry['main'].__globals__, {'get': fake_get}):
            registry['main']('v1.16.0', mode)

    def test_absence_conflict_and_unknown(self):
        missing = (404, {}, b'{"errors":[{"code":"MANIFEST_UNKNOWN"}]}')
        self.run_registry((404, {}, b''), missing)
        self.run_registry((200, {}, b'{"repository":{"full_name":"GuionAI/flicknote-cli"}}'), missing)
        for package, manifest in [((404, {}, b''), (200, {'Docker-Content-Digest': 'sha256:abc'}, b'')),
                                  ((404, {}, b''), (403, {}, b'')),
                                  ((200, {}, b'{"repository":{"full_name":"other/repo"}}'), missing),
                                  ((200, {}, b'{}'), missing),
                                  ((401, {}, b''), missing),
                                  ((404, {}, b''), (404, {}, b'{\"errors\":[]}'))]:
            with self.subTest(package=package, manifest=manifest), self.assertRaises(ValueError):
                self.run_registry(package, manifest)

    def test_published_digest_is_required(self):
        digest = 'sha256:' + 'a' * 64
        self.run_registry((404, {}, b''), (200, {'Docker-Content-Digest': digest}, b''), mode='after')
        with self.assertRaises(ValueError):
            self.run_registry((404, {}, b''), (200, {}, b''), mode='after')


if __name__ == '__main__':
    unittest.main()
