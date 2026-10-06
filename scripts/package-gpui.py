#!/usr/bin/env python3
"""Package the normal macOS GUI from clean source; never launch or install it."""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def package(output, binaries):
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
    shutil.copy2(binaries / 'flicknote-gpui', macos / 'flicknote-gpui')
    bundle_id = 'app.flicknote.gpui'
    with (app / 'Contents/Info.plist').open('wb') as file:
        plistlib.dump({'CFBundleExecutable': 'flicknote-gpui', 'CFBundleIdentifier': bundle_id,
                      'CFBundleName': 'FlickNote', 'CFBundlePackageType': 'APPL',
                      'CFBundleVersion': commit[:12], 'NSHighResolutionCapable': True}, file)
    manifest = {'spec': 3374, 'mode': 'normal', 'source_commit': commit, 'source_tree': tree,
                'bundle_id': bundle_id, 'configuration': 'Config::load',
                'sha256': {str(p.relative_to(output)): digest(p)
                           for p in sorted(app.rglob('*')) if p.is_file()}}
    (output / 'SOURCE.json').write_text(json.dumps(manifest, indent=2) + '\n')
    (output / 'SHA256SUMS').write_text(''.join(f'{h}  {p}\n' for p, h in manifest['sha256'].items()))
    (output / 'RUN.md').write_text(
        f'# Normal FlickNote GUI #3374\n\nSource `{commit}`; tree `{tree}`.\n'
        'Verify `shasum -a 256 -c SHA256SUMS` from this directory.\n'
        'The app directly runs its GUI binary without a CLI dependency or endpoint/profile flags.\n'
        'It uses the existing normal config/session/data, daemon.sock and default MCP37789.\n'
        'Saved config and FLICKNOTE_* overrides retain shared daemon semantics; missing endpoint fields\n'
        'use FLICKNOTE_ENV, default dev. FLICKNOTE_MCP_PORT remains available to operators.\n'
        'After acceptance, quit the old GUI trial, then use the installed CLI to run\n'
        '`flicknote daemon uninstall` and `flicknote daemon stop` before manual launch with\n'
        '`open "FlickNote.app"`. Check `flicknote daemon status` and the original MCP client.\n'
        'No automatic takeover occurs. Close retains the host; Command-Q releases it.\n'
        'Boot-at-login registration, signing and updater are deferred.\n'
        'Packaging performs no launch/install/service/login/cloud operations. Preserve old apps/profiles.\n'
        'Normal native launch, input/pixels, live cutover and cloud verification remain unperformed.\n'
        'See docs/normal-gui-host.md in the matching source for cutover and rollback instructions.\n')
    return manifest


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binaries', type=Path, default=ROOT / 'target/debug')
    args = parser.parse_args()
    print(json.dumps(package(args.output, args.binaries), indent=2))
