#!/usr/bin/env python3
"""Fail closed before publication; report the digest returned by GHCR."""
import base64
import json
import os
import re
import sys
import urllib.error
import urllib.request

PACKAGE = 'guionai/flicknote-private-mcp'
ACCEPT = ', '.join(('application/vnd.oci.image.index.v1+json', 'application/vnd.oci.image.manifest.v1+json', 'application/vnd.docker.distribution.manifest.list.v2+json', 'application/vnd.docker.distribution.manifest.v2+json'))


def get(url, headers):
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=30) as response:
            return response.status, response.headers, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.headers, error.read()


def main(tag, mode):
    if len(tag) > 128 or not re.fullmatch(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', tag):
        raise ValueError('invalid tag')
    token = os.environ['GITHUB_TOKEN']
    actor = os.environ['GITHUB_ACTOR']
    headers = {'Authorization': 'Bearer ' + token, 'Accept': 'application/vnd.github+json', 'User-Agent': 'flicknote-image-publisher'}
    if mode == 'before':
        status, _, body = get('https://api.github.com/orgs/GuionAI/packages/container/flicknote-private-mcp', headers)
        if status == 200:
            package = json.loads(body)
            if (package.get('repository') or {}).get('full_name', '').lower() != 'guionai/flicknote-cli':
                raise ValueError('existing package linkage cannot be verified; inspect ownership')
        elif status != 404:
            raise ValueError(f'package lookup failed: HTTP {status}')
    basic = base64.b64encode((actor + ':' + token).encode()).decode()
    status, _, body = get('https://ghcr.io/token?service=ghcr.io&scope=repository:guionai/flicknote-private-mcp:pull,push', {'Authorization': 'Basic ' + basic})
    if status != 200:
        raise ValueError(f'GHCR token request failed: HTTP {status}')
    registry_token = json.loads(body)['token']
    status, headers, body = get(f'https://ghcr.io/v2/{PACKAGE}/manifests/{tag}', {'Authorization': 'Bearer ' + registry_token, 'Accept': ACCEPT})
    if mode == 'before':
        if status == 200:
            raise ValueError('version already exists: ' + str(headers.get('Docker-Content-Digest')) + '; inspect digest and provenance, do not overwrite')
        if status != 404 or not any(e.get('code') in ('MANIFEST_UNKNOWN', 'NAME_UNKNOWN') for e in json.loads(body).get('errors', [])):
            raise ValueError(f'cannot establish tag absence: HTTP {status}')
    elif mode == 'after':
        digest = headers.get('Docker-Content-Digest', '')
        if status != 200 or not re.fullmatch(r'sha256:[0-9a-f]{64}', digest):
            raise ValueError(f'cannot verify published manifest: HTTP {status}; inspect before retry')
        print(f'ghcr.io/{PACKAGE}@{digest}')
    else:
        raise ValueError('invalid mode')


if __name__ == '__main__':
    try:
        main(sys.argv[1], sys.argv[2])
    except (ValueError, KeyError, OSError) as error:
        sys.exit(f'registry check failed: {error}')
