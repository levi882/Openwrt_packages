#!/usr/bin/env python3
"""Exercise the retained bootstrap's APK protocol without touching the host."""

import json
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(os.environ.get("OVERLAY_RESTORE_TEST_ROOT", Path(__file__).resolve().parents[1]))
SOURCE = Path(os.environ.get("OVERLAY_RESTORE_BOOTSTRAP_SOURCE", ROOT / "packages/overlay-restore/files/etc/overlay-restore-bootstrap"))
HOOK = Path(os.environ.get("OVERLAY_RESTORE_UPGRADE_HOOK", ROOT / "packages/overlay-restore/files/lib/upgrade/zz-overlay-restore.sh"))
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
if name == "jsonfilter":
    print(json.loads(Path(args[1]).read_text())["status"])
    sys.exit(0)
if name in ("logger", "sleep"):
    if name == "sleep" and args == ["300"] and os.environ.get("MOCK_WATCH_DISABLE"):
        values = json.loads((root / "config.json").read_text())
        values["upgrade_bootstrap"] = "0"
        (root / "config.json").write_text(json.dumps(values))
    sys.exit(0)
with (root / "calls.jsonl").open("a") as handle:
    handle.write(json.dumps(args) + "\n")
if "--print-arch" in args:
    print(os.environ.get("MOCK_ARCH", "x86_64"))
    sys.exit(0)
offline = "--no-network" in args
while args and args[0] in ("--root", "--wait", "--timeout", "--cache-dir", "--cache-max-age", "--repositories-file", "--no-network"):
    if args[0] == "--no-network":
        args = args[1:]
        continue
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
    sys.exit(1 if os.environ.get("MOCK_UPDATE_ERROR") or
             (not offline and os.environ.get("MOCK_ONLINE_UPDATE_ERROR")) else 0)
if command not in ("add", "fix"):
    sys.exit(2)
if os.environ.get("MOCK_FIRMWARE_PINS"):
    world = (root / "etc/apk/world").read_text().splitlines()
    assert "kernel=6.18.55-r1" in world, world
    assert "luci-base=26.100" in world, world
    assert "luci-mod-status=26.099" in world, world
if os.environ.get("MOCK_LOCAL_ONLY") and not offline:
    print("ERROR: online transaction forbidden")
    sys.exit(1)
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
    elif "--upgrade" in args and os.environ.get("MOCK_CURRENT"):
        print("OK: recovery tools already current")
    elif "--upgrade" in args and command == "add":
        print("(1/2) Upgrading overlay-restore (0.2.0-r13 -> 0.2.0-r14)")
        print("(2/2) Upgrading luci-app-overlay-restore (0.2.0-r20 -> 0.2.0-r21)")
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
if offline and os.environ.get("MOCK_OLD_BOOTSTRAP"):
    directory = root / "etc/overlay-restore-bootstrap"
    (directory / "run").write_text("#!/bin/sh\nexit 0\n")
    (directory / "init").write_text("#!/bin/sh\n# legacy service\n")
    (root / "etc/init.d/overlay-restore-bootstrap").write_bytes((directory / "init").read_bytes())
