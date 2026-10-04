#!/usr/bin/env python3
"""Exercise the machine-consumed dev artifact with owned fake executables only."""
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
spec = importlib.util.spec_from_file_location("package", Path(__file__).with_name("package-gpui-dev.py"))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


@unittest.skipUnless(sys.platform == "darwin", "macOS dev artifact; Linux acceptance deferred")
class DevPackage(unittest.TestCase):
    def test_artifact_launch_contract_hashes_and_owned_socket(self):
        (package.ROOT / ".scratch").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=package.ROOT / ".scratch", prefix="pkg-test-") as output_root, tempfile.TemporaryDirectory(dir="/private/tmp", prefix="fn-") as root:
            owned = Path(root).resolve()
            binaries = owned / "bin"
            binaries.mkdir()
            fake = binaries / "flicknote-gpui"
            fake.write_text(
                f"#!{sys.executable}\nimport json, os, sys\n"
                "print(json.dumps({'args':sys.argv[1:],'environment':{k:v for k,v in os.environ.items() if k.startswith('FLICKNOTE_')}}))\n"
            )
            fake.chmod(0o755)
            (binaries / "flicknote").write_text("owned CLI companion\n")
            profile = owned / "fn-dev-profile"
            output = Path(output_root) / "dev-v1"
            package.package(output, profile, binaries)
            manifest = json.loads((output / "SOURCE.json").read_text())
            app = output / "FlickNote Dev.app"
            metadata = plistlib.loads((app / "Contents/Info.plist").read_bytes())
            self.assertEqual(metadata["CFBundleIdentifier"], manifest["bundle_id"])
            self.assertEqual(metadata["CFBundleExecutable"], "launch")
            self.assertEqual(manifest["environment"], "dev")
            self.assertEqual(manifest["spec"], 3296)
            self.assertEqual(manifest["profile"], str(profile))
            self.assertEqual(manifest["socket_bytes"], len(os.fsencode(profile / "data/flicknote/daemon.sock")))
            for path, digest in manifest["sha256"].items():
                self.assertEqual(package.digest(output / path), digest)
            self.assertEqual(manifest["source_commit"], subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=package.ROOT, text=True).strip())
            poisoned = {"PATH": os.defpath, "FLICKNOTE_ENV": "prod", "FLICKNOTE_WEB_URL": "https://wrong.invalid"}
            poisoned.update({key: "https://wrong.invalid" for key in package.ENDPOINTS})
            launched = json.loads(subprocess.check_output([app / "Contents/MacOS/launch"], env=poisoned, text=True))
            self.assertEqual(launched["args"], ["--profile", str(profile), "--mcp-port", "0"])
            self.assertEqual(launched["environment"]["FLICKNOTE_ENV"], "dev")
            self.assertNotIn("FLICKNOTE_WEB_URL", launched["environment"])
            self.assertEqual({k: launched["environment"][k] for k in package.ENDPOINTS}, manifest["public_endpoints"])
            self.assertFalse(profile.exists())
            with self.assertRaises(ValueError):
                package.package(output, profile, binaries)
            profile.mkdir()
            with self.assertRaises(ValueError):
                package.package(Path(output_root) / "dev-v2", profile, binaries)


if __name__ == "__main__":
    unittest.main()
