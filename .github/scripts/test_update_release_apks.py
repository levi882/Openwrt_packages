#!/usr/bin/env python3
import importlib.util
import hashlib
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
    def candidate(self, version="0.1.1-r2"):
        return release(f"esa-cert-sync-v{version}", f"esa-cert-sync-{version}.apk",
                       f"luci-app-esa-cert-sync-{version}.apk", "SHA256SUMS")

    def test_ignores_unrelated_draft_and_prerelease_releases(self):
        draft = dict(self.candidate("0.2.0-r1"), draft=True)
        prerelease = dict(self.candidate("0.1.2-r1"), prerelease=True)
        selected = UPDATER.select_esa_cert_sync_assets([
            release("unrelated-v9.0.0"), draft, prerelease, self.candidate()])
        self.assertEqual(selected[0]["tag_name"], "esa-cert-sync-v0.1.1-r2")
        self.assertEqual(selected[3], "0.1.1-r2")

    def test_rejects_mismatched_frontend_revision_in_newest_release(self):
        broken = release("esa-cert-sync-v0.1.2-r1", "esa-cert-sync-0.1.2-r1.apk",
                         "luci-app-esa-cert-sync-0.1.1-r2.apk")
        with self.assertRaises(SystemExit):
            UPDATER.select_esa_cert_sync_assets([broken, self.candidate()])

    def test_missing_release_fails_instead_of_removing_package_group(self):
        with patch.object(UPDATER, "releases_list", return_value=[]):
            with self.assertRaises(SystemExit):
                UPDATER.get_esa_cert_sync()

    def test_hashes_both_selected_packages(self):
        payloads = {"esa-cert-sync-0.1.1-r2.apk": b"backend",
                    "luci-app-esa-cert-sync-0.1.1-r2.apk": b"frontend"}
        with patch.object(UPDATER, "releases_list", return_value=[self.candidate()]) as listing, \
             patch.object(UPDATER, "download_asset", side_effect=lambda a: payloads[a["name"]]):
            result = UPDATER.get_esa_cert_sync()
        listing.assert_called_once_with("levi882/Openwrt_packages")
        self.assertEqual(result["tag"], "esa-cert-sync-v0.1.1-r2")
        self.assertEqual(result["version"], "0.1.1-r2")
        self.assertEqual(result["main_sha"], hashlib.sha256(b"backend").hexdigest())
        self.assertEqual(result["luci_sha"], hashlib.sha256(b"frontend").hexdigest())


if __name__ == "__main__":
    unittest.main()
