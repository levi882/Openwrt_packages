#!/usr/bin/env python3
"""Exercise the retained bootstrap's APK protocol without touching the host."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "packages/overlay-restore/files/etc/overlay-restore-bootstrap"
MOCK = r"""#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys

root = Path(os.environ["OVERLAY_RESTORE_BOOTSTRAP_ROOT"])
name = Path(sys.argv[0]).name
args = sys.argv[1:]
if name == "uci":
    key = args[-1].removeprefix("overlay_restore.main").lstrip(".")
    values = json.loads((root / "config.json").read_text())
    if key in values:
        print(values[key])
        sys.exit(0)
    sys.exit(1)
if name in ("logger", "sleep"):
    sys.exit(0)
with (root / "calls.jsonl").open("a") as handle:
    handle.write(json.dumps(args) + "\n")
if "--print-arch" in args:
    print(os.environ.get("MOCK_ARCH", "x86_64"))
    sys.exit(0)
while args and args[0] in ("--root", "--wait", "--timeout"):
    args = args[2:]
command = args.pop(0)
if command == "info":
    sys.exit(0 if (root / ("installed-" + args[-1])).exists() else 1)
updates = root / "updates"
count = int(updates.read_text()) if updates.exists() else 0
if command == "update":
    updates.write_text(str(count + 1))
    if os.environ.get("MOCK_LARGE_LOG"):
        print("x" * 100000)
    print("fetch signed package indices")
    # An unrelated feed failure does not prevent an otherwise valid plan.
    sys.exit(1 if os.environ.get("MOCK_UPDATE_ERROR") else 0)
if command not in ("add", "fix"):
    sys.exit(2)
assert not any(a in args for a in ("--allow-untrusted", "--force-broken-world", "--force-overwrite"))
if os.environ.get("MOCK_BAD_SIGNATURE"):
    print("ERROR: UNTRUSTED signature")
    sys.exit(1)
if count <= int(os.environ.get("MOCK_NOT_READY", "0")):
    print("ERROR: repository temporarily unavailable")
    sys.exit(1)
if "--simulate" in args:
    plan = os.environ.get("MOCK_PLAN")
    if plan:
        print(plan)
    elif command == "fix":
        print("(1/2) Reinstalling overlay-restore (0.2.0-r11)")
        print("(2/2) Reinstalling luci-app-overlay-restore (0.2.0-r17)")
    else:
        print("(1/3) Installing rpcd-mod-file (2026.01-r1)")
        print("(2/3) Installing overlay-restore (0.2.0-r11)")
        print("(3/3) Installing luci-app-overlay-restore (0.2.0-r17)")
    sys.exit(0)
if os.environ.get("MOCK_INSTALL_ERROR"):
    print("ERROR: unable to install")
    sys.exit(1)
for path in ("usr/sbin/overlay-restore", "usr/libexec/rpcd/overlay-restore",
             "www/luci-static/resources/view/system/overlay-restore.js"):
    file = root / path
    file.parent.mkdir(parents=True, exist_ok=True)
    file.write_text("#!/bin/sh\nexit 0\n")
    file.chmod(0o755)
for package in ("overlay-restore", "luci-app-overlay-restore"):
    (root / ("installed-" + package)).touch()
