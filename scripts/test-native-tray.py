#!/usr/bin/env python3
"""Run the tray ABI/resource contract on the macOS main thread in an owned fixture.

Ordinary libtest workers are not AppKit's main thread. This explicit probe builds
from the same source and locked dependency artifacts, without launching a normal
GUI bundle, changing services or reading accounts. It does not simulate physical
mouse delivery, menu tracking/pixels, VoiceOver navigation or App Nap efficacy.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import tomllib


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="NEW ignored scratch directory")
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("requires macOS")
    repo = Path(__file__).resolve().parent.parent
    output = args.output.resolve()
    output.relative_to(repo / ".scratch")
    output.mkdir(parents=True, exist_ok=False)
    build = subprocess.run(
        ["cargo", "test", "--locked", "-p", "flicknote-gpui", "--no-run", "--message-format=json"],
        cwd=repo, capture_output=True, text=True, check=False,
    )
    (output / "cargo.jsonl").write_text(build.stdout)
    (output / "cargo.log").write_text(build.stderr)
    build.check_returncode()
    names = {"objc2", "objc2_app_kit", "objc2_foundation", "tokio", "anyhow", "flicknote_sync", "gpui_kit"}
    libraries = {}
    for line in build.stdout.splitlines():
        event = json.loads(line)
        if event.get("reason") != "compiler-artifact":
            continue
        name = event["target"]["name"]
        for filename in event["filenames"]:
            if name in names and filename.endswith(".rlib"):
                # objc2 0.5 belongs to unrelated dependencies; use our locked family.
                if name.startswith("objc2") and "0.5.2" in event["package_id"]:
                    continue
                if name in {"objc2_app_kit", "objc2_foundation"} and "0.2.2" in event["package_id"]:
                    continue
                libraries[name] = filename
    assert libraries.keys() == names, libraries.keys()
    source = output / "main.rs"
    source.write_text(f'''#[path = {json.dumps(str(repo / 'flicknote-gpui/src/tray.rs'))}]
mod tray;
fn main() {{
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let root = std::path::PathBuf::from(std::env::args_os().nth(1).unwrap());
    let host = runtime.block_on(flicknote_sync::spike::SpikeHost::start(
        &root, 0, 1, std::time::Duration::ZERO)).unwrap();
    let application = gpui_kit::application();
    let activity = application.background_executor().prevent_app_nap("Owned native tray contract probe");
    tray::tests::native_contract();
    runtime.block_on(host.run_until(async {{ Ok(()) }})).unwrap();
    assert!(!root.join("data/flicknote/daemon.sock").exists());
    drop(activity);
    drop(application);
    println!("PASS: owned main-thread AppKit target/action, template, AX action and cleanup; physical input/pixels unverified");
}}
''')
    binary = output / "native-tray-contract"
    command = ["rustc", "--edition=2024", "--cfg", "test", str(source), "-o", str(binary),
               "-L", f"dependency={Path(libraries['objc2']).parent}"]
    for name, library in libraries.items():
        command.extend(["--extern", f"{name}={library}"])
    (output / "rustc-command.json").write_text(json.dumps(command, indent=2) + "\n")
    workspace = tomllib.loads((repo / "Cargo.toml").read_text())
    subprocess.run(command, cwd=repo, check=True,
                   env={**os.environ, "CARGO_PKG_VERSION": workspace["workspace"]["package"]["version"]})
    # The root is newly created by SpikeHost; never use the normal app as a fixture.
    # A short test-owned root keeps the Unix socket under macOS's path bound.
    with tempfile.TemporaryDirectory(prefix="fntray-", dir="/tmp") as fixture:
        (output / "fixture-path.txt").write_text(fixture + "\n")
        subprocess.run([str(binary), fixture], cwd=repo, check=True, timeout=30,
                       env={**os.environ, "RUST_BACKTRACE": "1"})


if __name__ == "__main__":
    main()
