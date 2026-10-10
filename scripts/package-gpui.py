#!/usr/bin/env python3
"""Package the normal macOS GUI from clean source; never launch or install it."""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def codesign(arguments):
    """Bound authentication waits; the operator handles any Keychain prompt."""
    try:
        result = subprocess.run(['/usr/bin/codesign', *map(str, arguments)],
                                capture_output=True, text=True, check=True, timeout=60)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        raise ValueError(f'codesign failed; check identity availability and handle authentication '
                         f'manually: {error.stderr or error}') from error
    return result.stdout + result.stderr


def sign_bundle(app, identity, runner):
    # Preserve only existing entitlements, never the old cdhash requirement or flags.
    runner(['--force', '--sign', identity, '--preserve-metadata=entitlements', app])
    runner(['--verify', '--deep', '--strict', '--verbose=2', app])
    signatures = []
    for target in (app, app / 'Contents/MacOS/flicknote-gpui'):
        runner(['--verify', '--strict', '--verbose=2', target])
        with tempfile.TemporaryDirectory(prefix='flicknote-signature-') as temporary:
            prefix = Path(temporary) / 'certificate-'
            details = runner(['--display', '--verbose=4', '-r-',
                              f'--extract-certificates={prefix}', target])
            certificate = prefix.with_name(prefix.name + '0')
            fingerprint = hashlib.sha1(certificate.read_bytes()).hexdigest().upper()
        fields = dict(line.split('=', 1) for line in details.splitlines() if '=' in line)
        requirement = fields.get('designated ', '').removeprefix('> ').strip()
        authorities = [line.removeprefix('Authority=') for line in details.splitlines()
                       if line.startswith('Authority=')]
        team = fields.get('TeamIdentifier', '')
        if (fingerprint != identity or fields.get('Identifier') != 'app.flicknote.gpui'
                or not authorities or not authorities[0].startswith('Apple Development: ')
                or not re.fullmatch(r'[A-Z0-9]{10}', team)
                or 'certificate' not in requirement or 'anchor apple' not in requirement
                or 'identifier "app.flicknote.gpui"' not in requirement
                or 'cdhash' in requirement):
            raise ValueError(f'unexpected certificate signing metadata for {target}: {details}')
        signatures.append({'identity_sha1': fingerprint, 'authorities': authorities,
                           'team_identifier': team, 'designated_requirement': requirement})
    if signatures[0] != signatures[1]:
        raise ValueError('bundle and executable signing identities differ')
    return {**signatures[0], 'strict_verified': True}


def package(output, binaries, identity, signer=codesign):
    if not re.fullmatch(r'[A-Fa-f0-9]{40}', identity or ''):
        raise ValueError('an explicit Apple Development certificate SHA1 is required')
    identity = identity.upper()
    output = output.resolve()
    if not output.is_relative_to(ROOT / '.scratch') or output.exists():
        raise ValueError('output must be a NEW versioned directory inside .scratch')
    if subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True).strip():
        raise ValueError('commit the source before packaging')
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    tree = subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=ROOT, text=True).strip()
    app = output / 'FlickNote.app'
    macos = app / 'Contents/MacOS'
    macos.mkdir(parents=True)
    unsigned_sha256 = digest(binaries / 'flicknote-gpui')
    shutil.copy2(binaries / 'flicknote-gpui', macos / 'flicknote-gpui')
    bundle_id = 'app.flicknote.gpui'
    with (app / 'Contents/Info.plist').open('wb') as file:
        plistlib.dump({'CFBundleExecutable': 'flicknote-gpui', 'CFBundleIdentifier': bundle_id,
                      'CFBundleName': 'FlickNote', 'CFBundlePackageType': 'APPL',
                      'CFBundleVersion': commit[:12], 'NSHighResolutionCapable': True}, file)
    signing = sign_bundle(app, identity, signer)
    manifest = {'mode': 'normal', 'source_commit': commit, 'source_tree': tree,
                'bundle_id': bundle_id, 'configuration': 'Config::load',
                'unsigned_input_sha256': unsigned_sha256, 'signing': signing,
                'sha256': {str(p.relative_to(output)): digest(p)
                           for p in sorted(app.rglob('*')) if p.is_file()}}
    (output / 'SOURCE.json').write_text(json.dumps(manifest, indent=2) + '\n')
    (output / 'SHA256SUMS').write_text(''.join(f'{h}  {p}\n' for p, h in manifest['sha256'].items()))
    (output / 'RUN.md').write_text(
        f'# Normal FlickNote GUI\n\nSource `{commit}`; tree `{tree}`.\n'
        'Verify `shasum -a 256 -c SHA256SUMS` from this directory.\n'
        'The app directly runs its GUI binary without a CLI dependency or endpoint/profile flags.\n'
        'It uses the existing normal config/session/data, daemon.sock and default MCP37789.\n'
        'Saved config and FLICKNOTE_* overrides retain shared daemon semantics; missing endpoint fields\n'
        'use FLICKNOTE_ENV, default dev. FLICKNOTE_MCP_PORT remains available to operators.\n'
        'Keep this artifact unlaunched until separate user direction for manual cutover.\n'
        'After acceptance and explicit launch direction, quit the old GUI trial, then use the installed CLI to run\n'
        '`flicknote daemon uninstall` and `flicknote daemon stop` before manual launch with\n'
        '`open "FlickNote.app"`. Check `flicknote daemon status` and the original MCP client.\n'
        'No automatic takeover occurs. Close retains the host; Command-Q releases it.\n'
        'Local Apple Development signing is verified before final hashes; distribution, login items and updater are deferred.\n'
        'Packaging performs no launch/install/service/login/cloud operations. Preserve old apps/profiles.\n'
        'Normal native launch, input/pixels, live cutover and cloud verification remain unperformed.\n'
        'Only mine in the right header defaults off and persists per account across Home/Today and project-All.\n'
        'It excludes only JSON boolean created_by_ai:true before the bounded watch limit; direct-ID access stays unfiltered; Failed/Shared/Archive bypass Only mine.\n'
        'Storage failure retains session scope and offers Not saved / retry; no Swift preferences are imported.\n'
        'Project editors use description; note summaries remain independent.\n'
        'Automatic organization/Catch up and client Jev credential/provider readers are retired (#3634).\n'
        'Existing obsolete preferences/Keychain items are untouched; automatic project selection belongs to the backend.\n'
        'Orc installs accepted merged updates to /Applications/FlickNote.app with verified hashes and rollback retained.\n'
        'Installation never automatically quits/restarts a running app or clears drafts.\n'
        'See docs/project-description-migration.md for separately operated owner-scoped dev conversion after old writers stop.\n'
        'See docs/normal-gui-host.md in the matching source for cutover and rollback instructions.\n')
    return manifest


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--signing-identity', required=True, help='explicit Apple Development certificate SHA1')
    parser.add_argument('--binaries', type=Path, default=ROOT / 'target/debug')
    args = parser.parse_args()
    print(json.dumps(package(args.output, args.binaries, args.signing_identity), indent=2))
