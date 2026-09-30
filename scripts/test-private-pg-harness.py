#!/usr/bin/env python3
"""Exercise migration failure and cleanup against test-owned source/container state.

Usage: python3 scripts/test-private-pg-harness.py /path/to/fb
The supplied checkout is read only; archive HEAD into a temporary repository.
"""
import io
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest

SCRIPT = Path(__file__).with_name("test-private-pg.sh").resolve()
SOURCE = Path(sys.argv.pop(1)).resolve()
ENGINE = os.environ.get("CONTAINER_TOOL", "podman")


class MigrationHarness(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="fn-pg-harness-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", "HEAD", "tanka"])
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(self.root, filter="data")
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        subprocess.run(["git", "-C", str(self.root), "add", "tanka"], check=True)
        subprocess.run(["git", "-C", str(self.root), "-c", "core.hooksPath=/dev/null",
                        "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                        "commit", "-qm", "fixture"], check=True)
        self.migrations = self.root / "tanka/charts/db-init/db/migrations"

    def environment(self):
        env = os.environ.copy()
        env.pop("FLICKNOTE_TEST_PG_IN_CONTAINER", None)
        env.pop("FLICKNOTE_TEST_FB_REVISION", None)
        env["FLICKNOTE_TEST_FB_CHECKOUT"] = str(self.root)
        return env

    def run_script(self, env):
        process = subprocess.Popen([str(SCRIPT), "--provision-only"], env=env,
                                   text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        try:
            output, _ = process.communicate(timeout=180)
        except subprocess.TimeoutExpired:
            process.terminate()
            try:
                output, _ = process.communicate(timeout=60)
            except subprocess.TimeoutExpired:
                process.kill()
                output, _ = process.communicate()
            name = re.search(r"^PG_CONTAINER=(.+)$", output, re.MULTILINE)
            if name:
                subprocess.run([ENGINE, "rm", "-f", name[1]], capture_output=True)
            raise
        return subprocess.CompletedProcess(process.args, process.returncode, output)

    def assert_cleaned(self, output):
        name = re.search(r"^PG_CONTAINER=(.+)$", output, re.MULTILINE)
        self.assertIsNotNone(name, output)
        probe = subprocess.run([ENGINE, "inspect", name[1]], capture_output=True)
        self.assertNotEqual(probe.returncode, 0, "test container leaked")

    def run_failure(self, expected):
        before = subprocess.check_output(["git", "-C", str(self.root), "diff"])
        result = self.run_script(self.environment())
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(expected, result.stdout)
        self.assertNotIn("FB_MIGRATIONS_APPLIED=", result.stdout)
        self.assert_cleaned(result.stdout)
        self.assertEqual(before, subprocess.check_output(["git", "-C", str(self.root), "diff"]))

    def test_revision_replay_ignores_dirty_migration_and_preserves_override(self):
        (self.migrations / "99999999999999_failure.sql").write_text(
            "-- migrate:up\nSELECT definitely_missing_fixture_function();\n-- migrate:down\n")
        revision = subprocess.check_output(["git", "-C", str(self.root), "rev-parse", "HEAD"], text=True).strip()
        before = subprocess.check_output(["git", "-C", str(self.root), "status", "--porcelain"])
        env = self.environment()
        env["FLICKNOTE_TEST_FB_REVISION"] = revision
        result = self.run_script(env)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn(f"FB_COMMIT={revision}", result.stdout)
        self.assertIn("FB_INPUT_DIRTY=true FB_DIRTY=false", result.stdout)
        self.assertIn(f"FB_MIGRATIONS_APPLIED={len(list(self.migrations.glob('*.sql'))) - 1}", result.stdout)
        self.assert_cleaned(result.stdout)
        self.assertEqual(before, subprocess.check_output(["git", "-C", str(self.root), "status", "--porcelain"]))

    def test_cancellation_removes_only_its_container(self):
        process = subprocess.Popen([str(SCRIPT), "--provision-only"], env=self.environment(),
                                   text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        output = ""

        def stop():
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=60)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                name = re.search(r"^PG_CONTAINER=(.+)$", output, re.MULTILINE)
                if name:
                    subprocess.run([ENGINE, "rm", "-f", name[1]], capture_output=True)

        self.addCleanup(stop)
        for line in process.stdout:
            output += line
            if line.startswith("PG_CONTAINER="):
                break
        name = re.search(r"^PG_CONTAINER=(.+)$", output, re.MULTILINE)
        self.assertIsNotNone(name, output)
        deadline = time.monotonic() + 60
        while subprocess.run([ENGINE, "inspect", name[1]], capture_output=True).returncode:
            self.assertLess(time.monotonic(), deadline, "container never started")
            time.sleep(0.1)
        process.terminate()
        tail, _ = process.communicate(timeout=60)
        self.assertEqual(process.returncode, 143, output + tail)
        self.assert_cleaned(output + tail)

    def test_failed_migration_propagates_and_cleans_up(self):
        (self.migrations / "99999999999999_failure.sql").write_text(
            "-- migrate:up\nSELECT definitely_missing_fixture_function();\n-- migrate:down\n")
        self.run_failure("definitely_missing_fixture_function")

    def test_missing_business_migration_fails_and_cleans_up(self):
        (self.migrations / "20251201000013_notes.sql").unlink()
        self.run_failure('relation "public.notes" does not exist')


if __name__ == "__main__":
    unittest.main()
