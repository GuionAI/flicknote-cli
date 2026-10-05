#!/usr/bin/env python3
"""Package a source-bound dev trial without launching or creating its profile."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shlex
import shutil
import socket
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
ENDPOINTS = {
    "FLICKNOTE_SUPABASE_URL": "https://dev-auth.flicknote.app",
    "FLICKNOTE_POWERSYNC_URL": "https://dev-sync.flicknote.app",
    "FLICKNOTE_API_URL": "https://dev-api.flicknote.app/api/v1",
    "FLICKNOTE_GATEWAY_URL": "https://dev-gw.flicknote.app",
}
DEV_KEY = "sb_publishable_4VEs5DX9YlkHuViFbmRMQb_f_LPrdOR"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_socket_length(profile):
    length = len(os.fsencode(profile / "data/flicknote/daemon.sock"))
    # Actual macOS AF_UNIX bind, in a test-owned directory with the exact length.
    with tempfile.TemporaryDirectory(prefix="fn-bind-", dir="/tmp") as root:
        prefix = os.fsencode(root + "/")
        if length <= len(prefix):
            raise ValueError("profile socket path is too short for owned bind verification")
        owned = prefix + b"s" * (length - len(prefix))
        with socket.socket(socket.AF_UNIX) as listener:
            listener.bind(owned)
            listener.listen(1)
    return length


def package(output, profile, binaries, reuse_profile_from=None, spec=3326):
    if spec not in (3326, 3341):
        raise ValueError("dev packaging supports only #3326 and #3341")
    if not profile.is_absolute() or ".." in profile.parts:
        raise ValueError("profile must be a canonical absolute path")
    if reuse_profile_from is not None:
        reuse_profile_from = reuse_profile_from.resolve()
        if not reuse_profile_from.is_relative_to(ROOT / ".scratch"):
            raise ValueError("reuse requires an existing scratch DEV artifact manifest")
        previous = json.loads(reuse_profile_from.read_text())
        if (previous.get("spec") != 3326 or previous.get("environment") != "dev"
                or previous.get("profile") != str(profile)
                or previous.get("profile_absent_at_packaging") is not True
                or previous.get("public_endpoints") != ENDPOINTS):
            raise ValueError("reuse requires the matching original #3326 DEV profile manifest")
        # Do not stat, resolve, read or mutate the user's existing profile.
    elif profile.resolve() != profile or profile.exists():
        raise ValueError("profile must be a NEW absent canonical absolute path")
    # Short dev names stay within macOS socket limits.
    if not profile.is_relative_to(Path("/private/tmp")) or not profile.name.startswith("fn-dev-"):
        raise ValueError("profile must be a short /private/tmp/.../fn-dev-* path")
    output = output.resolve()
    if not output.is_relative_to(ROOT / ".scratch") or output.exists():
        raise ValueError("output must be a NEW versioned directory inside .scratch")
    socket_bytes = verify_socket_length(profile)
    status = subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True)
    if status.strip():
        raise ValueError("commit the source before packaging")
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    tree = subprocess.check_output(["git", "rev-parse", "HEAD^{tree}"], cwd=ROOT, text=True).strip()
    app = output / "FlickNote Dev.app"
    macos = app / "Contents/MacOS"
    macos.mkdir(parents=True)
    for name in ["flicknote-gpui", "flicknote"]:
        shutil.copy2(binaries / name, macos / name)
    environment = {"FLICKNOTE_ENV": "dev", **ENDPOINTS, "FLICKNOTE_SUPABASE_KEY": DEV_KEY}
    env = " ".join(shlex.quote(f"{k}={v}") for k, v in environment.items())
    launcher = macos / "launch"
    launcher.write_text(
        '#!/bin/sh\nset -eu\nTASK_BIN_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\n'
        f'exec env -u FLICKNOTE_WEB_URL {env} "$TASK_BIN_DIR/flicknote-gpui" '
        f'--profile {shlex.quote(str(profile))} --mcp-port 0\n'
    )
    launcher.chmod(0o755)
    bundle_id = f"app.flicknote.gpui.dev.{output.name}"
    with (app / "Contents/Info.plist").open("wb") as file:
        plistlib.dump({"CFBundleExecutable": "launch", "CFBundleIdentifier": bundle_id,
                      "CFBundleName": "FlickNote Dev", "CFBundlePackageType": "APPL",
                      "CFBundleVersion": commit[:12], "NSHighResolutionCapable": True}, file)
    manifest = {"spec": spec, "environment": "dev", "source_commit": commit,
                "source_tree": tree, "bundle_id": bundle_id, "profile": str(profile),
                "profile_absent_at_packaging": None if reuse_profile_from else True,
                "profile_reuse_from": str(reuse_profile_from) if reuse_profile_from else None,
                "socket_bytes": socket_bytes,
                "mcp_port": 0, "public_endpoints": ENDPOINTS,
                "sha256": {str(p.relative_to(output)): digest(p) for p in sorted(app.rglob("*")) if p.is_file()}}
    (output / "SOURCE.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (output / "SHA256SUMS").write_text("".join(f"{h}  {p}\n" for p, h in manifest["sha256"].items()))
    run = output / "run-dev.sh"
    run.write_text(f"#!/bin/sh\nset -eu\nopen -a {shlex.quote(str(app))}\n")
    run.chmod(0o755)
    (output / "RUN.md").write_text(
        f"# Dev trial #{manifest['spec']}\n\nSource `{commit}`; bundle `{bundle_id}`.\n"
        + (f"Reuses approved dev profile `{profile}` from `{reuse_profile_from}`.\n"
         "Quit the old app before launch; concurrent ownership is rejected.\n"
         if reuse_profile_from else f"New independent profile `{profile}` (absent at packaging).\n")
        + "Environment **dev**.\n" +
        f"Socket path {socket_bytes} bytes; an equal-length test-owned AF_UNIX bind passed.\n"
        "Local MCP allocates a loopback port; read the actual IPC/MCP endpoints at startup.\n"
        "The package includes a source-matched CLI companion in Contents/MacOS/flicknote.\n"
        "Verify SHA256SUMS from this directory before opt-in launch with `bash run-dev.sh`.\n"
        "Email login and cloud operations require your own opt-in; the Worker never launched this app.\n"
        "Use this profile only with dev. Preserve previous apps/profiles/services; never copy sessions.\n"
        "Capture stays global/unassigned. Home is current Today; projects show All active notes, capped at 10k.\n"
        "Persisted previews: <=512 raw UTF-8 bytes use folded content; longer uses title or Untitled note.\n"
        "Line edges are trimmed, blank lines omitted, internal spacing retained; pending uses content. Detail stays canonical.\n"
        "Cmd1 Home; Cmd2..9 first eight projects; OptionUp/Down empty/unmarked only, without wrap.\n"
        "Visible Kit first-sync bar weights notes to 90%; unknown totals are indeterminate; all active defaults complete then hide.\n"
        "Empty/unmarked composer gives bindings first refusal; draft/marked retains Kit IME routing.\n"
        "Native IME/cloud acceptance is unperformed; rendered tests/builds are separate evidence.\n"
    )
    return manifest


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--binaries", type=Path, default=ROOT / "target/debug")
    parser.add_argument("--reuse-profile-from", type=Path,
                        help="Explicit #3326/#3341 rebuild using the original #3326 DEV SOURCE.json; requires user authorization")
    parser.add_argument("--spec", type=int, choices=[3326, 3341], default=3326,
                        help="Current delivery spec recorded in the source manifest")
    args = parser.parse_args()
    print(json.dumps(package(args.output, args.profile, args.binaries, args.reuse_profile_from, args.spec), indent=2))
