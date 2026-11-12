#!/usr/bin/env python3
import importlib.util
import hashlib
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


MODULE_PATH = Path(__file__).with_name("update-release-apks.py")
SPEC = importlib.util.spec_from_file_location("update_release_apks", MODULE_PATH)
UPDATER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(UPDATER)


def release(tag, *asset_names):
    return {
        "tag_name": tag,
        "html_url": f"https://example.invalid/releases/tag/{tag}",
        "assets": [
            {"name": name, "browser_download_url": f"https://example.invalid/{name}"}
            for name in asset_names
        ],
    }


class SelectHomeboxAssetsTests(unittest.TestCase):
    def test_selects_matching_x86_64_openwrt_apks(self):
        candidate = release(
            "v0.1.4",
            "homebox-0.1.4-r1-openwrt-25.12.5-aarch64_generic.apk",
            "homebox-0.1.4-r1-openwrt-25.12.5-x86_64.apk",
            "luci-app-homebox-0.1.4-openwrt-25.12.5-x86_64.apk",
            "luci-i18n-homebox-zh-cn-0.1.4-openwrt-25.12.5-x86_64.apk",
        )

        main, luci, i18n, version, package_release, openwrt_release = (
            UPDATER.select_homebox_assets(candidate)
        )

        self.assertEqual(
            main["name"], "homebox-0.1.4-r1-openwrt-25.12.5-x86_64.apk"
        )
        self.assertEqual(
            luci["name"], "luci-app-homebox-0.1.4-openwrt-25.12.5-x86_64.apk"
        )
        self.assertEqual(
            i18n["name"],
            "luci-i18n-homebox-zh-cn-0.1.4-openwrt-25.12.5-x86_64.apk",
        )
        self.assertEqual(version, "0.1.4")
        self.assertEqual(package_release, "r1")
        self.assertEqual(openwrt_release, "25.12.5")


class SelectSmartdnsAssetsTests(unittest.TestCase):
    def test_pairs_webui_daemon_and_luci_across_releases(self):
        releases = [
            release(
                "1.2026.v48.4.1_with_ui",
                "smartdns.1.2026.v48.4.1.x86_64.apk",
            ),
            release(
                "1.2026.v48.4.1",
                "smartdns.1.2026.v48.4.1.x86_64-openwrt.apk",
                "luci-app-smartdns.1.2026.v48.4.1.luci-all.apk",
            ),
        ]

        selected = UPDATER.select_smartdns_assets(releases)

        self.assertIsNotNone(selected)
        main_release, main, luci_release, luci, version = selected
        self.assertEqual(main_release["tag_name"], "1.2026.v48.4.1_with_ui")
        self.assertEqual(main["name"], "smartdns.1.2026.v48.4.1.x86_64.apk")
        self.assertEqual(luci_release["tag_name"], "1.2026.v48.4.1")
        self.assertEqual(
            luci["name"], "luci-app-smartdns.1.2026.v48.4.1.luci-all.apk"
        )
        self.assertEqual(version, "1.2026.v48.4.1")

    def test_selects_renamed_webui_daemon_with_matching_luci(self):
        releases = [
            release(
                "1.2026.v48.4.11_with_ui",
                "smartdns_with_ui.1.2026.v48.4.11.x86_64.apk",
            ),
            release(
                "1.2026.v48.4.10_with_ui",
                "smartdns_with_ui.1.2026.v48.4.10.x86_64.apk",
            ),
            release(
                "1.2026.v48.4.10",
                "luci-app-smartdns.1.2026.v48.4.10.luci-all.apk",
            ),
        ]

        self.assertEqual(
            UPDATER.select_smartdns_assets(releases),
            (
                releases[1],
                releases[1]["assets"][0],
                releases[2],
                releases[2]["assets"][0],
                "1.2026.v48.4.10",
            ),
        )

    def test_rejects_static_openwrt_daemon(self):
        releases = [
            release(
                "1.2026.v48.4.1",
                "smartdns.1.2026.v48.4.1.x86_64-openwrt.apk",
                "luci-app-smartdns.1.2026.v48.4.1.luci-all.apk",
            )
        ]

        self.assertIsNone(UPDATER.select_smartdns_assets(releases))


class EsaCertSyncTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "prebuilt" / "esa-cert-sync"
        self.source.mkdir(parents=True)
        source_commit = "a" * 40
        artifacts = []
        for name, payload in [("esa-cert-sync-0.1.1-r2.apk", b"backend"),
                              ("luci-app-esa-cert-sync-0.1.1-r2.apk", b"frontend")]:
            (self.source / name).write_bytes(payload)
            artifacts.append({"type": "file", "output": name,
                              "url": f"https://raw.githubusercontent.com/levi882/Openwrt_packages/{source_commit}/prebuilt/esa-cert-sync/{name}",
                              "sha256": hashlib.sha256(payload).hexdigest()})
        self.package = {"id": "esa-cert-sync", "metadata": {
            "distribution": "feed-only", "version": "0.1.1-r2", "source_commit": source_commit},
            "artifacts": artifacts}
        for target, value in [("ROOT", self.root)]:
            patcher = patch.object(UPDATER, target, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        patcher = patch.object(UPDATER, "read_manifest", return_value={"packages": [self.package]})
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_preserves_feed_only_packages_without_querying_releases(self):
        with patch.object(UPDATER, "releases_list") as listing:
            self.assertEqual(UPDATER.get_esa_cert_sync(), self.package)
        listing.assert_not_called()

    def test_missing_package_fails_instead_of_silently_removing_it(self):
        with patch.object(UPDATER, "read_manifest", return_value={"packages": []}):
            with self.assertRaisesRegex(SystemExit, "Missing feed-only ESA"):
                UPDATER.get_esa_cert_sync()

    def test_modified_apk_fails_hash_validation(self):
        (self.source / "esa-cert-sync-0.1.1-r2.apk").write_bytes(b"modified")
        with self.assertRaisesRegex(SystemExit, "Missing or modified"):
            UPDATER.get_esa_cert_sync()

    def test_missing_apk_fails_validation(self):
        (self.source / "esa-cert-sync-0.1.1-r2.apk").unlink()
        with self.assertRaisesRegex(SystemExit, "Missing or modified"):
            UPDATER.get_esa_cert_sync()

    def test_cannot_drop_frontend_package(self):
        self.package["artifacts"].pop()
        with self.assertRaisesRegex(SystemExit, "both backend and LuCI"):
            UPDATER.get_esa_cert_sync()

    def test_rejects_unpinned_url(self):
        self.package["artifacts"][0]["url"] = "https://example.invalid/esa.apk"
        with self.assertRaisesRegex(SystemExit, "pinned"):
            UPDATER.get_esa_cert_sync()


if __name__ == "__main__":
    unittest.main()
