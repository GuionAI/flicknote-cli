#!/usr/bin/env python3
"""Prepare a runtime context from one public, stable GitHub release."""
import hashlib
import io
import json
import re
import subprocess
import sys
import tarfile
import urllib.request
from pathlib import Path

REPO = 'GuionAI/flicknote-cli'
TARGET = 'x86_64-unknown-linux-musl'
ASSET = f'flicknote-cli-{TARGET}.tar.xz'
ROOT = f'flicknote-cli-{TARGET}/'


def request(url):
    with urllib.request.urlopen(urllib.request.Request(url, headers={'User-Agent': 'flicknote-image-publisher', 'Accept': 'application/vnd.github+json'}), timeout=120) as response:
        return response.read()


def prepare(tag, dest, fetch=request):
    if len(tag) > 128 or not re.fullmatch(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', tag):
        raise ValueError('expected stable release tag vX.Y.Z')
    release = json.loads(fetch(f'https://api.github.com/repos/{REPO}/releases/tags/{tag}'))
    if release['tag_name'] != tag or release['draft'] or release['prerelease'] or not release.get('published_at'):
        raise ValueError('release identity or publication state mismatch')
    base = f'https://github.com/{REPO}/releases/download/{tag}/'
    assets = release['assets']
    selected = {}
    for name in (ASSET, ASSET + '.sha256'):
        matches = [a for a in assets if a['name'] == name and a['state'] == 'uploaded']
        if len(matches) != 1 or matches[0]['browser_download_url'] != base + name:
            raise ValueError(f'invalid release asset: {name}')
        selected[name] = matches[0]
    checksum = fetch(base + ASSET + '.sha256')
    match = re.fullmatch(rb'([0-9a-f]{64}) [ *]' + re.escape(ASSET.encode()) + rb'\s*', checksum)
    if not match:
        raise ValueError('invalid checksum file')
    digest = match[1].decode()
    archive = fetch(base + ASSET)
    if hashlib.sha256(archive).hexdigest() != digest or selected[ASSET].get('digest') != 'sha256:' + digest:
        raise ValueError('archive checksum or GitHub asset digest mismatch')
    if selected[ASSET + '.sha256'].get('digest') != 'sha256:' + hashlib.sha256(checksum).hexdigest():
        raise ValueError('checksum asset digest mismatch')
    # Accept only regular files under the expected directory; never extract paths.
    files = {}
    with tarfile.open(fileobj=io.BytesIO(archive), mode='r:xz') as tar:
        for member in tar:
            name = member.name.removeprefix('./')
            if member.isdir() and name.rstrip('/') == ROOT.rstrip('/'):
                continue
            if not name.startswith(ROOT) or not member.isfile() or '/' in name[len(ROOT):] or name in files:
                raise ValueError(f'unexpected archive member: {member.name}')
            files[name] = tar.extractfile(member).read()
    if not files.get(ROOT + 'flicknote') or not files.get(ROOT + 'LICENSE'):
        raise ValueError('missing executable or LICENSE')
    dest.mkdir(parents=True, exist_ok=False)
    (dest / 'notices').mkdir()
    dest.chmod(0o755)
    (dest / 'notices').chmod(0o755)
    for name, content in files.items():
        filename = name[len(ROOT):]
        (dest / ('flicknote' if filename == 'flicknote' else 'notices/' + filename)).write_bytes(content)
    (dest / 'notices' / (ASSET + '.sha256')).write_bytes(checksum)
    for notice in (dest / 'notices').iterdir():
        notice.chmod(0o644)
    binary = dest / 'flicknote'
    binary.chmod(0o755)
    version = subprocess.check_output([str(binary.resolve()), '--version'], text=True).strip()
    if version != 'flicknote ' + tag[1:]:
        raise ValueError(f'executable version mismatch: {version!r}')
    return digest, base + ASSET


if __name__ == '__main__':
    try:
        digest, url = prepare(sys.argv[1], Path(sys.argv[2]))
        print(json.dumps({'sha256': digest, 'url': url}))
    except (ValueError, OSError, KeyError, subprocess.SubprocessError) as error:
        sys.exit(f'release preparation failed: {error}')