print("OK: recovery tools installed")
"""


class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="restore-bootstrap-")
        self.root = Path(self.temp.name)
        self.directory = self.root / "etc/overlay-restore-bootstrap"
        shutil.copytree(SOURCE, self.directory)
        (self.root / "etc/openwrt_release").write_text("DISTRIB_RELEASE='25.12.5'\n")
        (self.root / "etc/config").mkdir()
        self.set_config({"": "restore", "upgrade_bootstrap": "1"})
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name in ("apk", "uci", "logger", "sleep"):
            script = self.bin / name
            script.write_text(MOCK)
            script.chmod(0o755)
        self.env = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                        OVERLAY_RESTORE_BOOTSTRAP_ROOT=str(self.root),
                        OVERLAY_RESTORE_BOOTSTRAP_ATTEMPTS="3",
                        OVERLAY_RESTORE_BOOTSTRAP_DELAY="0")

    def tearDown(self):
        self.temp.cleanup()

    def set_config(self, values):
        (self.root / "config.json").write_text(json.dumps(values))

    def run_bootstrap(self, action="run", **settings):
        result = subprocess.run(["sh", str(self.directory / "run"), action],
                                env=dict(self.env, **settings), capture_output=True,
                                text=True, timeout=15)
        self.assertFalse((self.root / "tmp/overlay-restore-bootstrap.lock").exists())
        return result

    def calls(self):
        path = self.root / "calls.jsonl"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def transactions(self):
        return [args for args in self.calls() if ("add" in args or "fix" in args) and "--simulate" not in args]

    def install_files(self):
        for relative in ("usr/sbin/overlay-restore", "usr/libexec/rpcd/overlay-restore",
                         "www/luci-static/resources/view/system/overlay-restore.js"):
            file = self.root / relative
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text("#!/bin/sh\n")
            file.chmod(0o755)
        for package in ("overlay-restore", "luci-app-overlay-restore"):
            (self.root / ("installed-" + package)).touch()

    def test_disabled_or_unpreserved_configuration_never_installs(self):
        for values in ({"": "restore", "upgrade_bootstrap": "0"}, {}):
            with self.subTest(values=values):
                self.set_config(values)
                self.assertEqual(self.run_bootstrap().returncode, 0)
                self.assertEqual(self.calls(), [])

    def test_existing_tools_are_left_alone(self):
        self.install_files()
        self.assertEqual(self.run_bootstrap().returncode, 0)
        self.assertEqual(self.transactions(), [])
        self.assertFalse((self.root / "updates").exists())
        self.assertFalse((self.directory / "boot.log").exists())

    def test_old_profile_defaults_to_install_and_then_skips(self):
        self.set_config({"": "restore"})
        self.assertEqual(self.run_bootstrap().returncode, 0)
        self.assertEqual(len(self.transactions()), 1)
        self.assertIn("overlay-restore@myfeed>=0.2.0-r11", self.transactions()[0])
        self.assertIn("luci-app-overlay-restore@myfeed>=0.2.0-r17", self.transactions()[0])
        self.assertEqual(self.run_bootstrap("status").stdout.strip(), "installed")
        self.assertEqual(self.run_bootstrap().returncode, 0)
        self.assertEqual(len(self.transactions()), 1)
        self.assertEqual((self.root / "etc/apk/keys/overlay-restore-bootstrap.pem").read_bytes(),
                         (ROOT / "public-key.pem").read_bytes())

    def test_delayed_network_retries_and_succeeds(self):
        self.assertEqual(self.run_bootstrap(MOCK_NOT_READY="2").returncode, 0)
        self.assertEqual((self.root / "updates").read_text(), "3")
        self.assertEqual(len(self.transactions()), 1)

    def test_invalid_signature_stops_after_bounded_retries(self):
        self.assertEqual(self.run_bootstrap(MOCK_BAD_SIGNATURE="1").returncode, 1)
        self.assertEqual((self.root / "updates").read_text(), "3")
        self.assertEqual(self.transactions(), [])
        self.assertIn("UNTRUSTED signature", (self.directory / "boot.log").read_text())

    def test_incompatible_or_downgraded_packages_are_not_changed(self):
        for operation in ("Upgrading libc (1 -> 2)", "Purging luci-app-ota (1)",
                          "Downgrading overlay-restore (0.2.0-r11 -> 0.2.0-r10)"):
            with self.subTest(operation=operation):
                result = self.run_bootstrap(MOCK_PLAN="(1/1) " + operation)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(self.transactions(), [])
        self.assertIn("Refusing an unrelated package change", (self.directory / "boot.log").read_text())

    def test_missing_package_files_are_repaired(self):
        for package in ("overlay-restore", "luci-app-overlay-restore"):
            (self.root / ("installed-" + package)).touch()
        self.assertEqual(self.run_bootstrap().returncode, 0)
        self.assertEqual(len(self.transactions()), 1)
        self.assertIn("fix", self.transactions()[0])
        self.assertIn("--reinstall", self.transactions()[0])
        self.assertEqual(self.run_bootstrap("status").stdout.strip(), "installed")

    def test_unrelated_unavailable_feed_does_not_block_valid_install(self):
        self.assertEqual(self.run_bootstrap(MOCK_UPDATE_ERROR="1").returncode, 0)
        self.assertEqual(len(self.transactions()), 1)

    def test_failure_keeps_latest_log_and_existing_repository(self):
        repo = self.root / "etc/apk/repositories.d/00-myfeed.list"
        repo.parent.mkdir(parents=True)
        original = "# user feed\nhttps://example.test/myfeed/packages.adb\n"
        repo.write_text(original)
        self.assertEqual(self.run_bootstrap(MOCK_LARGE_LOG="1", MOCK_INSTALL_ERROR="1").returncode, 1)
        self.assertEqual((self.directory / "repositories.before").read_text(), original)
        self.assertTrue(repo.read_text().startswith(original))
        self.assertEqual(repo.read_text().count("@myfeed https://example.test/myfeed/packages.adb"), 1)
        self.assertLessEqual((self.directory / "boot.log").stat().st_size, 65536)
        self.assertIn("Automatic installation stopped", (self.directory / "boot.log").read_text())
        self.assertEqual(self.run_bootstrap().returncode, 0)
        self.assertEqual(repo.read_text().count("@myfeed https://example.test/myfeed/packages.adb"), 1)

    def test_unsupported_firmware_or_architecture_never_installs(self):
        self.assertEqual(self.run_bootstrap(MOCK_ARCH="aarch64").returncode, 1)
        (self.root / "etc/openwrt_release").write_text("DISTRIB_RELEASE='26.0.0'\n")
        self.assertEqual(self.run_bootstrap().returncode, 1)
        self.assertEqual(self.transactions(), [])
        self.assertFalse((self.root / "updates").exists())

    def test_setup_retains_a_separate_service_without_running_apk(self):
        rc_local = self.root / "etc/rc.local"
        rc_local.write_text("# user startup\nexit 0\n")
        self.assertEqual(self.run_bootstrap("setup").returncode, 0)
        init = self.root / "etc/init.d/overlay-restore-bootstrap"
        self.assertEqual(init.read_bytes(), (SOURCE / "init").read_bytes())
        self.assertTrue(os.access(init, os.X_OK))
        self.assertEqual((self.root / "etc/rc.d/S99overlay-restore-bootstrap").resolve(), init)
        self.assertEqual(rc_local.read_text(), "# user startup\nexit 0\n")
        self.assertEqual(self.calls(), [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