print("OK: recovery tools installed")
"""


class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="restore-bootstrap-")
        self.root = Path(self.temp.name)
        self.directory = self.root / "etc/overlay-restore-bootstrap"
        shutil.copytree(SOURCE, self.directory)
        (self.root / "etc/openwrt_release").write_text("DISTRIB_RELEASE='25.12.5'\n")
        (self.directory / "firmware.checked").write_text(self.fingerprint() + "\n")
        boot = self.root / "proc/sys/kernel/random/boot_id"
        boot.parent.mkdir(parents=True)
        boot.write_text("11111111-1111-4111-8111-111111111111\n")
        (self.root / "etc/config").mkdir()
        self.set_config({"": "restore", "upgrade_bootstrap": "1"})
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name in ("apk", "uci", "jsonfilter", "logger", "sleep"):
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

    def fingerprint(self):
        file = self.root / "rom/etc/openwrt_release"
        if not file.exists():
            file = self.root / "etc/openwrt_release"
        return hashlib.sha256(file.read_bytes()).hexdigest()

    def firmware_changed(self):
        (self.directory / "firmware.checked").write_text("0" * 64 + "\n")

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
        self.assertIn("overlay-restore@myfeed>=0.2.0-r17", self.transactions()[0])
        self.assertIn("luci-app-overlay-restore@myfeed>=0.2.0-r22", self.transactions()[0])
        self.assertEqual(self.run_bootstrap("status").stdout.strip(), "installed")
        self.assertEqual(self.run_bootstrap().returncode, 0)
        self.assertEqual(len(self.transactions()), 1)
        self.assertEqual((self.root / "etc/apk/keys/overlay-restore-bootstrap.pem").read_bytes(),
                         (ROOT / "public-key.pem").read_bytes())

    def test_delayed_network_retries_and_succeeds(self):
        self.assertEqual(self.run_bootstrap(MOCK_NOT_READY="2").returncode, 0)
        self.assertEqual((self.root / "updates").read_text(), "3")
        self.assertEqual(len(self.transactions()), 1)

    def test_firmware_versions_are_pinned_and_world_is_restored(self):
        database = self.root / "lib/apk/db/installed"
        database.parent.mkdir(parents=True)
        database.write_text("P:kernel\nV:6.18.55-r1\n\nP:luci-base\nV:26.100\n\nP:luci-mod-status\nV:26.099\n\n")
        world = self.root / "etc/apk/world"
        world.parent.mkdir(parents=True)
        world.write_text("kernel\nluci-base@custom\ncurl\n")
        result = self.run_bootstrap(MOCK_FIRMWARE_PINS="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(set(world.read_text().splitlines()), {"kernel", "luci-base@custom", "curl"})
        self.assertFalse((self.directory / "firmware-world.before").exists())
        self.assertFalse((self.directory / "firmware-pins").exists())

    def test_firmware_constraints_are_restored_when_plan_is_rejected(self):
        database = self.root / "lib/apk/db/installed"
        database.parent.mkdir(parents=True)
        database.write_text("P:kernel\nV:6.18.55-r1\n\nP:luci-base\nV:26.100\n\nP:luci-mod-status\nV:26.099\n\n")
        world = self.root / "etc/apk/world"
        world.parent.mkdir(parents=True)
        world.write_text("kernel\nluci-base@custom\n")
        result = self.run_bootstrap(MOCK_FIRMWARE_PINS="1", MOCK_PLAN="(1/1) Upgrading luci-base (26.100 -> 26.200)")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.transactions())
        self.assertEqual(set(world.read_text().splitlines()), {"kernel", "luci-base@custom"})

    def prepare_local_fixture(self):
        directory = self.root / "mnt/disk/restore-feed"
        snapshot = "a" * 32
        store = directory / "snapshots" / snapshot
        (store / "cache").mkdir(parents=True)
        (directory / "current").write_text(snapshot)
        repo = store / "repositories.list"
        repo.write_text("https://example.org/packages.adb\n@myfeed https://example.org/packages.adb\n")
        package = store / "cache/test.apk"
        package.write_bytes(b"fixture package")
        (store / "checksums.sha256").write_text(
            hashlib.sha256(repo.read_bytes()).hexdigest() + "  repositories.list\n" +
            hashlib.sha256(package.read_bytes()).hexdigest() + "  cache/test.apk\n")
        self.set_config({"": "restore", "upgrade_bootstrap": "1", "local_feed_dir": "/mnt/disk/restore-feed"})
        return store

    def test_upgrade_bootstrap_uses_only_the_prepared_local_feed(self):
        self.prepare_local_fixture()
        self.assertEqual(self.run_bootstrap(MOCK_LOCAL_ONLY="1").returncode, 0)
        self.assertEqual(len(self.transactions()), 1)
        self.assertIn("--no-network", self.transactions()[0])
        self.assertIn("--repositories-file", self.transactions()[0])
        repository = self.root / "etc/apk/repositories.d/00-myfeed.list"
        self.assertIn("https://openwrt-packages.pages.dev/", repository.read_text())
        self.assertNotIn("/mnt/disk", repository.read_text())

    def test_corrupt_local_feed_never_falls_back_to_online_install(self):
        store = self.prepare_local_fixture()
        (store / "cache/test.apk").write_bytes(b"tampered")
        self.assertEqual(self.run_bootstrap().returncode, 1)
        self.assertEqual(self.transactions(), [])
        self.assertIn("local feed is incomplete", (self.directory / "boot.log").read_text())

    def test_online_selection_ignores_a_retained_local_directory(self):
        self.set_config({"": "restore", "restore_source": "online", "local_feed_dir": "/mnt/missing/restore-feed"})
        self.assertEqual(self.run_bootstrap().returncode, 0)
        self.assertEqual(len(self.transactions()), 1)
        self.assertNotIn("--no-network", self.transactions()[0])

    def test_explicit_local_selection_requires_a_directory(self):
        self.set_config({"": "restore", "restore_source": "local"})
        self.assertEqual(self.run_bootstrap().returncode, 1)
        self.assertEqual(self.transactions(), [])

    def test_explicit_local_selection_keeps_the_bootstrap_offline(self):
        self.prepare_local_fixture()
        self.set_config({"": "restore", "restore_source": "local", "local_feed_dir": "/mnt/disk/restore-feed"})
        self.assertEqual(self.run_bootstrap(MOCK_LOCAL_ONLY="1").returncode, 0)
        self.assertIn("--no-network", self.transactions()[0])

    def test_missing_local_disk_keeps_the_bootstrap_retryable(self):
        self.set_config({"": "restore", "local_feed_dir": "/mnt/missing/restore-feed"})
        self.assertEqual(self.run_bootstrap().returncode, 1)
        self.assertEqual(self.transactions(), [])
        self.assertIn("Waiting for the prepared local feed disk", (self.directory / "boot.log").read_text())

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
        self.assertEqual((self.directory / "runner").read_bytes(), (SOURCE / "run").read_bytes())
        self.assertEqual((self.directory / "runner-init").read_bytes(), (SOURCE / "init").read_bytes())

    def test_retained_complete_tools_are_updated_after_upgrade(self):
        self.install_files()
        self.firmware_changed()
        self.assertEqual(self.run_bootstrap().returncode, 0)
        self.assertEqual(len(self.transactions()), 1)
        self.assertIn("--upgrade", self.transactions()[0])
        self.assertIn("--repositories-file", self.transactions()[0])
        self.assertIn("--cache-max-age", self.transactions()[0])
        self.assertEqual((self.directory / "firmware.checked").read_text().strip(), self.fingerprint())
        before = len(self.calls())
        self.assertEqual(self.run_bootstrap().returncode, 0)
        self.assertEqual(len(self.transactions()), 1)
        self.assertFalse(any("update" in c for c in self.calls()[before:]))

    def test_package_setup_replaces_old_protected_bootstrap_resources(self):
        packaged = self.root / "usr/libexec/overlay-restore-bootstrap"
        packaged.parent.mkdir(parents=True)
        packaged.write_bytes((SOURCE / "run").read_bytes())
        resources = self.root / "usr/share/overlay-restore-bootstrap"
        resources.mkdir(parents=True)
        for name in ("init", "myfeed.pem"):
            (resources / name).write_bytes((SOURCE / name).read_bytes())
            (self.directory / name).write_text("old protected resource\n")
        (self.directory / "run").write_text("#!/bin/sh\nexit 1\n")
        result = subprocess.run(["sh", str(packaged), "setup"], env=self.env,
                                capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        for name in ("run", "init", "myfeed.pem"):
            self.assertEqual((self.directory / name).read_bytes(), (SOURCE / name).read_bytes())
        self.assertEqual((self.directory / "runner").read_bytes(), packaged.read_bytes())
        self.assertEqual((self.directory / "firmware.checked").read_text().strip(), self.fingerprint())
        self.assertEqual(self.calls(), [])

    def test_current_tools_after_upgrade_do_not_reinstall(self):
        self.install_files()
        self.firmware_changed()
        self.assertEqual(self.run_bootstrap(MOCK_CURRENT="1").returncode, 0)
        self.assertEqual(self.transactions(), [])
        self.assertEqual((self.directory / "firmware.checked").read_text().strip(), self.fingerprint())

    def test_old_local_cache_installs_before_online_update(self):
        self.prepare_local_fixture()
        self.run_bootstrap("setup")
        self.firmware_changed()
        self.assertEqual(self.run_bootstrap(MOCK_OLD_BOOTSTRAP="1").returncode, 0)
        transactions = self.transactions()
        self.assertEqual(len(transactions), 2)
        self.assertIn("--no-network", transactions[0])
        self.assertNotIn("--no-network", transactions[1])
        self.assertIn("--upgrade", transactions[1])
        self.assertEqual((self.root / "etc/init.d/overlay-restore-bootstrap").read_bytes(), (SOURCE / "init").read_bytes())
        self.assertEqual((self.directory / "runner").read_bytes(), (SOURCE / "run").read_bytes())

    def test_offline_update_keeps_cached_tools_and_checkpoint_pending(self):
        self.prepare_local_fixture()
        self.run_bootstrap("setup")
        self.firmware_changed()
        self.assertEqual(self.run_bootstrap(MOCK_OLD_BOOTSTRAP="1", MOCK_ONLINE_UPDATE_ERROR="1").returncode, 1)
        self.assertEqual(len(self.transactions()), 1)
        self.assertTrue((self.root / "installed-overlay-restore").exists())
        self.assertEqual((self.directory / "firmware.checked").read_text().strip(), "0" * 64)
        result = subprocess.run(["sh", str(self.directory / "runner"), "run"], env=self.env,
                                capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.transactions()), 2)

    def test_watch_retries_when_network_becomes_ready_and_releases_lock(self):
        self.install_files()
        self.firmware_changed()
        self.assertEqual(self.run_bootstrap("watch", MOCK_NOT_READY="3").returncode, 0)
        self.assertEqual(len(self.transactions()), 1)
        self.assertEqual((self.root / "updates").read_text(), "4")

    def test_watch_stops_after_user_disables_automatic_updates(self):
        self.install_files()
        self.firmware_changed()
        self.assertEqual(self.run_bootstrap("watch", MOCK_ONLINE_UPDATE_ERROR="1", MOCK_WATCH_DISABLE="1").returncode, 0)
        self.assertEqual(self.transactions(), [])
        self.assertEqual(json.loads((self.root / "config.json").read_text())["upgrade_bootstrap"], "0")

    def test_online_refresh_failure_does_not_accept_a_stale_index(self):
        self.install_files()
        self.firmware_changed()
        self.assertEqual(self.run_bootstrap(MOCK_ONLINE_UPDATE_ERROR="1").returncode, 1)
        self.assertEqual(self.transactions(), [])
        self.assertEqual((self.directory / "firmware.checked").read_text().strip(), "0" * 64)

    def test_tool_update_refuses_unrelated_changes_and_downgrades(self):
        self.install_files()
        self.firmware_changed()
        for plan in ("Upgrading libc (1 -> 2)", "Purging luci-app-ota (1)",
                     "Downgrading overlay-restore (0.2.0-r14 -> 0.2.0-r13)"):
            with self.subTest(plan=plan):
                self.assertEqual(self.run_bootstrap(MOCK_PLAN="( 1/10) " + plan).returncode, 1)
                self.assertEqual(self.transactions(), [])
                self.assertEqual((self.directory / "firmware.checked").read_text().strip(), "0" * 64)

    def test_active_or_failed_recovery_postpones_tool_updates(self):
        self.install_files()
        self.firmware_changed()
        task = self.root / "etc/overlay-restore/jobs" / ("a" * 32) / "state.json"
        task.parent.mkdir(parents=True)
        for status in ("installing", "awaiting_reboot", "failed_packages", "queued_clean"):
            with self.subTest(status=status):
                task.write_text(json.dumps({"status": status}))
                self.assertEqual(self.run_bootstrap().returncode, 1)
                self.assertEqual(self.transactions(), [])
                self.assertFalse((self.root / "updates").exists())
        task.write_text(json.dumps({"status": "complete"}))
        self.assertEqual(self.run_bootstrap().returncode, 0)
        self.assertEqual(len(self.transactions()), 1)

    def test_rom_firmware_identity_is_used_over_retained_release_file(self):
        self.install_files()
        rom = self.root / "rom/etc/openwrt_release"
        rom.parent.mkdir(parents=True)
        rom.write_text("DISTRIB_RELEASE='25.12.6'\nDISTRIB_REVISION='new-image'\n")
        self.assertEqual(self.run_bootstrap().returncode, 0)
        self.assertEqual(len(self.transactions()), 1)
        self.assertEqual((self.directory / "firmware.checked").read_text().strip(), self.fingerprint())

    def run_hook(self, **values):
        env = dict(self.env, COMMAND="/lib/upgrade/do_stage2", TEST="0", CONF_BACKUP_LIST="0", CONF_BACKUP="", CONF_RESTORE="")
        env.update(values)
        return subprocess.run(["sh", "-c", '. "$1"; overlay_restore_mark_tool_refresh', "hook", str(HOOK)],
                              env=env, capture_output=True, text=True, timeout=15)

    def test_keep_config_upgrade_marks_same_image_refresh_only_after_reboot(self):
        self.install_files()
        self.assertEqual(self.run_hook().returncode, 0)
        self.assertTrue((self.directory / "refresh-pending").exists())
        self.assertEqual(self.run_bootstrap().returncode, 0)
        self.assertEqual(self.transactions(), [])
        (self.root / "proc/sys/kernel/random/boot_id").write_text("22222222-2222-4222-8222-222222222222\n")
        self.assertEqual(self.run_bootstrap().returncode, 0)
        self.assertEqual(len(self.transactions()), 1)
        self.assertFalse((self.directory / "refresh-pending").exists())

    def test_backup_list_test_and_clean_stage_do_not_arm_updates(self):
        for values in ({"CONF_BACKUP": "/tmp/config.tar.gz"}, {"CONF_BACKUP_LIST": "1"}, {"TEST": "1"},
                       {"COMMAND": "/usr/sbin/overlay-restore clean-stage /tmp/overlay-restore-stage.json"}):
            with self.subTest(values=values):
                self.assertEqual(self.run_hook(**values).returncode, 0)
                self.assertFalse((self.directory / "refresh-pending").exists())
        self.set_config({"": "restore", "upgrade_bootstrap": "0"})
        self.assertEqual(self.run_hook().returncode, 0)
        self.assertFalse((self.directory / "refresh-pending").exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
